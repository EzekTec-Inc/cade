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
                    messages[idx].content = serde_json::Value::String(format!("{existing}{delta}"));
                }
            }
            "thought" | "reasoning_message" => {
                let r_text = event.reasoning().or_else(|| event.content()).unwrap_or("");
                if !r_text.is_empty() {
                    reasoning_acc.push_str(r_text);
                    let reasoning_block = format!("<reasoning>\n{reasoning_acc}\n</reasoning>");
                    if let Some(idx) = messages.iter().position(|m| m.id == stream_id) {
                        let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                        let updated = if existing.is_empty() || existing == reasoning_block {
                            reasoning_block.clone()
                        } else if let Some(tail) = existing.split("</reasoning>").nth(1) {
                            format!("{reasoning_block}{tail}")
                        } else {
                            format!("{reasoning_block}\n{existing}")
                        };
                        messages[idx].content = serde_json::Value::String(updated);
                    }
                }
            }
            "tool_call_message" | "tool_executing" => {
                let name = event.tool_name().or_else(|| event.data.get("name").and_then(|v| v.as_str())).unwrap_or("tool");
                let args = event.tool_args().or_else(|| event.data.get("arguments").and_then(|v| v.as_str())).unwrap_or("");
                if let Some(idx) = messages.iter().position(|m| m.id == stream_id) {
                    let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                    let tool_block = format!("\n\n[Tool Executing: {name}]\nArguments: {args}\n");
                    messages[idx].content = serde_json::Value::String(format!("{existing}{tool_block}"));
                }
            }
            "tool_result_message" | "tool_completed" => {
                let name = event.tool_name().unwrap_or("tool");
                let is_error = event.data.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                let output = event.data.get("output").and_then(|v| v.as_str()).unwrap_or("");
                let status_label = if is_error { "Failed" } else { "Completed" };
                let ui_meta = if let Some(uri) = event.data.get("ui_resource_uri").and_then(|v| v.as_str()) {
                    format!("\n[UI Widget Resource: {uri}]\n")
                } else {
                    String::new()
                };
                if let Some(idx) = messages.iter().position(|m| m.id == stream_id) {
                    let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                    let result_block = format!("\n[Tool {status_label}: {name}]{ui_meta}\nOutput: {output}\n");
                    messages[idx].content = serde_json::Value::String(format!("{existing}{result_block}"));
                }
            }
            "approval_required" => {
                let tool_name = event.tool_name().unwrap_or("tool");
                let approval_id = event.approval_id().unwrap_or("pending");
                let args = event.tool_args().unwrap_or("");
                if let Some(idx) = messages.iter().position(|m| m.id == stream_id) {
                    let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                    let approval_card = format!(
                        "\n\n[Approval Required: {tool_name}] (ID: {approval_id})\nRequires human review before execution.\nArguments: {args}\n"
                    );
                    messages[idx].content = serde_json::Value::String(format!("{existing}{approval_card}"));
                }
            }
            "approval_resolved" => {
                let approval_id = event.approval_id().unwrap_or("unknown");
                let approved = event.data.get("approved").and_then(|v| v.as_bool()).unwrap_or(true);
                let verdict_str = if approved { "Approved" } else { "Denied" };
                if let Some(idx) = messages.iter().position(|m| m.id == stream_id) {
                    let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                    let resolved_block = format!("\n[Approval Resolved: {approval_id} -> {verdict_str}]\n");
                    messages[idx].content = serde_json::Value::String(format!("{existing}{resolved_block}"));
                }
            }
            "progress" => {
                let percent = event.data.get("percent").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let msg = event.data.get("message").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(idx) = messages.iter().position(|m| m.id == stream_id) {
                    let existing = messages[idx].content.as_str().unwrap_or("").to_string();
                    let progress_block = format!("\n[Progress: {:.0}%] {}\n", percent, msg);
                    messages[idx].content = serde_json::Value::String(format!("{existing}{progress_block}"));
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
                    format!(
                        "{existing}

[Stream Error: {e}]"
                    )
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

        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("<reasoning>")
        );
        assert!(
            messages[0]
                .content
                .as_str()
                .unwrap()
                .contains("Thinking step 1...")
        );
    }

    #[test]
    fn test_apply_stream_event_approval_and_widget_flow() {
        let stream_id = "streaming-approval-123";
        let mut messages = vec![ChatMessage {
            id: stream_id.to_string(),
            role: "assistant".to_string(),
            content: json!("Starting task..."),
            conversation_id: None,
        }];
        let mut reasoning_acc = String::new();

        // 1. Tool Executing
        let exec_event = StreamEvent {
            message_type: "tool_executing".to_string(),
            data: json!({ "name": "delete_file", "arguments": "{\"path\": \"old.txt\"}" }),
        };
        ChatSessionCoordinator::apply_stream_event(&mut messages, stream_id, exec_event, &mut reasoning_acc);
        assert!(messages[0].content.as_str().unwrap().contains("[Tool Executing: delete_file]"));

        // 2. Approval Required
        let appr_event = StreamEvent {
            message_type: "approval_required".to_string(),
            data: json!({ "tool_name": "delete_file", "approval_id": "appr-999", "tool_args": "{\"path\": \"old.txt\"}" }),
        };
        ChatSessionCoordinator::apply_stream_event(&mut messages, stream_id, appr_event, &mut reasoning_acc);
        assert!(messages[0].content.as_str().unwrap().contains("[Approval Required: delete_file]"));
        assert!(messages[0].content.as_str().unwrap().contains("(ID: appr-999)"));

        // 3. Approval Resolved
        let resolved_event = StreamEvent {
            message_type: "approval_resolved".to_string(),
            data: json!({ "approval_id": "appr-999", "approved": true }),
        };
        ChatSessionCoordinator::apply_stream_event(&mut messages, stream_id, resolved_event, &mut reasoning_acc);
        assert!(messages[0].content.as_str().unwrap().contains("[Approval Resolved: appr-999 -> Approved]"));

        // 4. Tool Completed with UI Widget
        let comp_event = StreamEvent {
            message_type: "tool_completed".to_string(),
            data: json!({ "tool_name": "delete_file", "output": "File removed", "is_error": false, "ui_resource_uri": "ui://widgets/status" }),
        };
        ChatSessionCoordinator::apply_stream_event(&mut messages, stream_id, comp_event, &mut reasoning_acc);
        assert!(messages[0].content.as_str().unwrap().contains("[Tool Completed: delete_file]"));
        assert!(messages[0].content.as_str().unwrap().contains("[UI Widget Resource: ui://widgets/status]"));
    }
}

// endregion: --- Tests