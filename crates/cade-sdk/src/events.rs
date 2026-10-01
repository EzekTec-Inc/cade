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
    ApprovalResolved {
        approval_id: String,
        status: String,
    },
    QuestionRequired {
        question_id: String,
        questions: Vec<cade_api_types::Question>,
    },
    QuestionResolved {
        question_id: String,
        status: String,
    },
    /// Cumulative token usage statistics.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        model: String,
    },
    /// Stream or turn completed with the final outcome/finish reason.
    Finished {
        outcome: String,
    },
    /// A canonical run's provider response ended; the agent may execute tools
    /// and produce more responses before its durable `run_done` event.
    ResponseFinished {
        reason: String,
    },
    /// A structured task plan update emitted when an agent plans or updates steps.
    PlanUpdate {
        plan: Value,
    },
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

    /// Returns the plan payload if this event is a [`CadeStreamEvent::PlanUpdate`].
    pub fn as_plan(&self) -> Option<&Value> {
        match self {
            Self::PlanUpdate { plan } => Some(plan),
            _ => None,
        }
    }

    /// Try to parse a loosely-typed [`cade_api_types::StreamEvent`] into a strongly-typed [`CadeStreamEvent`].
    pub fn from_stream_event(event: &cade_api_types::StreamEvent) -> Option<Self> {
        match event.msg_type() {
            "assistant_message" => event.content().map(|c| Self::MessageDelta(c.to_string())),
            "reasoning_message" | "thought" => event
                .reasoning()
                .or_else(|| event.content())
                .map(|r| Self::Thought(r.to_string())),
            "tool_call_message" | "tool_executing" => {
                let id = event.tool_call_id().unwrap_or_default().to_string();
                let name = event.tool_name().unwrap_or_default().to_string();
                let arguments = event.tool_arguments();
                Some(Self::ToolExecuting {
                    tool_call_id: id,
                    tool_name: name,
                    arguments,
                })
            }
            "tool_result_message" | "tool_completed" => {
                let id = event.tool_call_id().unwrap_or_default().to_string();
                let name = event.tool_name().unwrap_or_default().to_string();
                let output = event.tool_output().unwrap_or_default().to_string();
                let is_error = event
                    .tool_payload()
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
            "approval_required" => {
                let request = event.approval_request()?;
                Some(Self::ApprovalRequired {
                    approval_id: request.id.to_owned(),
                    tool_name: request.tool_name.to_owned(),
                    arguments: cade_api_types::decode_json_value(request.arguments.clone()),
                })
            }
            "approval_resolved" if event.approval_id().is_some_and(|id| id.starts_with("q-")) => {
                Some(Self::QuestionResolved {
                    question_id: event.approval_id()?.to_owned(),
                    status: event.data["status"].as_str().unwrap_or_default().to_owned(),
                })
            }
            "approval_resolved" => Some(Self::ApprovalResolved {
                approval_id: event.approval_id()?.to_owned(),
                status: event.data["status"].as_str().unwrap_or_default().to_owned(),
            }),
            "question_required" => {
                let request = event.question_request()?;
                Some(Self::QuestionRequired {
                    question_id: request.id,
                    questions: request.questions,
                })
            }
            "question_resolved" => Some(Self::QuestionResolved {
                question_id: event.approval_id()?.to_owned(),
                status: event.data["status"].as_str().unwrap_or_default().to_owned(),
            }),
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
            "finish_reason" if event.run_id().is_some() => Some(Self::ResponseFinished {
                reason: event.data["reason"].as_str().unwrap_or_default().to_owned(),
            }),
            "finish_reason" | "run_done" => {
                let reason = event
                    .data
                    .get(if event.is_terminal() {
                        "status"
                    } else {
                        "reason"
                    })
                    .and_then(|v| v.as_str())
                    .unwrap_or("done")
                    .to_string();
                Some(Self::Finished { outcome: reason })
            }
            "plan_update" => {
                let plan = event.data.get("plan").cloned().unwrap_or(Value::Null);
                Some(Self::PlanUpdate { plan })
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
    fn wire_projection_covers_approvals_questions_resolution_and_run_done() {
        let project = |value| {
            CadeStreamEvent::from_stream_event(&serde_json::from_value(value).unwrap()).unwrap()
        };
        assert_eq!(
            project(
                json!({"message_type":"approval_required","id":"app-1","tool_name":"write_file","arguments":"{\"path\":\"notes.txt\"}","reason":"review"})
            ),
            CadeStreamEvent::ApprovalRequired {
                approval_id: "app-1".into(),
                tool_name: "write_file".into(),
                arguments: json!({"path":"notes.txt"}),
            }
        );
        assert!(
            matches!(project(json!({"event_type":"approval_resolved","id":"app-1","status":"denied"})), CadeStreamEvent::ApprovalResolved { status, .. } if status == "denied")
        );
        assert!(
            matches!(project(json!({"type":"question_required","id":"q-1","questions":[{"header":"Scope","question":"Which scope?","options":[{"label":"Local"}]}]})), CadeStreamEvent::QuestionRequired { question_id, questions } if question_id == "q-1" && questions[0].header == "Scope")
        );
        assert!(
            matches!(project(json!({"event_type":"approval_resolved","id":"q-1","status":"approved:{\"Scope\":\"Local\"}"})), CadeStreamEvent::QuestionResolved { question_id, .. } if question_id == "q-1")
        );
        assert_eq!(
            project(
                json!({"message_type":"run_done","run_id":"r","seq_id":11,"status":"cancelled"})
            ),
            CadeStreamEvent::Finished {
                outcome: "cancelled".into()
            }
        );
        let response_end = project(
            json!({"message_type":"finish_reason","run_id":"r","seq_id":10,"reason":"tool_calls"}),
        );
        assert!(
            !response_end.is_finished(),
            "canonical provider response completion cannot terminate an agent run"
        );
        assert_eq!(
            response_end,
            CadeStreamEvent::ResponseFinished {
                reason: "tool_calls".into()
            }
        );
        assert!(
            project(json!({"message_type":"finish_reason","reason":"stop"})).is_finished(),
            "legacy streams keep their completion projection"
        );
    }

    #[test]
    fn wire_projection_nested_and_flat_tools_share_provider_independent_semantics() {
        let nested: cade_api_types::StreamEvent = serde_json::from_value(json!({"message_type":"tool_call_message","tool_call":{"id":"tc","name":"inspect","arguments":{"path":"Cargo.toml"}}})).unwrap();
        let flat: cade_api_types::StreamEvent = serde_json::from_value(json!({"message_type":"tool_executing","tool_call_id":"tc","tool_name":"inspect","arguments":"{\"path\":\"Cargo.toml\"}"})).unwrap();
        assert_eq!(
            CadeStreamEvent::from_stream_event(&nested),
            CadeStreamEvent::from_stream_event(&flat)
        );
        let result: cade_api_types::StreamEvent = serde_json::from_value(json!({"message_type":"tool_result_message","tool_result":{"tool_call_id":"tc","tool_name":"inspect","output":"actual output","is_error":true}})).unwrap();
        assert_eq!(
            CadeStreamEvent::from_stream_event(&result),
            Some(CadeStreamEvent::ToolCompleted {
                tool_call_id: "tc".into(),
                tool_name: "inspect".into(),
                output: "actual output".into(),
                is_error: true
            })
        );
    }

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

    #[test]
    fn test_stream_event_plan_update() {
        let plan_event = cade_api_types::StreamEvent {
            message_type: "plan_update".to_string(),
            data: json!({
                "plan": {
                    "title": "Roadmap",
                    "steps": [
                        { "id": 1, "description": "Step 1", "is_done": false }
                    ]
                }
            }),
        };
        let parsed = CadeStreamEvent::from_stream_event(&plan_event);
        assert!(matches!(parsed, Some(CadeStreamEvent::PlanUpdate { .. })));
        let plan_val = parsed.as_ref().and_then(|e| e.as_plan()).unwrap();
        assert_eq!(plan_val["title"], "Roadmap");
        assert_eq!(plan_val["steps"][0]["description"], "Step 1");
    }
}
