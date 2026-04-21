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

/// Phase C: maximum number of rotated `session_summary_N` blocks to keep in
/// the long-term tier. When the ring fills, the oldest is evicted and a
/// one-line excerpt is appended to the pinned `session_index` block.
const SESSION_SUMMARY_RING_CAP: usize = 5;

/// Max chars retained per rotated `session_summary_N` block. Lower than the
/// live cap because older phases get less frequent attention.
const SESSION_SUMMARY_ARCHIVED_MAX_CHARS: usize = 2_000;

/// Max chars retained in the `session_index` pinned block. When the FIFO
/// line-buffer exceeds this, the oldest lines are dropped.
const SESSION_INDEX_MAX_CHARS: usize = 3_000;

/// Fraction of the estimated history budget used as the threshold: turns that
/// fit within `char_budget * HISTORY_BUDGET_FRACTION` are considered "in
/// context"; everything older is considered "dropped" and summarised.
const HISTORY_BUDGET_FRACTION: f64 = 0.40;

/// Characters per token approximation (conservative).
const CHARS_PER_TOKEN: usize = 3;

// ── preview / filter helpers (M2) ────────────────────────────────────────────

/// Maximum chars kept per message in the history text fed to the summariser.
///
/// Limits are per-role because assistant turns carry the highest-signal
/// technical content (file edits, decisions, error reports) and were being
/// clipped at the old flat 600-char cap. Tool outputs are medium-signal;
/// user prompts are shortest on average. Unknown roles get the smallest
/// limit to prevent an unexpected role from flooding the summariser.
fn preview_limit_for_role(role: &str) -> usize {
    match role {
        "assistant" => 1_200,
        "tool" => 800,
        "user" => 400,
        _ => 400,
    }
}

/// Whether to drop a tool message from the summary prompt as pure noise.
///
/// M2: the old heuristic (`len < 15 && no '/' && no digit`) incorrectly
/// dropped legitimate short confirmations such as `"ok"` or `"done"`, making
/// the summariser think those tools never ran. The `MAX_SUMMARY_INPUT_CHARS`
/// cap upstream is now the only safeguard against runaway input, and
/// whitespace-only content is already filtered via `trimmed.is_empty()` before
/// this function is called.
fn should_skip_noisy_tool(_role: &str, _trimmed: &str) -> bool {
    false
}

/// Number of turns between eager (turn-count-driven) consolidation runs.
///
/// The Sleeptime background task fires consolidation after 20 s of inactivity
/// (see `src/bin/cade-server.rs`). During a continuous interactive session
/// that timer may never expire between turns, so we also fire consolidation
/// once every `EAGER_CONSOLIDATION_TURN_THRESHOLD` turns that produce a
/// `needs_consolidation` signal. 20 is comfortably below the 80-turn
/// `STALE_THRESHOLD` so `active_goal`'s pin (see M1) and the session_summary
/// block are refreshed before `promote_stale_blocks` could archive them.
pub(crate) const EAGER_CONSOLIDATION_TURN_THRESHOLD: i64 = 20;

