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

pub mod accumulator;
pub mod knowledge_lifting;

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
#[allow(dead_code)]
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
/// A stateful context compaction engine that unifies context consolidation,
/// prompt budgeting, LLM summary generation, and SQLite transactions behind
/// a high-leverage interface.
pub struct ContextCompactionEngine<'a> {
    state: &'a AppState,
    agent_id: String,
    conversation_id: Option<String>,
}

impl<'a> ContextCompactionEngine<'a> {
    pub fn new(state: &'a AppState, agent_id: &str, conversation_id: Option<&str>) -> Self {
        Self {
            state,
            agent_id: agent_id.to_string(),
            conversation_id: conversation_id.map(String::from),
        }
    }

    /// Compact/consolidate the context window by summarizing older dropped turns,
    /// writing the result to `session_summary`, caching raw dialogue to archival memory,
    /// extracting durable facts, and inserting a compaction marker.
    ///
    /// Returns the number of characters in the newly updated summary block.
    pub async fn compact_context(&self, override_history_budget: Option<usize>) -> Option<usize> {
        let state = self.state;
        let agent_id = &self.agent_id;
        let conversation_id = self.conversation_id.as_deref();

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
                let artifacts = extract_artifacts(trimmed);
                let artifact_prefix = if artifacts.is_empty() {
                    String::new()
                } else {
                    format!(" | artifacts: {}", artifacts.join(", "))
                };
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
                    if payload.chars().count() + entry.chars().count() > MAX_ARCHIVAL_PAYLOAD_CHARS
                    {
                        payload
                            .push_str("\n[…remaining dropped turns truncated for archival cap…]");
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
        let existing_blocks = sqlite::get_memory_blocks(&state.db, agent_id).unwrap_or_default();
        let existing = existing_blocks
            .iter()
            .find(|(label, _, _)| label == "session_summary")
            .map(|(_, val, _)| val.as_str())
            .unwrap_or("");

        let (new_read, new_mod) = extract_touched_files(&all_rows);
        let touched_files = accumulator::TouchedFiles {
            read: new_read,
            modified: new_mod,
        };

        let compaction_model = agent
            .compaction_model
            .as_deref()
            .filter(|m| !m.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| default_compaction_model(&agent.model));

        // Construct functional core accumulator
        let acc = accumulator::SummaryAccumulator::new(state.llm.clone(), compaction_model.clone());

        // Convert existing blocks to (label, value) pairs
        let existing_blocks_pairs: Vec<(String, String)> = existing_blocks
            .iter()
            .map(|(label, val, _)| (label.clone(), val.clone()))
            .collect();

        // Run in-memory accumulation
        let acc_result = acc
            .accumulate(
                existing,
                &final_summary,
                touched_files,
                &existing_blocks_pairs,
            )
            .await;

        let new_value = match acc_result {
            accumulator::AccumulationResult::Merged(val) => {
                // Simply upsert
                if let Err(e) = sqlite::upsert_memory_block(
                    &state.db,
                    agent_id,
                    "session_summary",
                    &val,
                    Some(
                        "Auto-generated summary of older conversation turns (Sleeptime consolidation)",
                    ),
                    Some(SESSION_SUMMARY_MAX_CHARS),
                ) {
                    tracing::warn!(
                        "consolidate [{}]: failed to write session_summary: {}",
                        agent_id,
                        e
                    );
                    return None;
                }
                val
            }
            accumulator::AccumulationResult::Rotated(plan) => {
                // Execute rotation plan in safe transactional order
                if let Some(ref archive) = plan.archive_content {
                    let tags = vec!["evicted-session-summary".to_string()];
                    let _ = sqlite::insert_archival_memory(&state.db, agent_id, archive, &tags);
                }

                if let Some(ref index_line) = plan.append_to_index {
                    append_to_session_index_db(&state.db, agent_id, index_line);
                }

                // Delete old blocks
                for del_label in plan.deletes {
                    let _ = sqlite::delete_memory_block(&state.db, agent_id, &del_label);
                }

                // Upsert new blocks
                for (up_label, up_val) in &plan.upserts {
                    let cap_limit = if up_label == "session_summary" {
                        SESSION_SUMMARY_MAX_CHARS
                    } else {
                        SESSION_SUMMARY_ARCHIVED_MAX_CHARS
                    };
                    if let Err(e) = sqlite::upsert_memory_block(
                        &state.db,
                        agent_id,
                        up_label,
                        up_val,
                        Some("Rotated session summary (Phase C ring)"),
                        Some(cap_limit),
                    ) {
                        tracing::warn!(
                            "consolidate [{}]: failed to upsert {}: {}",
                            agent_id,
                            up_label,
                            e
                        );
                    }
                    if up_label != "session_summary" {
                        let _ =
                            sqlite::set_memory_tier(&state.db, agent_id, up_label, "long", false);
                    }
                }

                // Extract new live value
                plan.upserts
                    .iter()
                    .find(|(l, _)| l == "session_summary")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            }
        };

        let _ = sqlite::set_memory_tier(&state.db, agent_id, "session_summary", "pinned", true);

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

        Box::pin(auto_extract_facts(
            state,
            agent_id,
            &summary,
            &compaction_model,
        ))
        .await;

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

        let dropped_msg_count: usize = turns[..dropped].iter().map(|t| t.len()).sum();

        let boundary_msg_id = if dropped_msg_count > 0 && dropped_msg_count <= all_rows.len() {
            Some(all_rows[dropped_msg_count - 1].id.clone())
        } else {
            None
        };

        if let Some(ref bid) = boundary_msg_id {
            if let Err(e) =
                sqlite::TimelineHorizon::advance(&state.db, agent_id, conversation_id, bid, dropped)
            {
                tracing::warn!(
                    "consolidate [{}]: failed to advance timeline horizon: {}",
                    agent_id,
                    e
                );
                return None;
            }
            tracing::debug!(
                "consolidate [{}]: successfully advanced timeline horizon to boundary_msg_id='{}'",
                agent_id,
                bid,
            );
        }

        let metrics = state.agent_metrics.clone();
        let m = metrics.entry(agent_id.to_string()).or_default();
        m.consolidation_runs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        m.chars_summarised
            .fetch_add(dropped_chars, std::sync::atomic::Ordering::Relaxed);
        m.chars_produced
            .fetch_add(summary_chars, std::sync::atomic::Ordering::Relaxed);

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
}

/// Summarise older conversation turns that are no longer in the active context
/// window and write the result to the agent's `session_summary` memory block.
///
/// This is safe to call concurrently for different agents; all DB access is
/// through the existing `Arc<parking_lot::Mutex<Connection>>` pool.
pub async fn consolidate_agent(
    state: AppState,
    agent_id: String,
    conversation_id: Option<String>,
    override_history_budget: Option<usize>,
) -> Option<usize> {
    let engine = ContextCompactionEngine::new(&state, &agent_id, conversation_id.as_deref());
    engine.compact_context(override_history_budget).await
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

    let engine = knowledge_lifting::KnowledgeLiftingEngine::new(
        state.llm.clone(),
        compaction_model.to_string(),
    );
    let facts = match engine.extract_from_text(summary).await {
        Ok(f) => f,
        Err(e) => {
            tracing::debug!(
                "consolidate [{}]: Phase B auto_extract_facts failed: {}",
                agent_id,
                e
            );
            return;
        }
    };

    let mut count = 0;
    for fact in facts {
        if let Err(e) = sqlite::upsert_memory_block_typed(
            &state.db,
            agent_id,
            &fact.label,
            &fact.value,
            Some("Auto-extracted by Phase B Consolidation"),
            Some(1000),
            Some(&fact.memory_type),
            Some(fact.confidence),
        ) {
            tracing::debug!(
                "consolidate [{}]: auto-extraction failed to save block {}: {}",
                agent_id,
                fact.label,
                e
            );
        } else {
            // A2 Provenance: we attribute it to the consolidation turn
            let turn = cade_store::sqlite::get_turn_counter(&state.db, agent_id).unwrap_or(0);
            cade_store::sqlite::memory::stamp_provenance(
                &state.db,
                agent_id,
                &fact.label,
                Some(turn),
                None,
                Some("auto_extraction"),
                None,
            );
            // Chunk it for semantic search
            cade_store::sqlite::memory::rechunk_block(
                &state.db,
                agent_id,
                &fact.label,
                &fact.value,
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
                        && let Some(patch_str) = args_obj.get("patch").and_then(|v| v.as_str())
                    {
                        for line in patch_str.lines() {
                            if let Some(stripped) = line.strip_prefix("+++ ") {
                                let path_part = stripped.trim();
                                let clean_path =
                                    if let Some(stripped_b) = path_part.strip_prefix("b/") {
                                        stripped_b.to_string()
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

/// Sanitize a line for inclusion in `session_index`: strip newlines,
/// collapse internal whitespace, cap at 200 chars.
fn sanitize_index_line(s: &str) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(200).collect()
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
                    && c.is_cancelled()
                {
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
                        consolidate_agent(state_c, agent_id, conv_id, None).await;
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
