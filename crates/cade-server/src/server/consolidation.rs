//! Background memory consolidation — the "Sleeptime Agent".
//!
//! When the budget-based context builder in `build_context()` drops older turns
//! from the LLM prompt it sets `needs_consolidation = true` in `agent_activity`.
//! After 60 s of agent inactivity the Sleeptime background task calls
//! [`consolidate_agent`], which summarises the dropped turns into a persistent
//! `session_summary` memory block so the agent retains the gist of past work
//! across context rotations.

use cade_ai::{CompletionRequest, LlmMessage, catalogue};

use crate::server::state::AppState;
use cade_store::sqlite;

/// Resolve the output directory for memory exports readable by cade-rag-mcp.
///
/// Precedence:
///   1. `CADE_RAG_EXPORT_DIR` env var (absolute path), agent-id appended.
///   2. `$HOME/.cade/rag/<agent_id>/memory/`
///   3. `None` — export will be skipped silently.
fn resolve_rag_export_dir(agent_id: &str) -> Option<std::path::PathBuf> {
    if let Ok(custom) = std::env::var("CADE_RAG_EXPORT_DIR")
        && !custom.trim().is_empty()
    {
        return Some(std::path::PathBuf::from(custom).join(agent_id).join("memory"));
    }
    dirs::home_dir().map(|h| h.join(".cade").join("rag").join(agent_id).join("memory"))
}

// ── tunables ──────────────────────────────────────────────────────────────────

/// Minimum number of DB rows required before consolidation is attempted.
/// Below this the conversation is too short to be worth summarising.
const MIN_ROWS_FOR_CONSOLIDATION: usize = 20;

/// Maximum chars of formatted history text fed to the summarisation LLM call.
/// ~8 k tokens at 3 chars/token — enough context without blowing cost.
const MAX_SUMMARY_INPUT_CHARS: usize = 24_000;

/// Maximum tokens the summarisation LLM is allowed to emit.
/// Budget: ~700 tokens for the narrative summary + ~100 tokens for search anchors.
const SUMMARY_MAX_TOKENS: u32 = 900;

/// Maximum chars stored in the `session_summary` memory block.
const SESSION_SUMMARY_MAX_CHARS: usize = 4_500;

/// Fraction of the estimated history budget used as the threshold: turns that
/// fit within `char_budget * HISTORY_BUDGET_FRACTION` are considered "in
/// context"; everything older is considered "dropped" and summarised.
const HISTORY_BUDGET_FRACTION: f64 = 0.40;

/// Characters per token approximation (conservative).
const CHARS_PER_TOKEN: usize = 3;

// ── public API ────────────────────────────────────────────────────────────────

