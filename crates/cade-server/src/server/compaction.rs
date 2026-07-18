use cade_ai::{LlmMessage, PromptBudgetManager};
use crate::server::state::AppState;
use cade_store::sqlite;

/// Result of synchronous inline compaction of conversation history.
#[derive(Debug, Clone)]
pub struct InlineCompactionResult {
    /// The flattened, budget-compliant list of selected historical messages.
    pub selected_messages: Vec<LlmMessage>,
    /// Number of older turns that had to be omitted.
    pub omitted_turns: usize,
    /// Whether the usage exceeds the proactive threshold (requiring background consolidation).
    pub needs_proactive_consolidation: bool,
}

/// Unified, deep interface for managing all context compaction and consolidation lifecycle stages.
#[async_trait::async_trait]
pub trait ContextCompactionEngine: Send + Sync {
    /// Stage 1: Synchronous inline compaction of the conversation history.
    /// Returns a budget-compliant message list and proactive consolidation signals.
    fn compact_inline(
        &self,
        model: &str,
        history: &[LlmMessage],
        message_budget_chars: usize,
        max_turn_chars: usize,
    ) -> InlineCompactionResult;

    /// Stage 2: Database footprint compaction. Purges old tool outputs from SQLite.
    fn compact_db_tool_outputs(
        &self,
        db_pool: &sqlite::Db,
        agent_id: &str,
        conversation_id: Option<&str>,
        protect_chars: usize,
        min_chars: usize,
    ) -> Result<usize, String>;

    /// Stage 3: Asynchronous background consolidation. Generates LLM summaries of older history.
    async fn consolidate_background(
        &self,
        state: AppState,
        agent_id: String,
        conversation_id: Option<String>,
        override_history_budget: Option<usize>,
    ) -> Option<usize>;
}

// ── Default Context Compactor Implementation ─────────────────────────────────

pub struct DefaultContextCompactor;

impl DefaultContextCompactor {
    /// Helper to group messages into turns (identical to the legacy group_into_turns).
    fn group_into_turns(&self, messages: &[LlmMessage], max_turn_chars: usize) -> Vec<Vec<LlmMessage>> {
        let mut turns: Vec<Vec<LlmMessage>> = Vec::new();
        let mut current: Vec<LlmMessage> = Vec::new();
        let mut current_chars = 0;

        for msg in messages {
            let msg_chars = msg.content.chars().count()
                + msg
                    .tool_calls
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|tc| tc.arguments.to_string().len())
                    .sum::<usize>();

            let is_safe_boundary = msg.role == "assistant";

            if (msg.role == "user" && !current.is_empty())
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
}

#[async_trait::async_trait]
impl ContextCompactionEngine for DefaultContextCompactor {
    fn compact_inline(
        &self,
        model: &str,
        history: &[LlmMessage],
        message_budget_chars: usize,
        max_turn_chars: usize,
    ) -> InlineCompactionResult {
        let mut turns = self.group_into_turns(history, max_turn_chars);

        // Ensure we never split tool_call/tool_result pairs at the oldest boundary.
        if let Some(first_msg) = turns.first().and_then(|t| t.first())
            && first_msg.role != "user"
            && first_msg.role != "assistant"
        {
            turns.remove(0);
        }

        let budget_manager = PromptBudgetManager::new();
        let mut selected: Vec<Vec<LlmMessage>> = Vec::new();
        let mut budget_used: usize = 0;
        let mut omitted_turns: usize = 0;

        for mut turn in turns.into_iter().rev() {
            let turn_cost_toks = budget_manager.turn_cost(model, &turn);
            let fallback_chars = budget_manager.turn_cost_fallback_chars(&turn);
            let raw_chars = if turn_cost_toks == 0 && fallback_chars > 0 {
                fallback_chars
            } else {
                budget_manager.chars_for_tokens(turn_cost_toks)
            };

            let mut turn_chars = raw_chars;

            if selected.is_empty() {
                // Always include the most-recent turn regardless of size.
                selected.push(turn);
                budget_used += turn_chars;
            } else if budget_used + turn_chars <= message_budget_chars {
                selected.push(turn);
                budget_used += turn_chars;
            } else {
                // Attempt Tool Result Truncation before dropping the turn
                let deficit = (budget_used + turn_chars).saturating_sub(message_budget_chars);
                let tool_results_chars: usize = turn
                    .iter()
                    .filter(|m| m.role == "tool")
                    .map(|m| m.content.chars().count())
                    .sum();

                let margin = 200;
                if tool_results_chars > deficit + margin {
                    let to_cut = deficit + margin;
                    let mut cut_remaining = to_cut;

                    for m in turn.iter_mut().filter(|m| m.role == "tool") {
                        let len = m.content.chars().count();
                        if len > margin && cut_remaining > 0 {
                            let cut_here = cut_remaining.min(len.saturating_sub(margin));
                            let keep = len - cut_here;
                            let keep_head = (keep as f64 * 0.2) as usize;
                            let keep_tail = keep.saturating_sub(keep_head);
                            let mut new_content: String = m.content.chars().take(keep_head).collect();
                            new_content.push_str(&format!(
                                "\n... [{} chars truncated to fit context window] ...\n",
                                cut_here
                            ));
                            let tail: String = m
                                .content
                                .chars()
                                .skip(keep_head + cut_here)
                                .take(keep_tail)
                                .collect();
                            new_content.push_str(&tail);
                            m.content = new_content;
                            cut_remaining -= cut_here;
                        }
                        if cut_remaining == 0 {
                            break;
                        }
                    }

                    if cut_remaining == 0 {
                        turn_chars -= to_cut;
                        if budget_used + turn_chars <= message_budget_chars {
                            selected.push(turn);
                            budget_used += turn_chars;
                            continue;
                        }
                    }
                }

                omitted_turns += 1;
            }
        }

        // Pre-flight overflow guard: drop oldest selected turns if they still overflow
        let mut preflight_dropped = 0usize;
        while selected.len() > 1 && budget_used > message_budget_chars {
            if let Some(dropped) = selected.pop() {
                let turn_cost_toks = budget_manager.turn_cost(model, &dropped);
                let fallback_chars = budget_manager.turn_cost_fallback_chars(&dropped);
                let chars = if turn_cost_toks == 0 && fallback_chars > 0 {
                    fallback_chars
                } else {
                    budget_manager.chars_for_tokens(turn_cost_toks)
                };
                budget_used = budget_used.saturating_sub(chars);
                preflight_dropped += 1;
            }
        }
        if preflight_dropped > 0 {
            omitted_turns += preflight_dropped;
        }

        // Reverse back to oldest-first and flatten
        selected.reverse();
        let selected_messages: Vec<LlmMessage> = selected.into_iter().flatten().collect();

        InlineCompactionResult {
            selected_messages,
            omitted_turns,
            needs_proactive_consolidation: omitted_turns > 0,
        }
    }

    fn compact_db_tool_outputs(
        &self,
        db_pool: &sqlite::Db,
        agent_id: &str,
        conversation_id: Option<&str>,
        protect_chars: usize,
        min_chars: usize,
    ) -> Result<usize, String> {
        sqlite::compact_old_tool_outputs(db_pool, agent_id, conversation_id, protect_chars, min_chars)
            .map_err(|e| e.to_string())
    }

    async fn consolidate_background(
        &self,
        state: AppState,
        agent_id: String,
        conversation_id: Option<String>,
        override_history_budget: Option<usize>,
    ) -> Option<usize> {
        crate::server::consolidation::consolidate_agent(
            state,
            agent_id,
            conversation_id,
            override_history_budget,
        )
        .await
    }
}
