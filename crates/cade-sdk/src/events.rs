use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Strongly-typed stream events emitted during agent execution.
///
/// Provides a structured, type-safe representation of SSE telemetry, tool dispatches,
/// thinking deltas, and lifecycle outcomes for SDK consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum CadeStreamEvent {
    /// Incremental reasoning or thinking trace emitted by the model.
    Thought(String),
    /// Incremental assistant text chunk.
    MessageDelta(String),
    /// A tool invocation has started.
    ToolExecuting {
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
    },
    /// A tool execution has completed with output or error.
    ToolCompleted {
        tool_call_id: String,
        tool_name: String,
        output: String,
        is_error: bool,
    },
    /// User approval is required before a tool action can proceed.
    ApprovalRequired {
        approval_id: String,
        tool_name: String,
        arguments: Value,
    },
    /// Cumulative token usage statistics.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        model: String,
    },
    /// Stream or turn completed with the final outcome/finish reason.
    Finished { outcome: String },
    /// An error occurred during execution or streaming.
    Error(String),
}

impl CadeStreamEvent {
    /// Returns the text content if this event is a [`CadeStreamEvent::MessageDelta`].
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::MessageDelta(t) => Some(t),
            _ => None,
        }
    }

    /// Returns the thought text if this event is a [`CadeStreamEvent::Thought`].
    pub fn as_thought(&self) -> Option<&str> {
        match self {
            Self::Thought(t) => Some(t),
            _ => None,
        }
    }

    /// Returns true if this event indicates a tool is starting execution.
    pub fn is_tool_executing(&self) -> bool {
        matches!(self, Self::ToolExecuting { .. })
    }

    /// Returns true if this event indicates the execution has finished.
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Finished { .. })
    }

    /// Try to parse a loosely-typed [`cade_api_types::StreamEvent`] into a strongly-typed [`CadeStreamEvent`].
    pub fn from_stream_event(event: &cade_api_types::StreamEvent) -> Option<Self> {
        match event.msg_type() {
            "assistant_message" => event.content().map(|c| Self::MessageDelta(c.to_string())),
            "reasoning_message" => event.reasoning().map(|r| Self::Thought(r.to_string())),
            "tool_call_message" => {
                let tc = event.data.get("tool_call")?;
                let id = tc
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let name = tc
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let arguments = tc.get("arguments").cloned().unwrap_or(Value::Null);
                Some(Self::ToolExecuting {
                    tool_call_id: id,
                    tool_name: name,
                    arguments,
                })
            }
            "tool_result_message" => {
                let tr = event.data.get("tool_result")?;
                let id = tr
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let name = tr
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let output = tr
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let is_error = tr
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Some(Self::ToolCompleted {
                    tool_call_id: id,
                    tool_name: name,
                    output,
                    is_error,
                })
            }
            "usage_statistics" => {
                let input = event
                    .data
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let output = event
                    .data
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let model = event
                    .data
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                Some(Self::Usage {
                    input_tokens: input,
                    output_tokens: output,
                    model,
                })
            }
            "finish_reason" => {
                let reason = event
                    .data
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("done")
                    .to_string();
                Some(Self::Finished { outcome: reason })
            }
            "error" => {
                let err_msg = event
                    .data
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown error")
                    .to_string();
                Some(Self::Error(err_msg))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_stream_event_parsing() {
        let text_event = cade_api_types::StreamEvent {
            message_type: "assistant_message".to_string(),
            data: json!({ "content": "Hello SDK!" }),
        };
        let parsed = CadeStreamEvent::from_stream_event(&text_event);
        assert_eq!(
            parsed,
            Some(CadeStreamEvent::MessageDelta("Hello SDK!".to_string()))
        );
        assert_eq!(
            parsed.as_ref().and_then(|e| e.as_text()),
            Some("Hello SDK!")
        );

        let tool_call_event = cade_api_types::StreamEvent {
            message_type: "tool_call_message".to_string(),
            data: json!({
                "tool_call": {
                    "id": "call-42",
                    "name": "read_file",
                    "arguments": { "path": "Cargo.toml" }
                }
            }),
        };
        let parsed_tc = CadeStreamEvent::from_stream_event(&tool_call_event);
        assert!(
            parsed_tc
                .as_ref()
                .map(|e| e.is_tool_executing())
                .unwrap_or(false)
        );
    }
}