/// Summarise older conversation turns that are no longer in the active context
/// window and write the result to the agent's `session_summary` memory block.
///
/// This is safe to call concurrently for different agents; all DB access is
/// through the existing `Arc<Mutex<Connection>>` pool.
pub async fn consolidate_agent(state: &AppState, agent_id: &str, conversation_id: Option<&str>) {
    let agent = match sqlite::get_agent(&state.db, agent_id) {
        Ok(Some(a)) => a,
        Ok(None) => {
            tracing::warn!("consolidate [{}]: agent not found — skipping", agent_id);
            return;
        }
        Err(e) => {
            tracing::warn!("consolidate [{}]: DB error: {}", agent_id, e);
            return;
        }
    };

    // ── 1. Fetch recent messages ──────────────────────────────────────────────
    let all_rows = sqlite::list_messages_page(&state.db, agent_id, conversation_id, 500, 0)
        .unwrap_or_default();

    if all_rows.len() < MIN_ROWS_FOR_CONSOLIDATION {
        tracing::debug!(
            "consolidate [{}]: only {} rows — skipping",
            agent_id,
            all_rows.len()
        );
        return;
    }

    // Convert rows to (role, text) pairs for turn grouping.
    let flat: Vec<(String, String)> = all_rows
        .iter()
        .map(|row| {
            let role = row.role.clone();
            let text = row.content["content"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| {
                    let raw = row.content.to_string();
                    if raw.len() > 400 {
                        format!("{}…", &raw[..400])
                    } else {
                        raw
                    }
                });
            (role, text)
        })
        .collect();

    // ── 2. Determine which turns are "in context" vs "dropped" ───────────────
    let window_tokens = catalogue::context_window_for_model(&agent.model) as usize;
    let output_reserve = ((window_tokens as f64) * 0.15).round() as usize;
    let input_tokens = window_tokens.saturating_sub(output_reserve);
    let char_budget = (input_tokens * CHARS_PER_TOKEN).clamp(8_000, 6_000_000);
    let history_budget = (char_budget as f64 * HISTORY_BUDGET_FRACTION).round() as usize;

    let turns = group_turns(&flat);
    let total_turns = turns.len();

    let mut in_context = 0usize;
    let mut used = 0usize;
    for turn in turns.iter().rev() {
        let chars: usize = turn.iter().map(|(_, t)| t.chars().count()).sum();
        if used + chars <= history_budget {
            in_context += 1;
            used += chars;
        } else {
            break;
        }
    }

    let dropped = total_turns.saturating_sub(in_context);
    if dropped == 0 {
        tracing::debug!(
            "consolidate [{}]: all {} turns fit in budget — nothing to summarise",
            agent_id,
            total_turns
        );
        return;
    }

    // ── 3. Format dropped turns into a text block for the LLM ────────────────
    //
    // Pre-processing extracts high-signal artifacts (file paths, error messages,
    // function names) from each message *before* truncation so they survive the
    // 600-char preview cut.  These are prepended as a structured prefix.
    let mut history_text = String::new();
    'outer: for turn in &turns[..dropped] {
        for (role, text) in turn {
            if history_text.chars().count() >= MAX_SUMMARY_INPUT_CHARS {
                history_text.push_str("\n[...older history truncated...]");
                break 'outer;
            }
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Skip noisy short tool results — but keep those that look like
            // error codes or short meaningful outputs (contain digits or '/')
            if role == "tool" && trimmed.len() < 15
                && !trimmed.contains('/')
                && !trimmed.chars().any(|c| c.is_ascii_digit())
            {
                continue;
            }
            // Extract high-signal artifacts before truncation: file paths,
            // function signatures, and error-like strings survive even when
            // the full message is cut to 600 chars.
            let artifacts = extract_artifacts(trimmed);
            let artifact_prefix = if artifacts.is_empty() {
                String::new()
            } else {
                format!(" | artifacts: {}", artifacts.join(", "))
            };
            // Truncate very long individual messages (file dumps, base64, etc.)
            let preview: String = if trimmed.chars().count() > 600 {
                format!("{}…", trimmed.chars().take(600).collect::<String>())
            } else {
                trimmed.to_string()
            };
            history_text.push_str(&format!("[{role}{artifact_prefix}] {preview}\n"));
        }
    }

    if history_text.trim().is_empty() {
        tracing::debug!(
            "consolidate [{}]: dropped turns have no useful text — skipping",
            agent_id
        );
        return;
    }

    // ── 4. Call the LLM to produce a consolidation summary ───────────────────
    let prompt = format!(
        "You are a memory consolidation sub-agent for a stateful coding assistant.\n\
         The following is older conversation history that has scrolled out of the \
         agent's active context window.\n\
         \n\
         Extract only what the agent needs to remember for future turns:\n\
         1. The main task or goal being worked on\n\
         2. Files read, created, or modified — use exact paths (e.g. `src/server/consolidation.rs`), \
            exact function names, exact variable names. Never paraphrase these.\n\
         3. Key decisions or approaches chosen, the reasoning behind them, \
            AND alternatives that were considered and rejected (with why)\n\
         4. Problems encountered — include exact error messages (first 80 chars) and error codes\n\
         5. Work completed vs work still in progress\n\
         6. Any conventions, constraints, or preferences discovered\n\
         \n\
         Write as a concise structured note (max 350 words). Be factual and specific. \
         Do not describe the conversation format or refer to 'the user said'. \
         Write in past tense from the perspective of what happened.\n\
         \n\
         After the summary, add a final section:\n\
         SEARCH ANCHORS: [up to 8 comma-separated keywords — specific filenames, \
         function names, error codes, or topic identifiers from the dropped history \
         that are NOT already mentioned in the summary above. These help the agent \
         recover granular detail via conversation_search.]\n\
         \n\
         HISTORY:\n\
         {history_text}"
    );

    // Use the compaction model if configured, otherwise fall back to the main model.
    let compaction_model = agent
        .compaction_model
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or(&agent.model);

    let req = CompletionRequest {
        model: compaction_model.to_string(),
        messages: vec![LlmMessage {
            role: "user".to_string(),
            content: prompt,
            tool_call_id: None,
            tool_calls: None,
            images: None,
        }],
        tools: vec![],
        max_tokens: SUMMARY_MAX_TOKENS,
        reasoning_effort: None,
    };

    let summary = match state.llm.complete(&req).await {
        Ok(resp) => resp.content.unwrap_or_default().trim().to_string(),
        Err(e) => {
            tracing::warn!("consolidate [{}]: LLM call failed: {}", agent_id, e);
            return;
        }
    };

    if summary.is_empty() {
        tracing::debug!("consolidate [{}]: LLM returned empty summary", agent_id);
        return;
    }

    // ── 4b. Inflation guard — reject summary if it's larger than the dropped content ──
    let dropped_chars = history_text.chars().count();
    let summary_chars = summary.chars().count();
    if is_summary_inflated(summary_chars, dropped_chars) {
        tracing::warn!(
            "consolidate [{}]: summary inflated ({} chars) vs dropped ({} chars) — skipping",
            agent_id,
            summary_chars,
            dropped_chars,
        );
        let mut metrics = state.agent_metrics.write().await;
        metrics.entry(agent_id.to_string()).or_default().inflation_guard_hits += 1;
        return;
    }

    // ── 5. Write to the `session_summary` memory block ───────────────────────
    // Append to the existing summary or start fresh; cap at SESSION_SUMMARY_MAX_CHARS.
    let existing_blocks = sqlite::get_memory_blocks(&state.db, agent_id).unwrap_or_default();
    let existing = existing_blocks
        .iter()
        .find(|(label, _, _)| label == "session_summary")
        .map(|(_, val, _)| val.as_str())
        .unwrap_or("");

    let new_value = if existing.is_empty() {
        summary.clone()
    } else {
        let combined = format!("{existing}\n\n---\n\n{summary}");
        if combined.chars().count() > SESSION_SUMMARY_MAX_CHARS {
            // Combined too long — keep only the latest summary to stay useful.
            summary.clone()
        } else {
            combined
        }
    };

    if let Err(e) = sqlite::upsert_memory_block(
        &state.db,
        agent_id,
        "session_summary",
        &new_value,
        Some("Auto-generated summary of older conversation turns (Sleeptime consolidation)"),
        Some(SESSION_SUMMARY_MAX_CHARS),
    ) {
        tracing::warn!(
            "consolidate [{}]: failed to write session_summary: {}",
            agent_id,
            e
        );
        return;
    }

    // P5-C: Ensure session_summary remains pinned across restarts.
    let _ = sqlite::set_memory_tier(&state.db, agent_id, "session_summary", "pinned", true);

    // P6: Ensure working_set (the agent's primary decision record) never ages
    // out.  Pin it if it exists — the agent writes this block at decision time
    // with full reasoning, so it must survive consolidation cycles.
    let has_working_set = existing_blocks
        .iter()
        .any(|(label, val, _)| label == "working_set" && !val.trim().is_empty());
    if has_working_set {
        let _ = sqlite::set_memory_tier(&state.db, agent_id, "working_set", "pinned", true);
    }

    tracing::info!(
        "consolidate [{}]: session_summary updated ({} chars; {} dropped turns summarised)",
        agent_id,
        new_value.chars().count(),
        dropped,
    );

    // Phase B.2: export memory to a directory cade-rag-mcp can index.
    //
    // The export path is `<cade_home>/rag/<agent_id>/memory/` unless the
    // operator has overridden it via `CADE_RAG_EXPORT_DIR`. If the path is
    // unavailable (no $HOME, disk full, permission denied) we log and move
    // on — this is an *optional* secondary surface, never a hard dependency.
    if let Some(out_dir) = resolve_rag_export_dir(agent_id) {
        match sqlite::export_memory_to_rag_dir(&state.db, agent_id, &out_dir) {
            Ok(report) => tracing::debug!(
                "consolidate [{}]: exported memory to rag dir ({} blocks, {} archival) at {}",
                agent_id,
                report.blocks_written,
                report.archival_written,
                report.out_dir,
            ),
            Err(e) => tracing::debug!(
                "consolidate [{}]: rag export skipped: {}",
                agent_id,
                e
            ),
        }
    }

    // ── 6. Insert a compaction marker into message history ───────────────────
    // The marker acts as a boundary: `get_context_window()` only loads messages
    // after the most recent marker, drastically reducing the scan set.
    //
    // Use the newest dropped message's ID to anchor the marker's timestamp.
    // The dropped turns are all_rows[0..dropped_msg_count] (oldest-first).
    // We want the marker's created_at to be equal to the newest dropped message.
    let dropped_msg_count: usize = turns[..dropped]
        .iter()
        .map(|t| t.len())
        .sum();

    // Look up the created_at of the boundary message from the DB.
    // all_rows is oldest-first; the boundary is at index dropped_msg_count - 1.
    let boundary_msg_id = if dropped_msg_count > 0 && dropped_msg_count <= all_rows.len() {
        Some(all_rows[dropped_msg_count - 1].id.clone())
    } else {
        None
    };

    if let Some(ref bid) = boundary_msg_id {
        let marker_ts = {
            let conn = state.db.lock().map_err(|e| {
                tracing::warn!("consolidate [{}]: DB lock: {}", agent_id, e);
            });
            match conn {
                Ok(c) => c
                    .query_row(
                        "SELECT created_at FROM messages WHERE id = ?1",
                        rusqlite::params![bid],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or_else(|_| {
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64
                    }),
                Err(_) => return,
            }
        };

        let marker_content = serde_json::json!({
            "content": format!(
                "[Compaction marker: {} turns summarised into session_summary]",
                dropped,
            ),
        });

        let marker = sqlite::MessageRow {
            id: format!("compact-{}", uuid::Uuid::new_v4()),
            agent_id: agent_id.to_string(),
            conversation_id: conversation_id.map(String::from),
            role: "compaction".to_string(),
            content: marker_content,
            char_count: 0,
        };

        // Insert with the boundary timestamp so ordering is correct.
        if let Ok(conn) = state.db.lock() {
            let _ = conn.execute(
                "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at, char_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    marker.id,
                    marker.agent_id,
                    marker.conversation_id,
                    marker.role,
                    marker.content.to_string(),
                    marker_ts,
                    0i64,
                ],
            );
            tracing::debug!(
                "consolidate [{}]: inserted compaction marker '{}' at ts={}",
                agent_id,
                marker.id,
                marker_ts,
            );
        }
    }

    let mut metrics = state.agent_metrics.write().await;
    let m = metrics.entry(agent_id.to_string()).or_default();
    m.consolidation_runs += 1;
    m.chars_summarised += dropped_chars;
    m.chars_produced += summary_chars;
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Returns `true` if the summary is inflated relative to the source text — i.e.,
/// the summary is ≥ 80% of the dropped-content size and should be rejected.
fn is_summary_inflated(summary_chars: usize, dropped_chars: usize) -> bool {
    dropped_chars > 0 && summary_chars > ((dropped_chars as f64) * 0.8) as usize
}

