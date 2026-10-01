use crate::server::state::AppState;
use cade_ai::{LlmMessage, PromptBudgetManager};
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
    /// A split tool exchange still belongs to its original user request. Carry
    /// a bounded textual anchor, not the entire preceding user/tool chain (or
    /// repeated image attachments), when that original prefix is evicted.
    pub(crate) fn user_anchor(message: &LlmMessage, max_chars: usize) -> LlmMessage {
        let mut anchor = message.clone();
        anchor.images = None;
        anchor.cache_control = None;
        let limit = max_chars.clamp(64, 1024);
        let len = anchor.content.chars().count();
        if len > limit {
            const MARKER: &str = "\n[earlier user request truncated]\n";
            let retained = limit.saturating_sub(MARKER.chars().count());
            let head = retained / 2;
            let tail = retained - head;
            anchor.content = anchor.content.chars().take(head).collect::<String>()
                + MARKER
                + &anchor.content.chars().skip(len - tail).collect::<String>();
        }
        anchor
    }

    /// Collapse verbose tool outputs older than `preserve_recent_turns` into compact metadata tombstones.
    pub fn compact_stale_tool_outputs(
        messages: &mut [LlmMessage],
        preserve_recent_turns: usize,
        threshold_chars: usize,
    ) {
        let total_turns = messages.iter().filter(|m| m.role == "user").count();

        let mut current_turn = 0usize;
        for msg in messages.iter_mut() {
            if msg.role == "user" {
                current_turn += 1;
            }

            let is_stale = current_turn + preserve_recent_turns <= total_turns;
            if is_stale && msg.role == "tool" {
                let char_count = msg.content.chars().count();
                if char_count > threshold_chars {
                    let line_count = msg.content.lines().count();
                    let tool_id = msg.tool_call_id.as_deref().unwrap_or("unknown");
                    msg.content = format!(
                        "[tool_output: {} lines ({} chars) from call {} omitted from history; information incorporated in earlier turns]",
                        line_count, char_count, tool_id
                    );
                }
            }
        }
    }

    /// Canonical grouping for inline context assembly and its legacy entry point.
    pub(crate) fn group_into_turns(
        messages: &[LlmMessage],
        max_turn_chars: usize,
    ) -> Vec<Vec<LlmMessage>> {
        let mut turns: Vec<Vec<LlmMessage>> = Vec::new();
        let mut current: Vec<LlmMessage> = Vec::new();
        let mut current_chars = 0usize;

        for msg in messages {
            let msg_chars =
                PromptBudgetManager::new().turn_cost_fallback_chars(std::slice::from_ref(msg));

            let is_safe_boundary = msg.role == "assistant";

            if (msg.role == "user" && !current.is_empty())
                || (is_safe_boundary && current_chars >= max_turn_chars && !current.is_empty())
            {
                turns.push(std::mem::take(&mut current));
                current_chars = 0;
            }

            current.push(msg.clone());
            current_chars = current_chars.saturating_add(msg_chars);
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
        let mut turns = Self::group_into_turns(history, max_turn_chars);

        // Ensure we never split tool_call/tool_result pairs at the oldest boundary.
        if let Some(first_msg) = turns.first().and_then(|t| t.first())
            && first_msg.role != "user"
            && first_msg.role != "assistant"
        {
            turns.remove(0);
        }

        // Each independently selectable segment must include its user anchor.
        // Charge that anchor before selection; deduplicate it after selection
        // when contiguous retained segments belong to the same user request.
        // This is conservative budgeting, without turning a long tool chain
        // into one unbounded, always-retained turn.
        let anchor_limit = (message_budget_chars / 4).min(max_turn_chars);
        let mut source_user = None;
        let turns: Vec<_> = turns
            .into_iter()
            .enumerate()
            .map(|(index, mut turn)| {
                if let Some(user) = turn.first().filter(|message| message.role == "user") {
                    source_user = Some((index, Self::user_anchor(user, anchor_limit)));
                } else if let Some((_, anchor)) = &source_user {
                    turn.insert(0, anchor.clone());
                }
                (source_user.as_ref().map(|(index, _)| *index), turn)
            })
            .collect();

        let budget_manager = PromptBudgetManager::new();
        let mut selected: Vec<(Option<usize>, Vec<LlmMessage>)> = Vec::new();
        let mut budget_used: usize = 0;
        let mut omitted_turns: usize = 0;

        for (source_user, mut turn) in turns.into_iter().rev() {
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
                selected.push((source_user, turn));
                budget_used = budget_used.saturating_add(turn_chars);
            } else if budget_used.saturating_add(turn_chars) <= message_budget_chars {
                selected.push((source_user, turn));
                budget_used = budget_used.saturating_add(turn_chars);
            } else {
                // Attempt Tool Result Truncation before dropping the turn
                let deficit = budget_used
                    .saturating_add(turn_chars)
                    .saturating_sub(message_budget_chars);
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
                            let mut new_content: String =
                                m.content.chars().take(keep_head).collect();
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
                        let tokens = budget_manager.turn_cost(model, &turn);
                        turn_chars = if tokens == 0 {
                            budget_manager.turn_cost_fallback_chars(&turn)
                        } else {
                            budget_manager.chars_for_tokens(tokens)
                        };
                        if budget_used.saturating_add(turn_chars) <= message_budget_chars {
                            selected.push((source_user, turn));
                            budget_used = budget_used.saturating_add(turn_chars);
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
            if let Some((_, dropped)) = selected.pop() {
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
        let mut selected_messages = Vec::new();
        let mut previous_user = None;
        for (source_user, turn) in selected {
            let duplicate_anchor = source_user.is_some() && source_user == previous_user;
            selected_messages.extend(turn.into_iter().skip(usize::from(duplicate_anchor)));
            previous_user = source_user;
        }

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
        sqlite::compact_old_tool_outputs(
            db_pool,
            agent_id,
            conversation_id,
            protect_chars,
            min_chars,
        )
        .map_err(|e| e.to_string())
    }

    async fn consolidate_background(
        &self,
        state: AppState,
        agent_id: String,
        conversation_id: Option<String>,
        override_history_budget: Option<usize>,
    ) -> Option<usize> {
        use crate::server::consolidation::{
            ConsolidationContext, DefaultMemoryConsolidationEngine, MemoryConsolidationEngine,
        };
        let engine = DefaultMemoryConsolidationEngine;
        let cx = ConsolidationContext::new()
            .with_conversation_id(conversation_id)
            .with_history_budget(override_history_budget);

        match engine.consolidate(&state, &agent_id, &cx).await {
            Ok(report) => Some(report.summary_length_chars),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: &str) -> LlmMessage {
        LlmMessage {
            role: role.into(),
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }
    }

    #[test]
    fn inline_compaction_counts_opaque_continuation_and_bounds_long_tool_chains() {
        let signature = "opaque-token-".repeat(100);
        let mut history = vec![message("user", "Keep inspecting until finished")];
        for index in 0..40 {
            let id = format!("call-{index}");
            let mut assistant = message("assistant", "");
            assistant.tool_calls = Some(vec![cade_ai::LlmToolCall {
                id: id.clone(),
                name: "inspect".into(),
                arguments: serde_json::json!({"path":"src"}),
                thought_signature: Some(signature.clone()),
            }]);
            history.push(assistant);
            let mut result = message("tool", "inspected");
            result.tool_call_id = Some(id);
            history.push(result);
        }
        let compacted = DefaultContextCompactor.compact_inline("openai/gpt-5", &history, 1500, 512);
        assert!(compacted.omitted_turns > 0);
        assert_eq!(
            compacted.selected_messages.len(),
            3,
            "Keep a bounded user anchor plus the latest atomic exchange, not all forty calls"
        );
        assert_eq!(compacted.selected_messages[0].role, "user");
        let call = &compacted.selected_messages[1].tool_calls.as_ref().unwrap()[0];
        assert_eq!(call.id, "call-39");
        assert_eq!(call.thought_signature.as_deref(), Some(signature.as_str()));
        assert_eq!(
            compacted.selected_messages[2].tool_call_id.as_deref(),
            Some("call-39")
        );
        assert!(
            PromptBudgetManager::new().turn_cost("openai/gpt-5", &compacted.selected_messages) * 3
                <= 1500
        );
    }

    #[test]
    fn inline_compaction_does_not_repeat_anchors_or_mix_distinct_user_requests() {
        let mut history = Vec::new();
        for task in ["first task", "second task"] {
            history.push(message("user", task));
            for _ in 0..3 {
                history.push(message("assistant", "working"));
            }
        }
        let compacted =
            DefaultContextCompactor.compact_inline("openai/gpt-6-sol", &history, 10_000, 1);
        let users: Vec<_> = compacted
            .selected_messages
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(users, vec!["first task", "second task"]);
        assert_eq!(compacted.selected_messages.len(), history.len());
    }

    #[test]
    fn test_compact_stale_tool_outputs_preserves_recent_turns() {
        let mut messages = vec![
            // Turn 1 (Old)
            LlmMessage {
                role: "user".to_string(),
                content: "Read file 1".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            LlmMessage {
                role: "tool".to_string(),
                content: "a".repeat(1000),
                tool_call_id: Some("call_1".to_string()),
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            LlmMessage {
                role: "assistant".to_string(),
                content: "I read file 1".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            // Turn 2 (Recent)
            LlmMessage {
                role: "user".to_string(),
                content: "Read file 2".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            LlmMessage {
                role: "tool".to_string(),
                content: "b".repeat(1000),
                tool_call_id: Some("call_2".to_string()),
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            // Turn 3 (Most recent)
            LlmMessage {
                role: "user".to_string(),
                content: "Next step".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
        ];

        DefaultContextCompactor::compact_stale_tool_outputs(&mut messages, 2, 200);

        assert!(messages[1].content.contains("[tool_output:"));
        assert!(messages[1].content.contains("call_1"));
        assert_eq!(messages[4].content, "b".repeat(1000));
    }
}
