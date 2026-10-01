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

#[path = "summary_accumulator.rs"]
pub mod accumulator;
pub mod knowledge_lifting;

// region:    --- Types

/// Need status indicating whether an agent's history requires compaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsolidationNeed {
    UpToDate,
    Pending {
        dropped_turns: usize,
        estimated_dropped_tokens: usize,
    },
    Busy,
}

/// Standard report produced upon completing a memory consolidation run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq)]
pub struct ConsolidationReport {
    pub agent_id: String,
    pub turns_summarized: usize,
    pub input_tokens_used: usize,
    pub output_tokens_used: usize,
    pub summary_length_chars: usize,
    pub knowledge_nodes_lifted: usize,
    pub ring_rotation_applied: bool,
}

/// Context parameters for a memory consolidation request.
#[derive(Debug, Clone, Default)]
pub struct ConsolidationContext {
    pub conversation_id: Option<String>,
    pub override_history_budget: Option<usize>,
    pub force: bool,
}

impl ConsolidationContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_conversation_id(mut self, conv_id: Option<String>) -> Self {
        self.conversation_id = conv_id;
        self
    }

    pub fn with_history_budget(mut self, budget: Option<usize>) -> Self {
        self.override_history_budget = budget;
        self
    }
}

/// Error kinds returned by the MemoryConsolidationEngine.
#[derive(Debug, Clone, derive_more::Display)]
#[display("{self:?}")]
pub enum ConsolidationError {
    #[display("Agent '{_0}' not found")]
    AgentNotFound(String),
    #[display("Database error: {_0}")]
    Db(String),
    #[display("LLM distillation failed: {_0}")]
    Llm(String),
    #[display("Source preserved in archival {_0}; distillation failed: {_1}")]
    ArchivedOnly(String, String),
    #[display("Consolidation already claimed")]
    Busy,
    #[display("Consolidation skipped: {_0}")]
    Skipped(String),
}

impl std::error::Error for ConsolidationError {}

/// Unified, deep interface for memory consolidation and knowledge distillation (ADR-0020 / PRD #52).
#[async_trait::async_trait]
pub trait MemoryConsolidationEngine: Send + Sync {
    /// Execute memory distillation and knowledge lifting for an agent.
    async fn consolidate(
        &self,
        state: &AppState,
        agent_id: &str,
        cx: &ConsolidationContext,
    ) -> Result<ConsolidationReport, ConsolidationError>;

    /// Check whether an agent requires proactive consolidation.
    async fn check_need(
        &self,
        state: &AppState,
        agent_id: &str,
        cx: &ConsolidationContext,
    ) -> ConsolidationNeed;
}

/// Production implementation of the MemoryConsolidationEngine.
pub struct DefaultMemoryConsolidationEngine;

#[async_trait::async_trait]
impl MemoryConsolidationEngine for DefaultMemoryConsolidationEngine {
    async fn consolidate(
        &self,
        state: &AppState,
        agent_id: &str,
        cx: &ConsolidationContext,
    ) -> Result<ConsolidationReport, ConsolidationError> {
        ContextCompactionEngine::new(state, agent_id, cx.conversation_id.as_deref())
            .compact_report(cx.override_history_budget)
            .await
    }

    async fn check_need(
        &self,
        state: &AppState,
        agent_id: &str,
        cx: &ConsolidationContext,
    ) -> ConsolidationNeed {
        if sqlite::consolidation::is_claimed(&state.db, agent_id, cx.conversation_id.as_deref())
            .unwrap_or(false)
        {
            return ConsolidationNeed::Busy;
        }
        let activities = state.agent_activity.read().await;
        if let Some(act) = activities.get(agent_id) {
            if act.needs_consolidation {
                ConsolidationNeed::Pending {
                    dropped_turns: act.last_omitted_turns,
                    estimated_dropped_tokens: act.last_omitted_turns * 500,
                }
            } else {
                ConsolidationNeed::UpToDate
            }
        } else {
            ConsolidationNeed::UpToDate
        }
    }
}

/// Mock implementation for deterministic in-memory testing without LLM or DB.
pub struct MockMemoryConsolidationEngine {
    pub canned_report: Option<ConsolidationReport>,
    pub canned_need: ConsolidationNeed,
}

impl Default for MockMemoryConsolidationEngine {
    fn default() -> Self {
        Self {
            canned_report: Some(ConsolidationReport {
                agent_id: "mock-agent".to_string(),
                turns_summarized: 5,
                input_tokens_used: 1200,
                output_tokens_used: 350,
                summary_length_chars: 1800,
                knowledge_nodes_lifted: 3,
                ring_rotation_applied: true,
            }),
            canned_need: ConsolidationNeed::UpToDate,
        }
    }
}