/// Pure decision: given the agent's current turn counter and the turn at which
/// the last eager consolidation fired (0 if never), should we trigger an eager
/// run now? This is the ONLY logic driving the eager path — keeping it pure
/// makes it exhaustively testable without state plumbing.
///
/// Returns `true` iff `current_turn - last_consolidation_turn >= threshold`,
/// using saturating subtraction so a `current < last` counter regression never
/// panics.
pub(crate) fn should_eager_consolidate(
    current_turn: i64,
    last_consolidation_turn: i64,
    threshold: i64,
) -> bool {
    if threshold <= 0 {
        return false;
    }
    let gap = current_turn.saturating_sub(last_consolidation_turn);
    gap >= threshold
}

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

    // ── 1. Fetch messages since the last compaction marker ───────────────────
    // Using `list_messages_since_last_compaction` ensures we only summarise
    // turns that have NOT yet been covered by a previous consolidation run.
    // `list_messages_page` returned ALL rows (ignoring markers), causing the
    // consolidation LLM to re-summarise already-compacted history on every
    // invocation — producing duplicate session_summary entries.
    let all_rows = sqlite::list_messages_since_last_compaction(
        &state.db,
        agent_id,
        conversation_id,
        500,
    )
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
    // per-role preview cut (see `preview_limit_for_role`). These are prepended
    // as a structured prefix.
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
            // Filter pure-noise tool messages (no-op after M2 — retained for
            // call-site readability in case future heuristics are added).
            if should_skip_noisy_tool(role, trimmed) {
                continue;
            }
            // Extract high-signal artifacts before truncation: file paths,
            // function signatures, and error-like strings survive even when
            // the full message is cut to the per-role preview limit.
            let artifacts = extract_artifacts(trimmed);
            let artifact_prefix = if artifacts.is_empty() {
                String::new()
            } else {
                format!(" | artifacts: {}", artifacts.join(", "))
            };
            // Truncate very long individual messages (file dumps, base64, etc.)
            // using per-role limits so assistant technical content survives.
            let preview_cap = preview_limit_for_role(role);
            let preview: String = if trimmed.chars().count() > preview_cap {
                format!("{}…", trimmed.chars().take(preview_cap).collect::<String>())
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
            // Phase C: instead of losing the previous summary, rotate it into
            // the `session_summary_N` ring (long-term tier). The new live
            // value becomes just the latest summary.
            rotate_and_archive_session_summary(state, agent_id, existing);
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

    // P6: Ensure active_goal (the agent's primary decision record) never ages
    // out.  Pin it if it exists — the agent writes this block at decision time
    // with full reasoning, so it must survive consolidation cycles.
    let has_active_goal = existing_blocks
        .iter()
        .any(|(label, val, _)| label == "active_goal" && !val.trim().is_empty());
    if has_active_goal {
        let _ = sqlite::set_memory_tier(&state.db, agent_id, "active_goal", "pinned", true);
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

/// Phase C: rotate the live `session_summary` into the `session_summary_N`
/// ring before it is overwritten by a fresh consolidation pass.
///
/// Behavior:
///   1. If `session_summary_{RING_CAP}` already exists, extract its first
///      non-empty line and append it to the pinned `session_index` block,
///      then delete the evicted block.
///   2. Shift blocks up by one: `session_summary_{N}` → `session_summary_{N+1}`
///      for N = RING_CAP-1 down to 1.
///   3. Write `prev_live` (capped at SESSION_SUMMARY_ARCHIVED_MAX_CHARS,
///      truncated head-first to preserve the tail / most-recent content)
///      to `session_summary_1` at tier `long`.
///
/// All DB errors are logged at debug/warn and swallowed — rotation is
/// best-effort and must never break the main consolidation path.
fn rotate_and_archive_session_summary(state: &AppState, agent_id: &str, prev_live: &str) {
    rotate_and_archive_session_summary_db(&state.db, agent_id, prev_live);
}

fn rotate_and_archive_session_summary_db(
    db: &cade_store::sqlite::Db,
    agent_id: &str,
    prev_live: &str,
) {
    if prev_live.trim().is_empty() {
        return;
    }

    let blocks = sqlite::get_memory_blocks(db, agent_id).unwrap_or_default();
    let label_for = |n: usize| format!("session_summary_{n}");

    // Step 1: evict oldest slot if occupied.
    let oldest_label = label_for(SESSION_SUMMARY_RING_CAP);
    if let Some((_, val, _)) = blocks.iter().find(|(l, _, _)| l == &oldest_label) {
        let excerpt = first_nonempty_line(val);
        if !excerpt.is_empty() {
            append_to_session_index_db(db, agent_id, &excerpt);
        }
        if let Err(e) = sqlite::delete_memory_block(db, agent_id, &oldest_label) {
            tracing::debug!(
                "consolidate [{}]: failed to evict {}: {}",
                agent_id,
                oldest_label,
                e
            );
        }
    }

    // Step 2: shift N → N+1, from N=RING_CAP-1 down to 1.
    for n in (1..SESSION_SUMMARY_RING_CAP).rev() {
        let src = label_for(n);
        let dst = label_for(n + 1);
        if let Some((_, val, _)) = blocks.iter().find(|(l, _, _)| l == &src) {
            if let Err(e) = sqlite::upsert_memory_block(
                db,
                agent_id,
                &dst,
                val,
                Some("Rotated session summary (Phase C ring)"),
                Some(SESSION_SUMMARY_ARCHIVED_MAX_CHARS),
            ) {
                tracing::debug!(
                    "consolidate [{}]: failed to shift {} → {}: {}",
                    agent_id,
                    src,
                    dst,
                    e
                );
                continue;
            }
            let _ = sqlite::set_memory_tier(db, agent_id, &dst, "long", false);
            if let Err(e) = sqlite::delete_memory_block(db, agent_id, &src) {
                tracing::debug!(
                    "consolidate [{}]: failed to delete old {}: {}",
                    agent_id,
                    src,
                    e
                );
            }
        }
    }

    // Step 3: write prev_live into slot 1 (head-truncated to preserve tail).
    let capped = truncate_head_to(prev_live, SESSION_SUMMARY_ARCHIVED_MAX_CHARS);
    let slot1 = label_for(1);
    if let Err(e) = sqlite::upsert_memory_block(
        db,
        agent_id,
        &slot1,
        &capped,
        Some("Rotated session summary (Phase C ring)"),
        Some(SESSION_SUMMARY_ARCHIVED_MAX_CHARS),
    ) {
        tracing::debug!(
            "consolidate [{}]: failed to write {}: {}",
            agent_id,
            slot1,
            e
        );
        return;
    }
    let _ = sqlite::set_memory_tier(db, agent_id, &slot1, "long", false);

    tracing::debug!(
        "consolidate [{}]: rotated session_summary ({} chars) → {}",
        agent_id,
        capped.chars().count(),
        slot1,
    );
}

/// Append a one-line excerpt to the pinned `session_index` block, evicting
/// oldest lines FIFO when the block exceeds `SESSION_INDEX_MAX_CHARS`.
fn append_to_session_index_db(db: &cade_store::sqlite::Db, agent_id: &str, excerpt: &str) {
    let blocks = sqlite::get_memory_blocks(db, agent_id).unwrap_or_default();
    let existing = blocks
        .iter()
        .find(|(l, _, _)| l == "session_index")
        .map(|(_, v, _)| v.as_str())
        .unwrap_or("");

    let line = sanitize_index_line(excerpt);
    if line.is_empty() {
        return;
    }

    let mut combined = if existing.is_empty() {
        line
    } else {
        format!("{existing}\n{line}")
    };

    // FIFO truncation: drop leading lines until within cap.
    while combined.chars().count() > SESSION_INDEX_MAX_CHARS {
        match combined.find('\n') {
            Some(i) => {
                combined.drain(..=i);
            }
            None => break,
        };
    }

    if let Err(e) = sqlite::upsert_memory_block(
        db,
        agent_id,
        "session_index",
        &combined,
        Some("Timeline index of evicted session summaries (Phase C)"),
        Some(SESSION_INDEX_MAX_CHARS),
    ) {
        tracing::debug!(
            "consolidate [{}]: failed to update session_index: {}",
            agent_id,
            e
        );
        return;
    }
    let _ = sqlite::set_memory_tier(db, agent_id, "session_index", "pinned", false);
}

/// Return the first non-empty, trimmed line of `s`, capped at 200 chars.
fn first_nonempty_line(s: &str) -> String {
    for line in s.lines() {
        let t = line.trim();
        if !t.is_empty() {
            return t.chars().take(200).collect();
        }
    }
    String::new()
}

/// Sanitize a line for inclusion in `session_index`: strip newlines,
/// collapse internal whitespace, cap at 200 chars.
fn sanitize_index_line(s: &str) -> String {
    let collapsed: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    collapsed.chars().take(200).collect()
}

/// Truncate `s` from the head so the result has at most `max_chars` chars.
/// Preserves the tail (most recent content). If already within cap, returns
/// `s` unchanged.
fn truncate_head_to(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_string();
    }
    let skip = total - max_chars;
    s.chars().skip(skip).collect()
}

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

    // ── Phase C: pure helper tests ───────────────────────────────────────

    #[test]
    fn truncate_head_to_preserves_tail() {
        let out = truncate_head_to("abcdefghij", 4);
        assert_eq!(out, "ghij");
    }

    #[test]
    fn truncate_head_to_noop_when_under_cap() {
        let out = truncate_head_to("abc", 100);
        assert_eq!(out, "abc");
    }

    #[test]
    fn truncate_head_to_handles_multibyte() {
        // 5 chars, each multi-byte
        let out = truncate_head_to("αβγδε", 3);
        assert_eq!(out.chars().count(), 3);
        assert_eq!(out, "γδε");
    }

    #[test]
    fn first_nonempty_line_skips_blank_lines() {
        let out = first_nonempty_line("\n\n  \nhello world\nnext");
        assert_eq!(out, "hello world");
    }

    #[test]
    fn first_nonempty_line_empty_input() {
        assert_eq!(first_nonempty_line(""), "");
        assert_eq!(first_nonempty_line("\n\n  \n"), "");
    }

    #[test]
    fn first_nonempty_line_caps_at_200() {
        let long = "x".repeat(500);
        let out = first_nonempty_line(&long);
        assert_eq!(out.chars().count(), 200);
    }

    #[test]
    fn sanitize_index_line_collapses_whitespace() {
        let out = sanitize_index_line("hello\n  world\t\tfoo");
        assert_eq!(out, "hello world foo");
    }

    #[test]
    fn sanitize_index_line_caps_at_200() {
        let long = "a ".repeat(200);
        let out = sanitize_index_line(&long);
        assert_eq!(out.chars().count(), 200);
    }

    // ── M2: per-role preview limits + tighter noisy-tool filter ──────────

    #[test]
    fn m2_preview_limit_assistant_is_1200() {
        assert_eq!(preview_limit_for_role("assistant"), 1_200);
    }

    #[test]
    fn m2_preview_limit_tool_is_800() {
        assert_eq!(preview_limit_for_role("tool"), 800);
    }

    #[test]
    fn m2_preview_limit_user_is_400() {
        assert_eq!(preview_limit_for_role("user"), 400);
    }

    #[test]
    fn m2_preview_limit_unknown_role_falls_back_to_user_limit() {
        // Unknown roles must get the smallest limit so an unexpected role cannot
        // flood the summary prompt.
        assert_eq!(preview_limit_for_role("system"), 400);
        assert_eq!(preview_limit_for_role(""), 400);
    }

    #[test]
    fn m2_should_skip_noisy_tool_returns_false_for_any_content() {
        // The noisy-tool-skip heuristic was removed in M2: the MAX_SUMMARY_INPUT_CHARS
        // cap is the only safeguard against runaway input. Short success confirmations
        // like "ok" and "done" must now survive into the summary prompt so the LLM
        // knows a tool ran successfully.
        assert!(!should_skip_noisy_tool("tool", "ok"));
        assert!(!should_skip_noisy_tool("tool", "done"));
        assert!(!should_skip_noisy_tool("tool", "nothing"));
        assert!(!should_skip_noisy_tool("tool", "a/b"));
        assert!(!should_skip_noisy_tool("tool", "E42"));
    }

    #[test]
    fn m2_should_skip_noisy_tool_still_skips_empty() {
        // Empty/whitespace-only content is already filtered earlier via
        // `trimmed.is_empty()`, but should_skip_noisy_tool must not re-introduce
        // the old behaviour for it.
        assert!(!should_skip_noisy_tool("tool", ""));
    }

    #[test]
    fn m2_should_skip_noisy_tool_never_skips_non_tool_roles() {
        // The filter only applies to `role == "tool"`; user/assistant messages
        // are never dropped by this rule.
        assert!(!should_skip_noisy_tool("user", "hi"));
        assert!(!should_skip_noisy_tool("assistant", "ok"));
    }

    // ── M3: eager consolidation trigger (turn-count based) ───────────────

    #[test]
    fn m3_eager_first_time_triggers_when_at_or_above_threshold() {
        // With last_consolidation_turn = 0 and current = threshold, trigger.
        assert!(should_eager_consolidate(
            /* current */ EAGER_CONSOLIDATION_TURN_THRESHOLD,
            /* last    */ 0,
            EAGER_CONSOLIDATION_TURN_THRESHOLD,
        ));
    }

    #[test]
    fn m3_eager_does_not_trigger_before_threshold() {
        // current - last < threshold → no eager consolidation.
        assert!(!should_eager_consolidate(
            /* current */ EAGER_CONSOLIDATION_TURN_THRESHOLD - 1,
            /* last    */ 0,
            EAGER_CONSOLIDATION_TURN_THRESHOLD,
        ));
    }

    #[test]
    fn m3_eager_does_not_double_fire_within_threshold_window() {
        // After a previous eager run stamped last = 25, we must not re-fire at
        // turn 30 if threshold = 10 (gap 5 < 10).
        assert!(!should_eager_consolidate(30, 25, 10));
    }

    #[test]
    fn m3_eager_fires_again_after_threshold_gap() {
        // After a previous eager run stamped last = 25, turn 35 (gap 10) should re-fire.
        assert!(should_eager_consolidate(35, 25, 10));
    }

    #[test]
    fn m3_eager_handles_current_equal_to_last() {
        // Edge case: current == last (shouldn't normally happen but must be safe).
        assert!(!should_eager_consolidate(10, 10, 5));
    }

    #[test]
    fn m3_eager_handles_current_less_than_last() {
        // Defensive: if the counter is ever somehow below last_consolidation_turn,
        // saturating arithmetic must prevent a panic and must not trigger.
        assert!(!should_eager_consolidate(5, 10, 5));
    }

    #[test]
    fn m3_eager_threshold_constant_is_sane() {
        // The threshold must be > 0 (else eager fires on every turn) and should
        // be well below the 80-turn STALE_THRESHOLD so consolidation wins the
        // race against promote_stale_blocks. A value in 10..=40 is reasonable.
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(EAGER_CONSOLIDATION_TURN_THRESHOLD >= 10);
            assert!(EAGER_CONSOLIDATION_TURN_THRESHOLD <= 40);
        }
    }

    // ── Phase C: DB-backed ring tests ────────────────────────────────────

    use cade_store::sqlite::{self as store_sqlite, AgentRow, Db};

    fn setup_db() -> Db {
        let db = store_sqlite::open(":memory:").expect("open in-memory db");
        store_sqlite::create_agent(
            &db,
            &AgentRow {
                id: "a1".into(),
                name: "A".into(),
                model: "m".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
            },
        )
        .unwrap();
        db
    }

    fn block_value(db: &Db, label: &str) -> Option<String> {
        store_sqlite::get_memory_blocks(db, "a1")
            .unwrap()
            .into_iter()
            .find(|(l, _, _)| l == label)
            .map(|(_, v, _)| v)
    }

    #[test]
    fn rotate_writes_prev_live_to_slot_1() {
        let db = setup_db();
        rotate_and_archive_session_summary_db(&db, "a1", "FIRST summary content");
        assert_eq!(
            block_value(&db, "session_summary_1").as_deref(),
            Some("FIRST summary content")
        );
        assert!(block_value(&db, "session_summary_2").is_none());
    }

    #[test]
    fn rotate_empty_input_is_noop() {
        let db = setup_db();
        rotate_and_archive_session_summary_db(&db, "a1", "   \n  ");
        assert!(block_value(&db, "session_summary_1").is_none());
    }

    #[test]
    fn rotate_shifts_slots_and_fills_slot_1() {
        let db = setup_db();
        rotate_and_archive_session_summary_db(&db, "a1", "ONE");
        rotate_and_archive_session_summary_db(&db, "a1", "TWO");
        rotate_and_archive_session_summary_db(&db, "a1", "THREE");
        assert_eq!(block_value(&db, "session_summary_1").as_deref(), Some("THREE"));
        assert_eq!(block_value(&db, "session_summary_2").as_deref(), Some("TWO"));
        assert_eq!(block_value(&db, "session_summary_3").as_deref(), Some("ONE"));
        assert!(block_value(&db, "session_summary_4").is_none());
    }

    #[test]
    fn rotate_evicts_to_session_index_when_ring_full() {
        let db = setup_db();
        // Fill RING_CAP slots (5).
        rotate_and_archive_session_summary_db(&db, "a1", "Summary ONE first line\nmore");
        rotate_and_archive_session_summary_db(&db, "a1", "Summary TWO first line\nmore");
        rotate_and_archive_session_summary_db(&db, "a1", "Summary THREE first line\nmore");
        rotate_and_archive_session_summary_db(&db, "a1", "Summary FOUR first line\nmore");
        rotate_and_archive_session_summary_db(&db, "a1", "Summary FIVE first line\nmore");
        // All 5 slots should now be occupied, no index yet.
        assert!(block_value(&db, "session_summary_5").is_some());
        assert!(block_value(&db, "session_index").is_none());

        // One more rotation — "ONE" should be evicted to session_index.
        rotate_and_archive_session_summary_db(&db, "a1", "Summary SIX first line\nmore");
        let index = block_value(&db, "session_index").expect("index block must exist");
        assert!(
            index.contains("Summary ONE first line"),
            "expected ONE's first line in index, got: {index}"
        );
        // Ring still bounded at 5.
        assert!(block_value(&db, "session_summary_5").is_some());
        assert!(block_value(&db, "session_summary_6").is_none());
        // Slot 1 has the newest.
        assert_eq!(
            block_value(&db, "session_summary_1").as_deref(),
            Some("Summary SIX first line\nmore")
        );
    }

    #[test]
    fn session_index_fifo_truncates_when_over_cap() {
        let db = setup_db();
        // Pre-seed session_index near the cap.
        let big = "X".repeat(SESSION_INDEX_MAX_CHARS - 10);
        store_sqlite::upsert_memory_block(
            &db,
            "a1",
            "session_index",
            &big,
            Some("seed"),
            Some(SESSION_INDEX_MAX_CHARS + 1000),
        )
        .unwrap();

        // Append a line long enough to push over cap — should trigger drain.
        append_to_session_index_db(&db, "a1", &"y".repeat(100));
        let v = block_value(&db, "session_index").unwrap();
        assert!(
            v.chars().count() <= SESSION_INDEX_MAX_CHARS,
            "expected ≤ {} chars, got {}",
            SESSION_INDEX_MAX_CHARS,
            v.chars().count()
        );
    }

    #[test]
    fn rotated_slot_capped_at_archived_max_chars() {
        let db = setup_db();
        let huge = "Z".repeat(SESSION_SUMMARY_ARCHIVED_MAX_CHARS * 3);
        rotate_and_archive_session_summary_db(&db, "a1", &huge);
        let v = block_value(&db, "session_summary_1").unwrap();
        assert_eq!(v.chars().count(), SESSION_SUMMARY_ARCHIVED_MAX_CHARS);
        // Tail-preserving truncation: still all Zs.
        assert!(v.chars().all(|c| c == 'Z'));
    }

    // ─────────────────────────────────────────────────────────────────────
    // M4 — End-to-end consolidation round-trip regression test
    // ─────────────────────────────────────────────────────────────────────
    //
    // Protects the full pipeline: many dropped turns → `consolidate_agent`
    // → `session_summary` memory block written with LLM output → block is
    // `pinned` so the next context build surfaces it even after restart.
    //
    // This is the first test that exercises the whole round-trip through
    // the real consolidation code path using an in-process mock LLM.
    //
    // Gap this test closes: prior to M4 no test verified that `consolidate_agent`
    // actually writes a usable `session_summary` block — only rotation, turn
    // grouping, and inflation-guard pieces were covered in isolation.

    use async_trait::async_trait;
    use cade_ai::{
        AiConfig, CompletionRequest, CompletionResponse, LlmProvider, LlmRouter,
        StreamChunk,
    };
    use cade_ai::Result as AiResult;
    use crate::server::config::{LlmProviderKind, ServerConfig};
    use crate::server::rate_limit::RateLimiter;
    use crate::server::state::AppState;
    use futures::Stream;
    use std::net::SocketAddr;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::RwLock as AsyncRwLock;

    /// Mock LLM provider that returns a fixed summary string and counts calls.
    struct MockSummaryLlm {
        summary: String,
        calls: AtomicUsize,
    }

    impl MockSummaryLlm {
        fn new(summary: impl Into<String>) -> Self {
            Self {
                summary: summary.into(),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockSummaryLlm {
        async fn complete(&self, _req: &CompletionRequest) -> AiResult<CompletionResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CompletionResponse {
                content: Some(self.summary.clone()),
                tool_calls: Vec::new(),
                finish_reason: "stop".into(),
            })
        }

        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> AiResult<Pin<Box<dyn Stream<Item = AiResult<StreamChunk>> + Send>>> {
            // Consolidation only ever calls complete(); stream must exist to satisfy
            // the trait but is never invoked in this test.
            Err(cade_ai::Error::custom("stream not supported in mock"))
        }
    }

    /// Build a minimal AppState around an in-memory DB and a mock LLM.
    fn mk_state(db: cade_store::sqlite::Db, llm: Arc<dyn LlmProvider>) -> AppState {
        let ai_cfg = AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: "http://localhost:11434".into(),
            llm_provider: "ollama".into(),
        };
        let router = Arc::new(AsyncRwLock::new(LlmRouter::build(&ai_cfg)));

        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let cfg = ServerConfig {
            addr,
            db_path: ":memory:".into(),
            llm_provider: LlmProviderKind::Ollama,
            default_model: "m".into(),
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: "http://localhost:11434".into(),
            api_key: None,

        allowed_origin: None,
        max_context_budget: None,
        };

        AppState {
            db,
            llm,
            llm_router: router,
            config: Arc::new(cfg),
            mcp: Arc::new(crate::server::state::McpManager::empty()),
            rate_limiter: RateLimiter::from_env(),
            memory_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            agent_activity: Arc::new(AsyncRwLock::new(std::collections::HashMap::new())),
            agent_metrics: Arc::new(AsyncRwLock::new(std::collections::HashMap::new())),
            context_cache: Arc::new(std::sync::Mutex::new(lru::LruCache::new(std::num::NonZeroUsize::new(20).unwrap()))),
            all_skills: Arc::new(AsyncRwLock::new(Vec::new())),
            agent_skills: Arc::new(AsyncRwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Seed `n` fake user↔assistant turns (content each ~`payload_chars` chars) so
    /// that consolidation has enough dropped content to summarise.
    fn seed_turns(db: &cade_store::sqlite::Db, agent_id: &str, n: usize, payload_chars: usize) {
        use cade_store::sqlite::MessageRow;
        for i in 0..n {
            let user_body = format!(
                "turn {i}: please edit src/mod_{i}.rs and fix `fn compute_{i}`. {}",
                "x".repeat(payload_chars)
            );
            let asst_body = format!(
                "turn {i}: I edited src/mod_{i}.rs — updated `fn compute_{i}`. error code E{:04}. {}",
                i,
                "y".repeat(payload_chars)
            );
            store_sqlite::insert_message(
                db,
                &MessageRow {
                    id: format!("u-{i}"),
                    agent_id: agent_id.into(),
                    conversation_id: None,
                    role: "user".into(),
                    content: serde_json::json!({ "content": user_body }),
                    char_count: user_body.chars().count(),
                },
            )
            .unwrap();
            store_sqlite::insert_message(
                db,
                &MessageRow {
                    id: format!("a-{i}"),
                    agent_id: agent_id.into(),
                    conversation_id: None,
                    role: "assistant".into(),
                    content: serde_json::json!({ "content": asst_body }),
                    char_count: asst_body.chars().count(),
                },
            )
            .unwrap();
        }
    }

    #[tokio::test]
    async fn m4_consolidation_round_trip_writes_pinned_session_summary() {
        // ── arrange ─────────────────────────────────────────────────────
        let db = setup_db(); // agent "a1", model "m" (unknown → 32 000 token window)
        let agent_id = "a1";

        // Seed enough turns that the older ones will not fit in HISTORY_BUDGET_FRACTION (40%)
        // of the estimated char budget. With model "m" → 32 000 tokens → ~81 600 char budget
        // → ~32 640 char history budget. 40 turns × ~4200 chars/turn ≈ 168 000 chars ⇒ most
        // turns must be classified as dropped, guaranteeing consolidate_agent reaches the
        // "write session_summary" branch.
        seed_turns(&db, agent_id, 40, 2_000);

        let mock_summary = "MOCK_ROUND_TRIP_SUMMARY: rewrote src/mod_3.rs, fixed fn compute_7, error E0042 resolved.";
        let llm = Arc::new(MockSummaryLlm::new(mock_summary));
        let llm_trait: Arc<dyn LlmProvider> = llm.clone();
        let state = mk_state(db.clone(), llm_trait);

        // ── act ─────────────────────────────────────────────────────────
        consolidate_agent(&state, agent_id, None).await;

        // ── assert ──────────────────────────────────────────────────────

        // 1. The mock LLM's complete() was invoked exactly once.
        assert_eq!(
            llm.calls.load(Ordering::SeqCst),
            1,
            "consolidate_agent must call LLM.complete exactly once when there are dropped turns"
        );

        // 2. `session_summary` block exists and contains the mock output verbatim.
        let blocks = store_sqlite::get_memory_blocks(&db, agent_id).unwrap();
        let summary_block = blocks
            .iter()
            .find(|(l, _, _)| l == "session_summary")
            .expect("session_summary block must be written after consolidation");
        assert!(
            summary_block.1.contains("MOCK_ROUND_TRIP_SUMMARY"),
            "session_summary must contain LLM's summary text; got: {}",
            summary_block.1
        );

        // 3. `session_summary` is `pinned` tier so it is not subject to
        //    promote_stale_blocks demotion on future context builds.
        let active = store_sqlite::get_active_blocks(&db, agent_id).unwrap();
        let (_, _, _, tier, _) = active
            .iter()
            .find(|(l, _, _, _, _)| l == "session_summary")
            .expect("session_summary must appear in active (pinned+short) blocks");
        assert_eq!(
            tier, "pinned",
            "session_summary must be pinned so next build_context always injects it"
        );
    }
}