/// Extract high-signal artifacts from a message that should survive truncation.
///
/// Scans the text for:
///   - File paths (containing `/` and a file extension like `.rs`, `.ts`, `.py`, etc.)
///   - Error-like patterns (lines starting with "error", "Error", "E0", "RUSTSEC-", etc.)
///   - Function/method names (word followed by `(`)
///
/// Returns up to 6 unique artifact strings, each capped at 80 chars.
fn extract_artifacts(text: &str) -> Vec<String> {
    let mut artifacts: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for word in text.split_whitespace() {
        if artifacts.len() >= 6 {
            break;
        }

        let cleaned = word.trim_matches(|c: char| c == ',' || c == ';' || c == '`' || c == '\'' || c == '"');

        // File paths: contains '/' and ends with a known extension
        if cleaned.contains('/')
            && (cleaned.ends_with(".rs")
                || cleaned.ends_with(".ts")
                || cleaned.ends_with(".js")
                || cleaned.ends_with(".py")
                || cleaned.ends_with(".toml")
                || cleaned.ends_with(".json")
                || cleaned.ends_with(".yaml")
                || cleaned.ends_with(".yml")
                || cleaned.ends_with(".md")
                || cleaned.ends_with(".html")
                || cleaned.ends_with(".css")
                || cleaned.ends_with(".go")
                || cleaned.ends_with(".java")
                || cleaned.ends_with(".c")
                || cleaned.ends_with(".h")
                || cleaned.ends_with(".cpp"))
        {
            let artifact: String = cleaned.chars().take(80).collect();
            if seen.insert(artifact.clone()) {
                artifacts.push(artifact);
            }
            continue;
        }

        // Error identifiers: RUSTSEC-*, E0xxx, error[Exxxx]
        if cleaned.starts_with("RUSTSEC-")
            || cleaned.starts_with("error[")
            || (cleaned.starts_with("E0") && cleaned.len() <= 6 && cleaned[2..].chars().all(|c| c.is_ascii_digit()))
        {
            let artifact: String = cleaned.chars().take(80).collect();
            if seen.insert(artifact.clone()) {
                artifacts.push(artifact);
            }
            continue;
        }

        // Function/method names: word ending with '(' or '()'
        if (cleaned.ends_with('(') || cleaned.ends_with("()"))
            && cleaned.len() > 2
            && cleaned.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        {
            let artifact: String = cleaned.chars().take(80).collect();
            if seen.insert(artifact.clone()) {
                artifacts.push(artifact);
            }
        }
    }

    // Also scan for error-prefixed lines (e.g. "error: ...", "Error: ...")
    for line in text.lines().take(100) {
        if artifacts.len() >= 6 {
            break;
        }
        let trimmed = line.trim();
        if (trimmed.starts_with("error:") || trimmed.starts_with("Error:") || trimmed.starts_with("ERROR:"))
            && trimmed.len() > 7
        {
            let artifact: String = trimmed.chars().take(80).collect();
            if seen.insert(artifact.clone()) {
                artifacts.push(artifact);
            }
        }
    }

    artifacts
}