#[async_trait::async_trait]
impl MemoryConsolidationEngine for MockMemoryConsolidationEngine {
    async fn consolidate(
        &self,
        _state: &AppState,
        agent_id: &str,
        _cx: &ConsolidationContext,
    ) -> Result<ConsolidationReport, ConsolidationError> {
        if let Some(ref r) = self.canned_report {
            let mut rep = r.clone();
            rep.agent_id = agent_id.to_string();
            Ok(rep)
        } else {
            Err(ConsolidationError::Skipped(
                "Mock configured to skip".to_string(),
            ))
        }
    }

    async fn check_need(
        &self,
        _state: &AppState,
        _agent_id: &str,
        _cx: &ConsolidationContext,
    ) -> ConsolidationNeed {
        self.canned_need.clone()
    }
}

// endregion: --- Types

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

/// Minimum number of DB rows normally required before consolidation is
/// attempted. A short tool-heavy exchange may still bypass this gate when
/// enough dropped source material would otherwise be lost.
const MIN_ROWS_FOR_CONSOLIDATION: usize = 20;

/// Dropped source size that makes consolidation worthwhile regardless of the
/// row count. This is about 4k tokens at the legacy 3:1 character estimate.
const MIN_DROPPED_CHARS_FOR_CONSOLIDATION: usize = 12_000;

/// Maximum chars of formatted history text fed to the summarisation LLM call.
/// P5: doubled from 24k → 48k so more dropped-turn detail survives into the
/// summary. At 3 chars/token this is ~16k input tokens on the compaction model.
const MAX_SUMMARY_INPUT_CHARS: usize = 48_000;

/// Maximum tokens the summarisation LLM is allowed to emit.
/// P5: raised from 900 → 1500 so the summary can preserve more decisions,
/// error details, and reasoning chains.
const SUMMARY_MAX_TOKENS: u32 = 1_500;

#[cfg(test)]
use sqlite::consolidation::{
    ARCHIVED_CAP as SESSION_SUMMARY_ARCHIVED_MAX_CHARS, INDEX_CAP as SESSION_INDEX_MAX_CHARS,
};

/// Maximum tokens for the P7 active_goal auto-update LLM call.
// const ACTIVE_GOAL_UPDATE_MAX_TOKENS: u32 = 400;

/// Fraction of the estimated history budget used as the threshold: turns that
/// fit within `char_budget * HISTORY_BUDGET_FRACTION` are considered "in
/// context"; everything older is considered "dropped" and summarised.
const HISTORY_BUDGET_FRACTION: f64 =
    crate::server::api::messages::PROACTIVE_CONSOLIDATION_THRESHOLD;

/// Characters per token approximation (conservative).
const CHARS_PER_TOKEN: usize = 3;

