//! SSE streaming state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Append a chunk of streamed assistant text.
    ///
    /// If the last message is already an assistant message (the one we're
    /// building), the chunk is appended to its content.  Otherwise a new
    /// assistant message is created.
    pub fn on_stream_chunk(&mut self, text: &str) {
        if let Self::Connected(session) = self {
            if !session.streaming { return; }
            let messages = &mut session.messages;
            let auto_scroll = &mut session.auto_scroll;
            // Append to existing assistant message or create one.
            if let Some(last) = messages.last_mut()
                && last.role == "assistant"
                && last.id.is_empty()
            {
                // Accumulate into the in-progress message.
                if let serde_json::Value::String(ref mut s) = last.content {
                    s.push_str(text);
                }
                return;
            }
            // First chunk — create the assistant message and re-enable scroll.
            *auto_scroll = true;
            messages.push(ChatMessage {
                id: String::new(),
                role: "assistant".to_string(),
                content: serde_json::Value::String(text.to_string()),
                conversation_id: None,
            });
        }
    }

    /// Mark the SSE stream as complete.
    pub fn on_stream_done(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  streaming, ..  } = &mut **session;
            *streaming = false;
        }
    }

    /// Append a chunk of streamed reasoning text.
    ///
    /// Works like `on_stream_chunk` but uses `role = "reasoning"`.
    /// Consecutive reasoning chunks accumulate into the same message.
    pub fn on_stream_reasoning(&mut self, text: &str) {
        if let Self::Connected(session) = self {
            if !session.streaming { return; }
            let messages = &mut session.messages;
            if let Some(last) = messages.last_mut()
                && last.role == "reasoning"
                && last.id.is_empty()
            {
                if let serde_json::Value::String(ref mut s) = last.content {
                    s.push_str(text);
                }
                return;
            }
            messages.push(ChatMessage {
                id: String::new(),
                role: "reasoning".to_string(),
                content: serde_json::Value::String(text.to_string()),
                conversation_id: None,
            });
        }
    }

    /// Record a tool call emitted by the assistant during streaming.
    ///
    /// Each tool call becomes its own message with `role = "tool_call"`
    /// and structured JSON content.
    ///
    /// Special case: when `name == "ask_user_question"` the arguments are
    /// also parsed into an [`crate::api::Question`] and set as the active
    /// question so the inline widget can render.
    pub fn on_stream_tool_call(&mut self, id: &str, name: &str, arguments: &str) {
        if let Self::Connected(session) = self {
            if !session.streaming { return; }
            let messages = &mut session.messages;
            let active_question = &mut session.active_question;
            let question_cursor = &mut session.question_cursor;
            let question_checked = &mut session.question_checked;
            let active_plan = &mut session.active_plan;
            messages.push(ChatMessage {
                id: String::new(),
                role: "tool_call".to_string(),
                content: serde_json::json!({
                    "id": id,
                    "name": name,
                    "arguments": arguments,
                }),
                conversation_id: None,
            });

            // Surface inline question widget for `ask_user_question`.
            if name == "ask_user_question" {
                if let Some(q) = crate::api::parse_ask_question(arguments) {
                    let n = q.options.len();
                    *question_cursor = 0;
                    *question_checked = vec![false; n];
                    *active_question = Some(q);
                }
            }

            // Intercept plan panel tool calls.
            if name == "set_plan" {
                let args_val = serde_json::from_str::<serde_json::Value>(arguments).unwrap_or_default();
                let steps: Vec<String> = args_val["steps"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|s| s.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let title = args_val["title"].as_str().unwrap_or("Tasks").to_string();
                if steps.is_empty() {
                    *active_plan = None;
                } else {
                    *active_plan = Some(PlanState {
                        title,
                        steps: steps
                            .into_iter()
                            .enumerate()
                            .map(|(i, desc)| PlanStep {
                                id: i + 1,
                                description: desc,
                                is_done: false,
                            })
                            .collect(),
                        is_visible: true,
                    });
                }
            }

            if name == "UpdatePlan" {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(arguments) {
                    let step_id = v["step_id"].as_u64().unwrap_or(0) as usize;
                    let done = v["done"].as_bool().unwrap_or(true);
                    if let Some(plan) = active_plan {
                        if let Some(step) = plan.steps.iter_mut().find(|s| s.id == step_id) {
                            step.is_done = done;
                        }
                    }
                }
            }
        }
    }

    /// Record a tool result emitted by the server during the agentic loop.
    ///
    /// Appended as a `role = "tool_result"` message for display in the timeline.
    pub fn on_stream_tool_result(&mut self, id: &str, name: &str, output: &str, is_error: bool) {
        if let Self::Connected(session) = self {
            if !session.streaming { return; }
            let messages = &mut session.messages;
            messages.push(ChatMessage {
                id: String::new(),
                role: "tool_result".to_string(),
                content: serde_json::json!({
                    "id":       id,
                    "name":     name,
                    "output":   output,
                    "is_error": is_error,
                }),
                conversation_id: None,
            });
        }
    }

    /// Whether the session is currently streaming an assistant response.
    pub fn is_streaming(&self) -> bool {
        matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession { 
                streaming: true,
                ..
             }))
    }

    /// Whether the timeline should auto-scroll to the bottom.
    pub fn auto_scroll(&self) -> bool {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  auto_scroll, ..  } = &**session;
            *auto_scroll
        } else {
            true
        }
    }

    /// Disable auto-scroll (user scrolled up manually).
    pub fn disable_auto_scroll(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  auto_scroll, ..  } = &mut **session;
            *auto_scroll = false;
        }
    }

    /// Re-enable auto-scroll (user clicked ↓ button).
    pub fn enable_auto_scroll(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  auto_scroll, ..  } = &mut **session;
            *auto_scroll = true;
        }
    }

    /// Store a conversation_id received from the server (e.g. SSE metadata).
    pub fn on_conversation_id(&mut self, id: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            conversation_id, ..
         } = &mut **session;
            *conversation_id = Some(id.to_string());
        }
    }

    /// The active conversation ID, if any.
    pub fn conversation_id(&self) -> Option<&str> {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            conversation_id, ..
         } = &**session;
            conversation_id.as_deref()
        } else {
            None
        }
    }

    /// Store token usage statistics from a `usage_statistics` SSE event.
    pub fn on_usage(&mut self, input_tokens: u64, output_tokens: u64, model: Option<&str>) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            last_usage,
            total_input_tokens,
            total_output_tokens,
            ..
         } = &mut **session;
            *last_usage = Some((input_tokens, output_tokens, model.map(String::from)));
            *total_input_tokens += input_tokens;
            *total_output_tokens += output_tokens;
        }
    }

    /// Store the finish reason from a `finish_reason` SSE event.
    pub fn on_finish_reason(&mut self, reason: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            last_finish_reason, ..
         } = &mut **session;
            *last_finish_reason = Some(reason.to_string());
        }
    }

    /// Last token usage: `(input_tokens, output_tokens, model)`.
    pub fn last_usage(&self) -> Option<(u64, u64, Option<&str>)> {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {  last_usage, ..  } = &**session;
            last_usage.as_ref().map(|(i, o, m)| (*i, *o, m.as_deref()))
        } else {
            None
        }
    }

    /// Last finish reason (e.g. "stop", "length").
    pub fn last_finish_reason(&self) -> Option<&str> {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { 
            last_finish_reason, ..
         } = &**session;
            last_finish_reason.as_deref()
        } else {
            None
        }
    }
}
