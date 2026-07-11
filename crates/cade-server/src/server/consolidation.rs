//! Background memory consolidation — the "Sleeptime Agent".
//!
//! When the budget-based context builder in `build_context()` drops older turns
//! from the LLM prompt it sets `needs_consolidation = true` in `agent_activity`.
//! After 20 s of agent inactivity the Sleeptime background task calls
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
        return Some(
            std::path::PathBuf::from(custom)
                .join(agent_id)
                .join("memory"),
        );
    }
    dirs::home_dir().map(|h| h.join(".cade").join("rag").join(agent_id).join("memory"))
}

// ── tunables ──────────────────────────────────────────────────────────────────

/// Minimum number of DB rows required before consolidation is attempted.
/// Below this the conversation is too short to be worth summarising.
const MIN_ROWS_FOR_CONSOLIDATION: usize = 20;

/// Maximum chars of formatted history text fed to the summarisation LLM call.
/// P5: doubled from 24k → 48k so more dropped-turn detail survives into the
/// summary. At 3 chars/token this is ~16k input tokens on the compaction model.
const MAX_SUMMARY_INPUT_CHARS: usize = 48_000;

/// Maximum tokens the summarisation LLM is allowed to emit.
/// P5: raised from 900 → 1500 so the summary can preserve more decisions,
/// error details, and reasoning chains.
const SUMMARY_MAX_TOKENS: u32 = 1_500;

/// Maximum chars stored in the `session_summary` memory block.
/// P5: raised from 4,500 → 8,000. The extra 3.5k of prompt budget is
/// acceptable on 128k+ context windows and dramatically reduces detail loss.
const SESSION_SUMMARY_MAX_CHARS: usize = 8_000;

/// Phase C: maximum number of rotated `session_summary_N` blocks to keep in
/// the long-term tier. When the ring fills, the oldest is evicted and a
/// one-line excerpt is appended to the pinned `session_index` block.
/// P5: raised from 5 → 8 for longer project continuity.
const SESSION_SUMMARY_RING_CAP: usize = 8;

/// Max chars retained per rotated `session_summary_N` block. Lower than the
/// live cap because older phases get less frequent attention.
/// P5: raised from 2,000 → 4,000 to preserve more cross-session history.
const SESSION_SUMMARY_ARCHIVED_MAX_CHARS: usize = 4_000;

/// Max chars retained in the `session_index` pinned block. When the FIFO
/// line-buffer exceeds this, the oldest lines are dropped.
/// A7: raised from 5,000 → 10,000 for richer session continuity.
const SESSION_INDEX_MAX_CHARS: usize = 10_000;

/// Maximum tokens for the P7 active_goal auto-update LLM call.
// const ACTIVE_GOAL_UPDATE_MAX_TOKENS: u32 = 400;

/// Fraction of the estimated history budget used as the threshold: turns that
/// fit within `char_budget * HISTORY_BUDGET_FRACTION` are considered "in
/// context"; everything older is considered "dropped" and summarised.
const HISTORY_BUDGET_FRACTION: f64 =
    crate::server::api::messages::PROACTIVE_CONSOLIDATION_THRESHOLD;

/// Characters per token approximation (conservative).
const CHARS_PER_TOKEN: usize = 3;

/// Resolve the cheapest capable summarisation model for a given primary model.
///
/// Compaction is structurally simple (single-shot summarisation, ~900 output
/// tokens) — it does not need a frontier model.  Auto-defaulting to a cheap
/// variant cuts ongoing background cost by 10–20× without measurable quality
/// loss on the summary task.
///
/// Mapping rules (provider prefix match):
///   * `anthropic/*`  → `anthropic/claude-haiku-4-5`
///   * `openai/*`     → `openai/gpt-4o-mini`
///   * `gemini/*`     → `gemini/gemini-2.5-flash`
///   * `openrouter/*:<free>` → passthrough (free-tier models share the same
///     strict 20 RPM / 200 RPD rate limit; using a different free model for
///     compaction would compete with the primary for the same quota).
///   * `openrouter/*` → `openrouter/z-ai/glm-4.5-air:free` (paid tier — cheap
///     enough to keep costs low without competing for the primary quota).
///   * anything else  → passthrough (e.g. `ollama/*` runs locally; unknown
///     providers don't have a guaranteed cheaper variant).
///
/// Idempotent: if the input is already a known cheap variant the same string
/// is returned, so this can be called unconditionally without the risk of
/// degrading an already-cheap configuration.
pub(crate) fn default_compaction_model(primary_model: &str) -> String {
    if primary_model.starts_with("openrouter/") {
        // Free-tier OpenRouter models (ending in `:free`) share a strict
        // 20 RPM / 200 RPD rate limit.  Using a different free model for
        // compaction would compete for the same limited quota, so passthrough.
        if primary_model.ends_with(":free") {
            return primary_model.to_string();
        }
        return "openrouter/z-ai/glm-4.5-air:free".to_string();
    }
    if primary_model.starts_with("anthropic/") {
        return "anthropic/claude-haiku-4-5".to_string();
    }
    if primary_model.starts_with("openai/") {
        return "openai/gpt-4o-mini".to_string();
    }
    if primary_model.starts_with("gemini/") {
        return "gemini/gemini-2.5-flash".to_string();
    }
    // Unknown provider (incl. ollama/*, custom): preserve passthrough — local
    // models cost nothing and unknown providers may not have a cheaper SKU.
    primary_model.to_string()
}

