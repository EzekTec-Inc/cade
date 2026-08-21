//! ChatSessionCoordinator module for cade-gui (PRD #65 / Issue #66).
//!
//! Encapsulates optimistic message insertions, SSE stream decoding,
//! reasoning block accumulation, and message ID stabilization behind a clean seam.

use cade_api_types::{ChatMessage, StreamEvent};
use dioxus::prelude::*;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::api::CadeApiClient;

// region:    --- Types

/// Outcome of a dispatched chat turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatTurnOutcome {
    Completed {
        final_message_id: String,
        content_length: usize,
        had_reasoning: bool,
    },
    Cancelled,
    Failed(String),
}

/// Standalone, deep coordinator managing conversation turn lifecycles and SSE streaming.
#[derive(Clone)]
pub struct ChatSessionCoordinator {
    api_client: CadeApiClient,
    agent_id: String,
    conversation_id: Option<String>,
}

impl ChatSessionCoordinator {
    pub fn new(
        api_client: CadeApiClient,
        agent_id: impl Into<String>,
        conversation_id: Option<String>,
    ) -> Self {
        Self {
            api_client,
            agent_id: agent_id.into(),
            conversation_id,
        }
    }

    /// Process an incoming stream event and update message state in-place.
    pub fn apply_stream_event(
        messages: &mut [ChatMessage],
        stream_id: &str,
        event: StreamEvent,
        reasoning_acc: &mut String,
    ) {
        match event.msg_type() {
            "assistant_message" => {
                if let Some(delta) = event.content()
                    && let Some(idx) = messages.iter().position(|m| m.id == stream_id)
                {
                    let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                    messages[idx].content =
                        serde_json::Value::String(format!("{existing}{delta}"));
                }
            }
            "reasoning_message" => {
                if let Some(r) = event.reasoning() {
                    reasoning_acc.push_str(r);
                    let reasoning_block = format!("<reasoning>
{reasoning_acc}
</reasoning>");
                    if let Some(idx) = messages.iter().position(|m| m.id == stream_id) {
                        let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                        let updated = if existing.is_empty() || existing == reasoning_block {
                            reasoning_block.clone()
                        } else if let Some(tail) = existing.split("</reasoning>").nth(1) {
                            format!("{reasoning_block}{tail}")
                        } else {
                            format!("{reasoning_block}
{existing}")
                        };
                        messages[idx].content = serde_json::Value::String(updated);
                    }
                }
            }
            "tool_call_message" => {
                if let Some(tc) = event.tool_call()
                    && let Some(idx) = messages.iter().position(|m| m.id == stream_id)
                {
                        let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                        let tool_block = format!(
                            "

[Tool Call: {}]
Arguments:
{}
",
                            tc.name, tc.arguments
                        );
                        messages[idx].content =
                            serde_json::Value::String(format!("{existing}{tool_block}"));
                }
            }
            "error" => {
                let err_msg = event.error().unwrap_or("Unknown error");
                if let Some(idx) = messages.iter().position(|m| m.id == stream_id) {
                    messages[idx].content = serde_json::Value::String(format!("[Error] {err_msg}"));
                }
            }
            _ => {}
        }
    }

    /// Dispatches a user prompt, managing optimistic state insertions,
    /// SSE event streaming, reasoning accumulator tags, and final ID stabilization.
    pub async fn dispatch_turn(
        &self,
        prompt: &str,
        mut messages_signal: Signal<Vec<ChatMessage>>,
        mut is_loading_signal: Signal<bool>,
        cancel_token: Arc<AtomicBool>,
    ) -> Result<ChatTurnOutcome, String> {
        let text = prompt.trim().to_string();
        if text.is_empty() {
            return Err("Empty prompt".to_string());
        }

        is_loading_signal.set(true);

        #[cfg(target_arch = "wasm32")]
        let timestamp = js_sys::Date::now() as u64;
        #[cfg(not(target_arch = "wasm32"))]
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let stream_id = format!("streaming-{timestamp}");

        // 1. Optimistic insertions
        let mut msgs = messages_signal();
        msgs.push(ChatMessage {
            id: format!("user-{timestamp}"),
            role: "user".to_string(),
            content: serde_json::Value::String(text.clone()),
            conversation_id: self.conversation_id.clone(),
        });
        msgs.push(ChatMessage {
            id: stream_id.clone(),
            role: "assistant".to_string(),
            content: serde_json::Value::String(String::new()),
            conversation_id: self.conversation_id.clone(),
        });
        messages_signal.set(msgs);

        // 2. Stream execution
        let mut reasoning_acc = String::new();
        let stream_id_clone = stream_id.clone();

        let stream_result = self
            .api_client
            .stream_messages(
                &self.agent_id,
                &text,
                self.conversation_id.as_deref(),
                Some(cancel_token.clone()),
                |event: StreamEvent| {
                    let mut current_msgs = messages_signal();
                    Self::apply_stream_event(
                        &mut current_msgs,
                        &stream_id_clone,
                        event,
                        &mut reasoning_acc,
                    );
                    messages_signal.set(current_msgs);
                },
            )
            .await;

        is_loading_signal.set(false);

        // 3. Finalization
        let mut final_msgs = messages_signal();
        let had_reasoning = !reasoning_acc.is_empty();
        let mut final_len = 0;
        let final_id = format!("msg-{timestamp}");

        if let Some(idx) = final_msgs.iter().position(|m| m.id == stream_id) {
            let final_content = match &stream_result {
                Err(e) => {
                    let existing = final_msgs[idx].content.as_str().unwrap_or("").to_string();
                    format!("{existing}

[Stream Error: {e}]")
                }
                Ok(_) => final_msgs[idx].content.as_str().unwrap_or("").to_string(),
            };
            final_len = final_content.len();
            final_msgs[idx].content = serde_json::Value::String(final_content);
            final_msgs[idx].id = final_id.clone();
            messages_signal.set(final_msgs);
        }

        match stream_result {
            Ok(_) => Ok(ChatTurnOutcome::Completed {
                final_message_id: final_id,
                content_length: final_len,
                had_reasoning,
            }),
            Err(e) => {
                if cancel_token.load(std::sync::atomic::Ordering::Acquire) {
                    Ok(ChatTurnOutcome::Cancelled)
                } else {
                    Err(e)
                }
            }
        }
    }
}

// endregion: --- Types

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_apply_stream_event_assistant_delta() {
        let stream_id = "streaming-123";
        let mut messages = vec![ChatMessage {
            id: stream_id.to_string(),
            role: "assistant".to_string(),
            content: json!("Hello"),
            conversation_id: None,
        }];
        let mut reasoning_acc = String::new();

        let event = StreamEvent {
            message_type: "assistant_message".to_string(),
            data: json!({ "content": " world!" }),
        };

        ChatSessionCoordinator::apply_stream_event(
            &mut messages,
            stream_id,
            event,
            &mut reasoning_acc,
        );

        assert_eq!(messages[0].content, json!("Hello world!"));
    }

    #[test]
    fn test_apply_stream_event_reasoning_accumulation() {
        let stream_id = "streaming-123";
        let mut messages = vec![ChatMessage {
            id: stream_id.to_string(),
            role: "assistant".to_string(),
            content: json!("Answer"),
            conversation_id: None,
        }];
        let mut reasoning_acc = String::new();

        let event = StreamEvent {
            message_type: "reasoning_message".to_string(),
            data: json!({ "reasoning": "Thinking step 1..." }),
        };

        ChatSessionCoordinator::apply_stream_event(
            &mut messages,
            stream_id,
            event,
            &mut reasoning_acc,
        );

        assert!(messages[0].content.as_str().unwrap().contains("<reasoning>"));
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("Thinking step 1...")
        );
    }
}

// endregion: --- Tests