/// Resolve an implicit compaction model through shared configuration. Explicit
/// agent compaction choices are preserved by the caller; compatibility choices
/// and passthrough policies are editable JSON, not provider/model checks here.
pub(crate) fn default_compaction_model(primary_model: &str) -> String {
    cade_ai::catalogue::background_model_for_main_model(primary_model)
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
/// The store captures a fenced history and memory snapshot before LLM work,
/// then atomically publishes its summary and historical boundary.
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
        match self.compact_report(override_history_budget).await {
            Ok(report) => Some(report.summary_length_chars),
            Err(error) => {
                tracing::debug!(agent_id = %self.agent_id, %error, "consolidation did not publish");
                None
            }
        }
    }

    pub async fn compact_report(
        &self,
        override_history_budget: Option<usize>,
    ) -> Result<ConsolidationReport, ConsolidationError> {
        let result = self.compact_snapshot(override_history_budget).await;
        if result.is_ok() {
            if let Some(activity) = self
                .state
                .agent_activity
                .write()
                .await
                .get_mut(&self.agent_id)
            {
                activity.needs_consolidation = false;
            }
        } else if matches!(
            &result,
            Err(ConsolidationError::Busy
                | ConsolidationError::Db(_)
                | ConsolidationError::Llm(_)
                | ConsolidationError::ArchivedOnly(_, _))
        ) {
            // Background/eager callers clear their signal before dispatching.
            // A fenced/stale/failed attempt must remain eligible for retry.
            if let Some(activity) = self
                .state
                .agent_activity
                .write()
                .await
                .get_mut(&self.agent_id)
                && activity.conversation_id == self.conversation_id
            {
                activity.needs_consolidation = true;
            }
        }
        result
    }

    async fn compact_snapshot(
        &self,
        override_history_budget: Option<usize>,
    ) -> Result<ConsolidationReport, ConsolidationError> {
        let state = self.state;
        let agent_id = &self.agent_id;
        let conversation_id = self.conversation_id.as_deref();

        let agent = match sqlite::get_agent(&state.db, agent_id) {
            Ok(Some(a)) => a,
            Ok(None) => {
                tracing::warn!(agent_id = %agent_id, "consolidate:  agent not found — skipping");
                return Err(ConsolidationError::AgentNotFound(agent_id.clone()));
            }
            Err(e) => {
                tracing::warn!("consolidate [{}]: DB error: {}", agent_id, e);
                return Err(ConsolidationError::Db(e.to_string()));
            }
        };

        // ── 1. Fetch messages since the last compaction marker ───────────────────
        let snapshot = sqlite::consolidation::ConsolidationSnapshot::capture(
            &state.db,
            agent_id,
            conversation_id,
        )
        .map_err(|e| ConsolidationError::Db(e.to_string()))?
        .ok_or(ConsolidationError::Busy)?;
        let all_rows = snapshot.messages();

        // Convert rows to (role, text) pairs for turn grouping.
        let flat: Vec<(String, String)> = all_rows
            .iter()
            .map(|row| {
                let role = row.role.clone();
                let text = row
                    .content
                    .as_str()
                    .or_else(|| {
                        row.content
                            .get("content")
                            .and_then(serde_json::Value::as_str)
                    })
                    .map(String::from)
                    .unwrap_or_else(|| row.content.to_string());
                let text = if let Some(calls) = row
                    .content
                    .get("tool_calls")
                    .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
                {
                    format!("{text}\n[tool_calls] {calls}")
                } else {
                    text
                };
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
        for (i, turn) in turns.iter().rev().enumerate() {
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

            // Always retain the newest turn, matching inline compaction.
            if i == 0 || used + chars <= history_budget {
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
            return Err(ConsolidationError::Skipped(
                "All turns fit in history budget".to_string(),
            ));
        }

        let dropped_chars: usize = turns[..dropped]
            .iter()
            .flatten()
            .map(|(_, text)| text.chars().count())
            .sum();
        if all_rows.len() < MIN_ROWS_FOR_CONSOLIDATION
            && dropped_chars < MIN_DROPPED_CHARS_FOR_CONSOLIDATION
        {
            tracing::debug!(
                "consolidate [{}]: only {} rows and {} dropped chars — skipping",
                agent_id,
                all_rows.len(),
                dropped_chars,
            );
            return Err(ConsolidationError::Skipped(
                "Dropped history below consolidation threshold".to_string(),
            ));
        }

        // ── 3. Format dropped turns into a text block for the LLM ────────────────
        let mut history_text = String::new();
        let mut summarized_turns = 0;
        for turn in &turns[..dropped] {
            let mut formatted_turn = String::new();
            for (role, text) in turn {
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
                formatted_turn.push_str(&format!("[{role}{artifact_prefix}] {preview}\n"));
            }
            if history_text.chars().count() + formatted_turn.chars().count()
                > MAX_SUMMARY_INPUT_CHARS
            {
                break;
            }
            history_text.push_str(&formatted_turn);
            summarized_turns += 1;
        }
        // Advance only over complete turns actually represented in this LLM
        // request. Remaining dropped turns stay visible for a subsequent pass.
        let dropped = summarized_turns;

        if history_text.trim().is_empty() {
            tracing::debug!(agent_id = %agent_id, "consolidate:  dropped turns have no useful text — skipping");
            return Err(ConsolidationError::Skipped(
                "No complete useful turn fits summary input cap".to_string(),
            ));
        }

        // ── 3b. F2: Cache full dropped turns into archival memory ────────────────
        let mut files_touched_block = String::new();
        let archival_id;
        let dropped_msg_count: usize = turns[..dropped].iter().map(|t| t.len()).sum();
        {
            const MAX_ARCHIVAL_PAYLOAD_CHARS: usize = 64_000;
            let mut payload = String::with_capacity(8_192);
            payload.push_str(&format!(
                "Dropped turns from agent {agent_id} (consolidation pass).\n\
                 Source: pre-compaction conversation history.\n\
                 Turn count: {dropped} | Message count: {dropped_msg_count}\n\
                 ---\n\n"
            ));
            let mut truncated = false;
            for row in &all_rows[..dropped_msg_count] {
                // Archive the stored content, including tool-call arguments and
                // IDs, rather than the summarizer's role-capped text previews.
                let entry = format!("[{}] {}\n\n", row.role, row.content);
                if payload.chars().count() + entry.chars().count() > MAX_ARCHIVAL_PAYLOAD_CHARS {
                    payload.push_str("\n[…remaining dropped turns truncated for archival cap…]");
                    truncated = true;
                    break;
                }
                payload.push_str(&entry);
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
            archival_id = snapshot
                .archive_source(payload.trim_end(), &tags)
                .map_err(|e| ConsolidationError::Db(e.to_string()))?;

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
                cache_control: None,
            }],
            tools: vec![],
            max_tokens: SUMMARY_MAX_TOKENS,
            reasoning_effort: None,
        };

        let response = match state.llm.complete(&req).await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("consolidate [{}]: LLM call failed: {}", agent_id, e);
                return Err(ConsolidationError::ArchivedOnly(archival_id, e.to_string()));
            }
        };
        // CompletionResponse does not carry provider usage (only streaming does).
        let input_tokens_used = 0;
        let output_tokens_used = 0;
        let summary = response.content.unwrap_or_default().trim().to_string();

        if summary.is_empty() {
            tracing::debug!(agent_id = %agent_id, "consolidate:  LLM returned empty summary");
            return Err(ConsolidationError::ArchivedOnly(
                archival_id,
                "Empty summary".to_string(),
            ));
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
            return Err(ConsolidationError::ArchivedOnly(
                archival_id,
                "No useful summary".to_string(),
            ));
        }

        // ── 5. Write to the `session_summary` memory block ───────────────────────
        let existing_blocks = snapshot.block_values();
        let existing = existing_blocks
            .iter()
            .find(|(label, _)| label == "session_summary")
            .map(|(_, val)| val.as_str())
            .unwrap_or("");

        let (new_read, new_mod) = extract_touched_files(&all_rows[..dropped_msg_count]);
        let touched_files = accumulator::TouchedFiles {
            read: new_read,
            modified: new_mod,
        };

        // Construct functional core accumulator
        let acc = accumulator::SummaryAccumulator::new(state.llm.clone(), compaction_model.clone());

        // Run in-memory accumulation
        let acc_result = acc
            .accumulate(existing, &final_summary, touched_files, existing_blocks)
            .await;

        let (plan, ring_rotation_applied) = acc_result.into_plan();
        let summary_length_chars = snapshot
            .commit(dropped_msg_count, dropped, &plan)
            .map_err(|e| ConsolidationError::Db(e.to_string()))?;
        // Publication is complete before optional knowledge lifting/export. The
        // claim and all DB connections are released before further LLM awaits.
        drop(snapshot);
        state.invalidate_context_cache(agent_id, conversation_id);
        {
            let mut telemetry_guard = state.agent_context_telemetry.write().await;
            if let Some(telemetry) = telemetry_guard.get_mut(agent_id) {
                telemetry.turns_omitted = telemetry.turns_omitted.saturating_sub(dropped);
                telemetry.consolidation_reason = None;
                telemetry.eager_consolidation_triggered = false;
            }
        }
        crate::server::api::agents::broadcast_global_event(serde_json::json!({
            "event_type": "compaction_completed",
            "agent_id": agent_id,
            "conversation_id": conversation_id,
            "dropped_turns": dropped,
        }));

        tracing::info!(
            "consolidate [{}]: session_summary updated ({} chars; {} dropped turns summarised)",
            agent_id,
            summary_length_chars,
            dropped,
        );

        let knowledge_nodes_lifted = Box::pin(auto_extract_facts(
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

        Ok(ConsolidationReport {
            agent_id: agent_id.clone(),
            turns_summarized: dropped,
            input_tokens_used,
            output_tokens_used,
            summary_length_chars,
            knowledge_nodes_lifted,
            ring_rotation_applied,
        })
    }
}

/// Summarise older conversation turns that are no longer in the active context
/// window and write the result to the agent's `session_summary` memory block.
///
/// This is safe to call concurrently for different agents; all DB access is
/// through a captured SQLite snapshot and fenced per-conversation claim.
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
) -> usize {
    if summary.trim().is_empty() {
        return 0;
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
            return 0;
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
    count
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Append a one-line excerpt to the pinned `session_index` block, evicting
/// oldest lines FIFO when the block exceeds `SESSION_INDEX_MAX_CHARS`.
#[cfg(test)]
fn append_to_session_index_db(db: &cade_store::sqlite::Db, agent_id: &str, excerpt: &str) {
    sqlite::consolidation::append_session_index(db, agent_id, excerpt).unwrap();
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
#[cfg(test)]
use accumulator::sanitize_index_line;

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