// ── preview / filter helpers (M2) ────────────────────────────────────────────

/// Maximum chars kept per message in the history text fed to the summariser.
///
/// Limits are per-role because assistant turns carry the highest-signal
/// technical content (file edits, decisions, error reports) and were being
/// clipped at the old flat 600-char cap. Tool outputs are medium-signal;
/// user prompts are shortest on average. Unknown roles get the smallest
/// limit to prevent an unexpected role from flooding the summariser.
/// P5: raised assistant from 1200→2000 to preserve more technical detail
/// (file edits, decisions, error reports) in the consolidation input.
fn preview_limit_for_role(role: &str) -> usize {
    match role {
        "assistant" => 2_000,
        "tool" => 1_200,
        "user" => 600,
        _ => 400,
    }
}

/// Whether to drop a tool message from the summary prompt as pure noise.
///
/// M2: the old heuristic (`len < 15 && no '/' && no digit`) incorrectly
/// dropped legitimate short confirmations such as `"ok"` or `"done"`, making
/// the summariser think those tools never ran.
///
/// M5: removed — the function had become a permanent no-op (always
/// returned `false`).  The `MAX_SUMMARY_INPUT_CHARS` cap upstream is the
/// only safeguard against runaway input, and whitespace-only content is
/// already filtered via `trimmed.is_empty()` at the call site.

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
/// through the existing `Arc<parking_lot::Mutex<Connection>>` pool.
pub async fn consolidate_agent(
    state: &AppState,
    agent_id: &str,
    conversation_id: Option<&str>,
    override_history_budget: Option<usize>,
) -> Option<usize> {
    let agent = match sqlite::get_agent(&state.db, agent_id) {
        Ok(Some(a)) => a,
        Ok(None) => {
            tracing::warn!(agent_id = %agent_id, "consolidate:  agent not found — skipping");
            return None;
        }
        Err(e) => {
            tracing::warn!("consolidate [{}]: DB error: {}", agent_id, e);
            return None;
        }
    };

    // ── 1. Fetch messages since the last compaction marker ───────────────────
    // Using `list_messages_since_last_compaction` ensures we only summarise
    // turns that have NOT yet been covered by a previous consolidation run.
    // `list_messages_page` returned ALL rows (ignoring markers), causing the
    // consolidation LLM to re-summarise already-compacted history on every
    // invocation — producing duplicate session_summary entries.
    let all_rows =
        sqlite::list_messages_since_last_compaction(&state.db, agent_id, conversation_id, 500)
            .unwrap_or_default();

    if all_rows.len() < MIN_ROWS_FOR_CONSOLIDATION {
        tracing::debug!(
            "consolidate [{}]: only {} rows — skipping",
            agent_id,
            all_rows.len()
        );
        return None;
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
    let history_budget = override_history_budget
        .unwrap_or_else(|| (char_budget as f64 * HISTORY_BUDGET_FRACTION).round() as usize);

    let max_turn_chars = state
        .config
        .max_tokens_per_turn
        .map(cade_ai::chars_for_tokens)
        .unwrap_or(64_000);
    let turns = group_turns(&flat, max_turn_chars);
    let total_turns = turns.len();

    let budget_manager = cade_ai::PromptBudgetManager::new();

    let mut in_context = 0usize;
    let mut used = 0usize;
    for turn in turns.iter().rev() {
        let mut total_tokens = 0usize;
        let mut fallback_chars = 0usize;
        for (_, text) in turn {
            if !text.is_empty() {
                total_tokens += budget_manager.count_tokens(&agent.model, text);
            }
            fallback_chars += text.chars().count();
        }
        let chars = if total_tokens == 0 && fallback_chars > 0 {
            fallback_chars
        } else {
            budget_manager.chars_for_tokens(total_tokens)
        };

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
        return None;
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
            // M5: removed `should_skip_noisy_tool` filter — it was a
            // permanent no-op.  Whitespace-only content is filtered above
            // and `MAX_SUMMARY_INPUT_CHARS` caps total input.
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
            // P3: High-priority messages get 2× preview cap to preserve detail.
            let base_cap = preview_limit_for_role(role);
            let priority_boost = is_high_priority_message(role, trimmed);
            let preview_cap = if priority_boost {
                base_cap * 2
            } else {
                base_cap
            };
            let preview: String = if trimmed.chars().count() > preview_cap {
                format!("{}…", trimmed.chars().take(preview_cap).collect::<String>())
            } else {
                trimmed.to_string()
            };
            history_text.push_str(&format!("[{role}{artifact_prefix}] {preview}\n"));
        }
    }

    if history_text.trim().is_empty() {
        tracing::debug!(agent_id = %agent_id, "consolidate:  dropped turns have no useful text — skipping");
        return None;
    }

    // ── 3b. F2: Cache full dropped turns into archival memory ────────────────
    //
    // BEFORE the LLM lossy summarisation, write the *full untruncated* text
    // of every dropped turn to archival memory.  This makes the original
    // dialogue recoverable via `archival_memory_search` even after the
    // session_summary entry is overwritten or rotated.  Without this cache,
    // the only post-compaction trace of the raw turns was the truncated
    // preview baked into `history_text` — which is fed to the LLM and then
    // discarded.
    //
    // We build the archival payload from the *flat* role+text pairs (full
    // length, no per-role caps), but still cap the total at
    // MAX_ARCHIVAL_PAYLOAD_CHARS so a single compaction event cannot blow up
    // the archival store.
    let mut files_touched_block = String::new();
    {
        const MAX_ARCHIVAL_PAYLOAD_CHARS: usize = 64_000;
        let dropped_msg_count: usize = turns[..dropped].iter().map(|t| t.len()).sum();
        let mut payload = String::with_capacity(8_192);
        payload.push_str(&format!(
            "Dropped turns from agent {agent_id} (consolidation pass).\n\
             Source: pre-compaction conversation history.\n\
             Turn count: {dropped} | Message count: {dropped_msg_count}\n\
             ---\n\n"
        ));
        let mut truncated = false;
        'outer: for turn in &turns[..dropped] {
            for (role, text) in turn {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let entry = format!("[{role}] {trimmed}\n\n");
                if payload.chars().count() + entry.chars().count() > MAX_ARCHIVAL_PAYLOAD_CHARS {
                    payload.push_str("\n[…remaining dropped turns truncated for archival cap…]");
                    truncated = true;
                    break 'outer;
                }
                payload.push_str(&entry);
            }
        }

        let mut tags = vec![
            "consolidation".to_string(),
            "dropped-turns".to_string(),
            format!("agent:{agent_id}"),
        ];
        if let Some(cid) = conversation_id {
            tags.push(format!("conversation:{cid}"));
        }
        if truncated {
            tags.push("truncated".to_string());
        }
        match sqlite::insert_archival_memory(&state.db, agent_id, payload.trim_end(), &tags) {
            Ok(id) => tracing::debug!(
                "consolidate [{}]: cached {} dropped turn(s) ({} chars) to archival id={}",
                agent_id,
                dropped,
                payload.chars().count(),
                id,
            ),
            Err(e) => tracing::warn!(
                "consolidate [{}]: failed to cache dropped turns to archival: {}",
                agent_id,
                e,
            ),
        }

        // ── 3c. Cumulative Deterministic File Tracking ───────────────────────────
        // Extract file paths from the full untruncated payload so the LLM doesn't have to guess.
        let mut files = std::collections::HashSet::new();
        if let Ok(re_path) = regex::Regex::new(r#""path"\s*:\s*"([^"]+)""#) {
            for cap in re_path.captures_iter(&payload) {
                files.insert(cap[1].to_string());
            }
        }
        if let Ok(re_file) = regex::Regex::new(r#""file"\s*:\s*"([^"]+)""#) {
            for cap in re_file.captures_iter(&payload) {
                files.insert(cap[1].to_string());
            }
        }
        let mut vec: Vec<_> = files.into_iter().collect();
        vec.sort();
        if !vec.is_empty() {
            files_touched_block =
                format!("\n\n<files_touched>\n{}\n</files_touched>", vec.join("\n"));
        }
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
         {history_text}{files_touched_block}"
    );

    // Use the compaction model if configured, otherwise auto-default to the
    // cheapest capable model for the agent's provider (P3).  Compaction is a
    // recurring background cost: summarising 24 KB of history with a frontier
    // model can cost 10–20× more than a cheap variant for negligible quality
    // loss on a structurally simple task.  See `default_compaction_model`.
    let compaction_model = agent
        .compaction_model
        .as_deref()
        .filter(|m| !m.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_compaction_model(&agent.model));

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
            return None;
        }
    };

    if summary.is_empty() {
        tracing::debug!(agent_id = %agent_id, "consolidate:  LLM returned empty summary");
        return None;
    }

    // ── 4b. Inflation Guard & Regex Fallback ──
    let dropped_chars = history_text.chars().count();
    let summary_chars = summary.chars().count();

    let final_summary = if is_summary_inflated(summary_chars, dropped_chars) {
        tracing::warn!(
            "consolidate [{}]: summary inflated ({} chars) vs dropped ({} chars) — falling back to regex anchors",
            agent_id,
            summary_chars,
            dropped_chars,
        );
        let metrics = state.agent_metrics.clone();
        metrics
            .entry(agent_id.to_string())
            .or_default()
            .inflation_guard_hits
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Fast regex fallback instead of skipping entirely
        let mut anchors = std::collections::HashSet::new();

        if let Ok(re_file) =
            regex::Regex::new(r"(/[\w\./-]+\.\w+|src/[\w\./-]+\.\w+|crates/[\w\./-]+\.\w+)")
        {
            for cap in re_file.captures_iter(&history_text) {
                anchors.insert(cap[1].to_string());
            }
        }

        if let Ok(re_tool) = regex::Regex::new(r"(\w+__\w+)") {
            for cap in re_tool.captures_iter(&history_text) {
                if !cap[1].starts_with("default_api") {
                    // basic filter
                    anchors.insert(cap[1].to_string());
                }
            }
        }

        if let Ok(re_err) = regex::Regex::new(r"(error\[E\d+\]|panic at|Exception)") {
            for cap in re_err.captures_iter(&history_text) {
                anchors.insert(cap[1].to_string());
            }
        }

        let mut vec: Vec<_> = anchors.into_iter().collect();
        vec.sort();
        vec.truncate(8);
        format!("SEARCH ANCHORS: {}", vec.join(", "))
    } else {
        summary.clone()
    };

    if final_summary.is_empty() {
        return None;
    }

    // ── 5. Write to the `session_summary` memory block ───────────────────────
    // Append to the existing summary or start fresh; cap at SESSION_SUMMARY_MAX_CHARS.
    let existing_blocks = sqlite::get_memory_blocks(&state.db, agent_id).unwrap_or_default();
    let existing = existing_blocks
        .iter()
        .find(|(label, _, _)| label == "session_summary")
        .map(|(_, val, _)| val.as_str())
        .unwrap_or("");

    // Extract newly touched files from the dropped turns being consolidated
    let (new_read, new_mod) = extract_touched_files(&all_rows);

    // Parse any existing touched files from the existing summary block
    let (mut accum_read, mut mut_mod) = parse_existing_touched_files(existing);
    
    // Merge lists
    for r in new_read {
        accum_read.insert(r);
    }
    for m in new_mod {
        mut_mod.insert(m);
    }

    let mut accum_read_vec: Vec<String> = accum_read.into_iter().collect();
    let mut mut_mod_vec: Vec<String> = mut_mod.into_iter().collect();
    accum_read_vec.sort();
    mut_mod_vec.sort();

    let files_metadata = format_touched_files_section(&accum_read_vec, &mut_mod_vec);

    // Strip file metadata from the summaries before the merge pass
    let clean_existing = strip_touched_files_section(existing);
    let clean_final_summary = strip_touched_files_section(&final_summary);

    let new_value = if clean_existing.is_empty() {
        format!("{clean_final_summary}{files_metadata}")
    } else {
        let combined = format!("{clean_existing}\n\n---\n\n{clean_final_summary}");
        if combined.chars().count() > SESSION_SUMMARY_MAX_CHARS {
            // Under Candidate A: merge existing and new summaries iteratively
            // using an LLM pass to compress into a single, high-density summary
            // instead of rotating and splitting across multiple ring blocks.
            tracing::info!(
                "consolidate [{}]: merging existing and new summaries ({} chars total)",
                agent_id,
                combined.chars().count()
            );
            match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                merge_session_summaries(
                    state.clone(),
                    agent_id.to_string(),
                    clean_existing,
                    clean_final_summary,
                ),
            )
            .await
            {
                Ok(Ok(merged)) => {
                    tracing::info!(
                        "consolidate [{}]: successfully merged summaries ({} chars)",
                        agent_id,
                        merged.chars().count()
                    );
                    format!("{merged}{files_metadata}")
                }
                Ok(Err(e)) => {
                    tracing::warn!(
                        "consolidate [{}]: failed to merge summaries: {}. Falling back to ring rotation.",
                        agent_id,
                        e
                    );
                    rotate_and_archive_session_summary(state, agent_id, existing);
                    format!("{final_summary}{files_metadata}")
                }
                Err(_) => {
                    tracing::warn!(
                        "consolidate [{}]: merge summaries timed out. Falling back to ring rotation.",
                        agent_id
                    );
                    rotate_and_archive_session_summary(state, agent_id, existing);
                    format!("{final_summary}{files_metadata}")
                }
            }
        } else {
            format!("{combined}{files_metadata}")
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
        return None;
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

    // Box::pin: auto_extract_facts holds
    // an LLM CompletionRequest + response locals across their await point.
    // Boxing its future prevents it from inflating consolidate_agent's
    // already-large state machine, which contributes to the tokio worker
    // thread stack overflow on archival/historic content access paths.

    // Phase B: Automated Extraction of durable facts
    Box::pin(auto_extract_facts(
        state,
        agent_id,
        &summary,
        &compaction_model,
    ))
    .await;

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
            Err(e) => tracing::debug!("consolidate [{}]: rag export skipped: {}", agent_id, e),
        }
    }

    // ── 6. Insert a compaction marker into message history ───────────────────
    // The marker acts as a boundary: `get_context_window()` only loads messages
    // after the most recent marker, drastically reducing the scan set.
    //
    // Use the newest dropped message's ID to anchor the marker's timestamp.
    // The dropped turns are all_rows[0..dropped_msg_count] (oldest-first).
    // We want the marker's created_at to be equal to the newest dropped message.
    let dropped_msg_count: usize = turns[..dropped].iter().map(|t| t.len()).sum();

    // Look up the created_at of the boundary message from the DB.
    // all_rows is oldest-first; the boundary is at index dropped_msg_count - 1.
    let boundary_msg_id = if dropped_msg_count > 0 && dropped_msg_count <= all_rows.len() {
        Some(all_rows[dropped_msg_count - 1].id.clone())
    } else {
        None
    };

    if let Some(ref bid) = boundary_msg_id {
        let marker_ts = {
            let Ok(conn) = state.db.get() else {
                tracing::warn!("consolidate_agent: pool get failed; skipping marker");
                return None;
            };
            conn.query_row(
                "SELECT created_at FROM messages WHERE id = ?1",
                rusqlite::params![bid],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or_else(|_| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
            })
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
        {
            let Ok(conn) = state.db.get() else {
                tracing::warn!("consolidate_agent: pool get failed; skipping marker insert");
                return None;
            };
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

    let metrics = state.agent_metrics.clone();
    let m = metrics.entry(agent_id.to_string()).or_default();
    m.consolidation_runs
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    m.chars_summarised
        .fetch_add(dropped_chars, std::sync::atomic::Ordering::Relaxed);
    m.chars_produced
        .fetch_add(summary_chars, std::sync::atomic::Ordering::Relaxed);

    // ── P8: Prune old observations during consolidation ──────────────────
    // Keep the observation table bounded by removing entries from turns
    // that have been compacted.  The current turn counter is the high-water
    // mark; anything older than `current_turn - 100` is stale.
    let current_turn = sqlite::get_turn_counter(&state.db, agent_id).unwrap_or(0);
    let prune_before = (current_turn - 100).max(0);
    if let Ok(pruned) =
        sqlite::observations::prune_old_observations(&state.db, agent_id, prune_before)
        && pruned > 0
    {
        tracing::debug!(
            "consolidate [{}]: P8 pruned {} stale observations (before turn {})",
            agent_id,
            pruned,
            prune_before,
        );
    }

    // ── P9: Decay confidence of stale memories ─────────────────────────────────
    // Phase C: Memories that haven't been accessed recently decay in confidence.
    if let Ok(decayed) = cade_store::sqlite::memory::decay_stale_memories(
        &state.db,
        agent_id,
        current_turn,
        crate::server::api::messages::MemoryBudgets::decay_threshold_for_model(&agent.model),
    ) && decayed > 0
    {
        tracing::debug!(
            "consolidate [{}]: Phase C decayed confidence for {} stale memory blocks",
            agent_id,
            decayed
        );
    }

    Some(new_value.chars().count())
}

// ── P7: active_goal auto-update ───────────────────────────────────────────────

// ── P3: Event-driven consolidation priority ──────────────────────────────────

/// Detect whether a message contains high-priority signals that deserve
/// extra detail preservation during consolidation.
///
/// High-priority signals:
///   - Git commit messages (milestone reached)
///   - Test results (pass/fail state is critical context)
///   - Error corrections ("actually", "no, that's wrong")
///   - Decision statements ("decided", "chosen", "rejected")
///   - Memory updates (update_memory calls carry decision context)
fn is_high_priority_message(role: &str, content: &str) -> bool {
    let lower = content.to_lowercase();
    match role {
        "tool" => {
            // Git commits, test results, error outputs
            lower.contains("commit")
                || lower.contains("test result")
                || lower.contains("tests passed")
                || lower.contains("tests failed")
                || lower.contains("cargo test")
                || lower.contains("exit code")
                || lower.contains("error[e")
                || lower.contains("panicked at")
        }
        "user" => {
            // User corrections and explicit decisions
            lower.starts_with("no,")
                || lower.starts_with("actually")
                || lower.starts_with("wrong")
                || lower.contains("that's wrong")
                || lower.contains("not what i")
                || lower.contains("i decided")
                || lower.contains("let's go with")
                || lower.contains("approved")
        }
        "assistant" => {
            // Agent decisions and memory operations
            lower.contains("update_memory")
                || lower.contains("create_checkpoint")
                || lower.contains("decided to")
                || lower.contains("the approach")
                || lower.contains("rejected because")
        }
        _ => false,
    }
}

// ── P7: active_goal auto-update ───────────────────────────────────────────────

/// Phase B: Automated Extraction of durable facts from consolidation summaries.
///
/// This background task uses the cheap compaction model to scan the session summary
/// and automatically lift any explicitly durable facts (decisions, conventions, etc.)
/// into structured memory blocks with provenance mapping.
async fn auto_extract_facts(
    state: &AppState,
    agent_id: &str,
    summary: &str,
    compaction_model: &str,
) {
    if summary.trim().is_empty() {
        return;
    }

    let prompt = format!(
        "You are a memory extraction sub-agent. Based on this consolidation summary, \
         extract durable facts, decisions, and constraints into a JSON array.\n\
         \n\
         Each object in the array must have:\n\
         - \"label\": a short snake_case identifier (e.g. \"project_convention_auth\", \"decision_db_sqlite\")\n\
         - \"memory_type\": exactly one of [\"decision\", \"convention\", \"project_fact\", \"constraint\"]\n\
         - \"value\": a concise, factual description\n\
         - \"confidence\": a number between 0.0 and 1.0 (default to 1.0)\n\
         \n\
         Only extract durable knowledge that will be useful across sessions. Do NOT extract transient state.\n\
         If there are no new durable facts, return exactly: []\n\
         \n\
         SUMMARY:\n\
         {summary}"
    );

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
        max_tokens: 1000,
        reasoning_effort: None,
    };

    let response_text = match state.llm.complete(&req).await {
        Ok(resp) => resp.content.unwrap_or_default().trim().to_string(),
        Err(e) => {
            tracing::debug!(
                "consolidate [{}]: Phase B auto_extract_facts LLM failed: {}",
                agent_id,
                e
            );
            return;
        }
    };

    let clean_json = response_text
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    if clean_json.is_empty() || clean_json == "[]" {
        return;
    }

    if let Ok(facts) = serde_json::from_str::<Vec<serde_json::Value>>(clean_json) {
        let mut count = 0;
        for fact in facts {
            let label = fact["label"].as_str().unwrap_or("").to_string();
            let memory_type = fact["memory_type"]
                .as_str()
                .unwrap_or("generic")
                .to_string();
            let value = fact["value"].as_str().unwrap_or("").to_string();
            let confidence = fact["confidence"].as_f64().unwrap_or(1.0);

            if label.is_empty() || value.is_empty() {
                continue;
            }

            if let Err(e) = sqlite::upsert_memory_block_typed(
                &state.db,
                agent_id,
                &label,
                &value,
                Some("Auto-extracted by Phase B Consolidation"),
                Some(1000),
                Some(&memory_type),
                Some(confidence),
            ) {
                tracing::debug!(
                    "consolidate [{}]: auto-extraction failed to save block {}: {}",
                    agent_id,
                    label,
                    e
                );
            } else {
                // A2 Provenance: we attribute it to the consolidation turn
                let turn = cade_store::sqlite::get_turn_counter(&state.db, agent_id).unwrap_or(0);
                cade_store::sqlite::memory::stamp_provenance(
                    &state.db,
                    agent_id,
                    &label,
                    Some(turn),
                    None,
                    Some("auto_extraction"),
                    None,
                );
                // Chunk it for semantic search
                cade_store::sqlite::memory::rechunk_block(
                    &state.db,
                    agent_id,
                    &label,
                    &value,
                    state.embedder.as_ref().map(|e| e.as_ref()),
                );
                count += 1;
            }
        }
        if count > 0 {
            tracing::info!(
                "consolidate [{}]: Phase B auto-extracted {} durable facts from summary",
                agent_id,
                count
            );
        }
    } else {
        tracing::debug!(agent_id = %agent_id, "consolidate:  Phase B auto-extraction failed to parse JSON");
    }
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
        // A7: archive the full evicted content to archival_memory for recovery.
        if !val.trim().is_empty() {
            let tags = vec!["evicted-session-summary".to_string()];
            let _ = sqlite::insert_archival_memory(db, agent_id, val, &tags);
        }

        // A7: use a 500-char excerpt instead of first_nonempty_line (~200 chars).
        let excerpt = truncate_head_to(val, 500);
        let excerpt = excerpt.trim();
        if !excerpt.is_empty() {
            append_to_session_index_db(db, agent_id, excerpt);
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
#[allow(dead_code)] // A7: no longer used in production (replaced by 500-char truncate_head_to), but kept for tests
fn first_nonempty_line(s: &str) -> String {
    for line in s.lines() {
        let t = line.trim();
        if !t.is_empty() {
            return t.chars().take(200).collect();
        }
    }
    String::new()
}

/// Extract newly touched files from the consolidated message rows.
fn extract_touched_files(rows: &[sqlite::MessageRow]) -> (Vec<String>, Vec<String>) {
    let mut read_files = std::collections::HashSet::new();
    let mut modified_files = std::collections::HashSet::new();

    for row in rows {
        if let Some(tool_calls) = row.content.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tool_calls {
                let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = tc.get("arguments");
                if let Some(args_obj) = args {
                    if let Some(path) = args_obj.get("path").and_then(|v| v.as_str()) {
                        let clean_path = path.trim().to_string();
                        if !clean_path.is_empty() {
                            match name {
                                "read_file" | "view_file" => {
                                    read_files.insert(clean_path);
                                }
                                "write_file" | "edit_file" | "create_file" => {
                                    modified_files.insert(clean_path);
                                }
                                _ => {}
                            }
                        }
                    } else if name == "apply_patch"
                        && let Some(patch_str) = args_obj.get("patch").and_then(|v| v.as_str()) {
                            for line in patch_str.lines() {
                                if line.starts_with("+++ ") {
                                    let path_part = line["+++ ".len()..].trim();
                                    let clean_path = if path_part.starts_with("b/") {
                                        path_part["b/".len()..].to_string()
                                    } else {
                                        path_part.to_string()
                                    };
                                    if !clean_path.is_empty() && clean_path != "/dev/null" {
                                        modified_files.insert(clean_path);
                                    }
                                }
                            }
                        }
                }
            }
        }
    }

    let mut r_vec: Vec<String> = read_files.into_iter().collect();
    let mut m_vec: Vec<String> = modified_files.into_iter().collect();
    r_vec.sort();
    m_vec.sort();
    (r_vec, m_vec)
}

/// Parse any existing touched files from the existing summary block.
fn parse_existing_touched_files(summary: &str) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    let mut read = std::collections::HashSet::new();
    let mut modified = std::collections::HashSet::new();

    for line in summary.lines() {
        if line.starts_with("* Read: [") && line.ends_with(']') {
            let content = &line["* Read: [".len()..line.len() - 1];
            for p in content.split(',') {
                let cleaned = p.trim().to_string();
                if !cleaned.is_empty() {
                    read.insert(cleaned);
                }
            }
        } else if line.starts_with("* Modified: [") && line.ends_with(']') {
            let content = &line["* Modified: [".len()..line.len() - 1];
            for p in content.split(',') {
                let cleaned = p.trim().to_string();
                if !cleaned.is_empty() {
                    modified.insert(cleaned);
                }
            }
        }
    }

    (read, modified)
}

/// Format the touched files section to append to the summary.
fn format_touched_files_section(read: &[String], modified: &[String]) -> String {
    if read.is_empty() && modified.is_empty() {
        return String::new();
    }
    
    let mut section = String::new();
    section.push_str("\n\n### Files Checked in this Session:\n");
    if !read.is_empty() {
        section.push_str(&format!("* Read: [{}]\n", read.join(", ")));
    }
    if !modified.is_empty() {
        section.push_str(&format!("* Modified: [{}]\n", modified.join(", ")));
    }
    section
}

/// Strip the touched files section from a summary block to keep it pure for synthesis.
fn strip_touched_files_section(summary: &str) -> String {
    if let Some(pos) = summary.find("### Files Checked in this Session:") {
        summary[..pos].trim().to_string()
    } else {
        summary.to_string()
    }
}

/// Synthesizes the previous session summary and the newest conversation summary 
/// into a single, high-density, cohesive summary under the SESSION_SUMMARY_MAX_CHARS limit.
pub(super) async fn merge_session_summaries(
    state: AppState,
    agent_id: String,
    old_summary: String,
    new_summary: String,
) -> Result<String, String> {
    let prompt = format!(
        "You are an expert context consolidation agent. Your task is to merge an older session summary with a newly generated summary of the most recent conversation turns into a single, high-density, cohesive summary.\n\n\
         CRITICAL CONSTRAINTS:\n\
         1. The combined summary must preserve all critical decisions, key file changes, error traces, and architectural goals.\n\
         2. It must be written in a high-density, professional, and concise format.\n\
         3. The final output must be strictly less than 6,500 characters so that it safely fits within CADE's active memory block buffers.\n\
         4. Do not include any intro, outro, preamble, or markdown code block wrappers (like ```markdown). Respond ONLY with the raw merged summary text.\n\n\
         OLDER SESSION SUMMARY:\n{old_summary}\n\n\
         NEW CONVERSATION SUMMARY:\n{new_summary}"
    );

    let model = sqlite::get_agent(&state.db, &agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Agent not found".to_string())?
        .model;

    // Resolve the cheapest capable model for compaction (GPT-4o-mini, Gemini-2.5-flash, etc.)
    let compaction_model = default_compaction_model(&model);

    let req = CompletionRequest {
        model: compaction_model,
        messages: vec![LlmMessage {
            role: "user".to_string(),
            content: prompt,
            tool_call_id: None,
            tool_calls: None,
            images: None,
        }],
        tools: vec![],
        max_tokens: 1500,
        reasoning_effort: None,
    };

    match state.llm.complete(&req).await {
        Ok(resp) => {
            if let Some(content) = resp.content {
                Ok(content.trim().to_string())
            } else {
                Err("Empty response from consolidation model".to_string())
            }
        }
        Err(e) => Err(format!("LLM completion failed: {e}")),
    }
}

/// Sanitize a line for inclusion in `session_index`: strip newlines,
/// collapse internal whitespace, cap at 200 chars.
fn sanitize_index_line(s: &str) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
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

        let cleaned =
            word.trim_matches(|c: char| c == ',' || c == ';' || c == '`' || c == '\'' || c == '"');

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
            || (cleaned.starts_with("E0")
                && cleaned.len() <= 6
                && cleaned[2..].chars().all(|c| c.is_ascii_digit()))
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
            && cleaned
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
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
        if (trimmed.starts_with("error:")
            || trimmed.starts_with("Error:")
            || trimmed.starts_with("ERROR:"))
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

fn group_turns(messages: &[(String, String)], max_turn_chars: usize) -> Vec<Vec<(String, String)>> {
    let mut turns: Vec<Vec<(String, String)>> = Vec::new();
    let mut current: Vec<(String, String)> = Vec::new();
    let mut current_chars = 0;

    for msg in messages {
        let msg_chars = msg.1.chars().count();
        let is_safe_boundary = msg.0 == "assistant";

        if (msg.0 == "user" && !current.is_empty())
            || (is_safe_boundary && current_chars >= max_turn_chars && !current.is_empty())
        {
            turns.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current.push(msg.clone());
        current_chars += msg_chars;
    }
    if !current.is_empty() {
        turns.push(current);
    }
    turns
}

// ── SleeptimeAgent ────────────────────────────────────────────────────────────

use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Background manager for automated, periodic memory consolidation ("Sleeptime Agent").
/// Monitors agent inactivity and triggers compaction when conversation budgets are crossed,
/// ensuring full isolation and thread safety.
pub struct SleeptimeAgent {
    state: AppState,
    poll_interval: Duration,
    inactivity_threshold_secs: i64,
    concurrency_limit: usize,
    cancellation_token: Option<CancellationToken>,
}

impl SleeptimeAgent {
    /// Create a new `SleeptimeAgent` with default parameters:
    /// - Poll Interval: 30 seconds
    /// - Inactivity Threshold: 20 seconds
    /// - Concurrency Limit: 4 concurrent tasks
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            poll_interval: Duration::from_secs(30),
            inactivity_threshold_secs: 20,
            concurrency_limit: 4,
            cancellation_token: None,
        }
    }

    /// Set a custom poll interval.
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Set a custom inactivity threshold (in seconds).
    pub fn with_inactivity_threshold(mut self, threshold_secs: i64) -> Self {
        self.inactivity_threshold_secs = threshold_secs;
        self
    }

    /// Set a custom maximum limit of parallel consolidation tasks.
    pub fn with_concurrency_limit(mut self, limit: usize) -> Self {
        self.concurrency_limit = limit;
        self
    }

    /// Set a cancellation token for graceful cooperative shutdown.
    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancellation_token = Some(token);
        self
    }

    /// Spawn the background consolidation task loop inside a tokio thread.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        let state_bg = self.state.clone();
        let poll_interval = self.poll_interval;
        let threshold_secs = self.inactivity_threshold_secs;
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(self.concurrency_limit));
        let cancel = self.cancellation_token.clone();

        tokio::spawn(async move {
            loop {
                if let Some(ref c) = cancel {
                    tokio::select! {
                        _ = c.cancelled() => {
                            tracing::info!("SleeptimeAgent background task cancelled cleanly");
                            break;
                        }
                        _ = tokio::time::sleep(poll_interval) => {}
                    }
                } else {
                    tokio::time::sleep(poll_interval).await;
                }

                if let Some(ref c) = cancel
                    && c.is_cancelled() {
                        break;
                    }

                let mut pending: Vec<(String, Option<String>)> = Vec::new();
                {
                    let mut activity = state_bg.agent_activity.write().await;
                    let now = chrono::Utc::now().timestamp();
                    for (agent_id, act) in activity.iter_mut() {
                        if act.needs_consolidation && (now - act.last_active_ts) > threshold_secs {
                            act.needs_consolidation = false;
                            pending.push((agent_id.clone(), act.conversation_id.clone()));
                        }
                    }
                }

                for (agent_id, conv_id) in pending {
                    tracing::info!(
                        "Sleeptime consolidation triggered for agent {} (conv={:?})",
                        agent_id,
                        conv_id
                    );
                    let state_c = state_bg.clone();
                    let sem_c = sem.clone();
                    tokio::spawn(async move {
                        let _permit = sem_c.acquire().await;
                        consolidate_agent(
                            &state_c,
                            &agent_id,
                            conv_id.as_deref(),
                            None,
                        )
                        .await;
                    });
                }
            }
        })
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "consolidation_tests.rs"]
mod tests;