/// Group `(role, text)` pairs into logical turns.
/// A turn starts at each `user` message and includes all following non-`user`
/// messages (assistant, tool) until the next `user` message.
fn group_turns(messages: &[(String, String)]) -> Vec<Vec<(String, String)>> {
    let mut turns: Vec<Vec<(String, String)>> = Vec::new();
    let mut current: Vec<(String, String)> = Vec::new();
    for msg in messages {
        if msg.0 == "user" && !current.is_empty() {
            turns.push(std::mem::take(&mut current));
        }
        current.push(msg.clone());
    }
    if !current.is_empty() {
        turns.push(current);
    }
    turns
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn m(role: &str, text: &str) -> (String, String) {
        (role.to_string(), text.to_string())
    }

    #[test]
    fn empty_produces_no_turns() {
        assert!(group_turns(&[]).is_empty());
    }

    #[test]
    fn single_user_message_is_one_turn() {
        let turns = group_turns(&[m("user", "hello")]);
        assert_eq!(turns.len(), 1);
    }

    #[test]
    fn two_exchanges_produce_two_turns() {
        let msgs = vec![
            m("user", "q1"),
            m("assistant", "a1"),
            m("user", "q2"),
            m("assistant", "a2"),
        ];
        let turns = group_turns(&msgs);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0][0].0, "user");
        assert_eq!(turns[1][0].0, "user");
    }

    #[test]
    fn tool_result_stays_in_same_turn_as_its_call() {
        let msgs = vec![
            m("user", "do thing"),
            m("assistant", ""), // triggers tool call
            m("tool", "ok"),    // result
            m("assistant", "done"),
            m("user", "next"),
        ];
        let turns = group_turns(&msgs);
        assert_eq!(turns.len(), 2);
        // First turn: user + assistant + tool + assistant = 4 messages
        assert_eq!(turns[0].len(), 4);
    }

    // ── Inflation guard tests ─────────────────────────────────────────────

    #[test]
    fn inflation_guard_rejects_when_summary_is_large() {
        assert!(is_summary_inflated(900, 1000));
    }

    #[test]
    fn inflation_guard_accepts_when_summary_is_compact() {
        assert!(!is_summary_inflated(200, 1000));
    }

    #[test]
    fn inflation_guard_boundary_at_80_percent() {
        assert!(!is_summary_inflated(800, 1000));
        assert!(is_summary_inflated(801, 1000));
    }

    #[test]
    fn inflation_guard_handles_zero_dropped() {
        assert!(!is_summary_inflated(100, 0));
    }

    #[test]
    fn inflation_guard_handles_empty_summary() {
        assert!(!is_summary_inflated(0, 1000));
    }

    // ── extract_artifacts tests ───────────────────────────────────────────

    #[test]
    fn extracts_rust_file_paths() {
        let text = "Modified src/server/consolidation.rs and crates/cade-core/src/lib.rs";
        let arts = extract_artifacts(text);
        assert!(arts.contains(&"src/server/consolidation.rs".to_string()));
        assert!(arts.contains(&"crates/cade-core/src/lib.rs".to_string()));
    }

    #[test]
    fn extracts_function_names() {
        let text = "Called extract_artifacts() and build_context( with args";
        let arts = extract_artifacts(text);
        assert!(arts.iter().any(|a| a.contains("extract_artifacts")));
        assert!(arts.iter().any(|a| a.contains("build_context")));
    }

    #[test]
    fn extracts_error_identifiers() {
        let text = "Found RUSTSEC-2025-0009 and error[E0433] in the build";
        let arts = extract_artifacts(text);
        assert!(arts.iter().any(|a| a.contains("RUSTSEC-2025-0009")));
        assert!(arts.iter().any(|a| a.contains("error[E0433]")));
    }

    #[test]
    fn extracts_error_lines() {
        let text = "output:\nerror: cannot find type `Foo` in this scope\nmore stuff";
        let arts = extract_artifacts(text);
        assert!(arts.iter().any(|a| a.starts_with("error: cannot find")));
    }

    #[test]
    fn caps_at_six_artifacts() {
        let text = "src/a.rs src/b.rs src/c.rs src/d.rs src/e.rs src/f.rs src/g.rs src/h.rs";
        let arts = extract_artifacts(text);
        assert!(arts.len() <= 6);
    }

    #[test]
    fn empty_text_yields_no_artifacts() {
        assert!(extract_artifacts("").is_empty());
    }

    #[test]
    fn plain_text_yields_no_artifacts() {
        assert!(extract_artifacts("hello world this is a normal sentence").is_empty());
    }

    #[test]
    fn deduplicates_artifacts() {
        let text = "src/lib.rs and again src/lib.rs and src/lib.rs";
        let arts = extract_artifacts(text);
        assert_eq!(arts.len(), 1);
    }
}
