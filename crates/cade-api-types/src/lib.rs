//! Shared API wire types between `cade-server` and the `cade-gui` WASM client.
//!
//! Strict rules:
//! - Pure `serde` only. No tokio / reqwest / parking_lot / native-only deps.
//! - Must compile under both `x86_64-unknown-linux-gnu` and
//!   `wasm32-unknown-unknown`. Enforced by CI target.
//! - Types mirror the JSON shapes returned by the existing `cade-server` REST
//!   endpoints and SSE streams. They are **additive**: adding fields is OK,
//!   removing or renaming is a breaking API change that requires approval.

use serde::{Deserialize, Serialize};

/// Minimal agent descriptor — what `GET /v1/agents` returns per row.
///
/// Fields marked `Option` are absent in some server responses (older rows,
/// freshly created agents). Keep them optional to stay tolerant of drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Theme name last persisted via `/theme <name>` (built-in or user theme).
    /// `None` → GUI should use the default dark theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

/// Response shape of `GET /v1/health`.
///
/// Mirrors the JSON returned by `cade-server` — see
/// `crates/cade-server/src/server/api/health.rs::get_health`. Fields are
/// additive; never remove or rename without bumping the wire contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthInfo {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// A single message in a conversation — what `GET /v1/agents/:id/messages`
/// returns per row.
///
/// The `content` field is `serde_json::Value` because the server stores both
/// plain-text strings and structured JSON (tool calls, multi-part content).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

/// A conversation associated with an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationInfo {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
}

/// A single event from the server's SSE stream (`POST /v1/agents/:id/messages/stream`).
///
/// The `message_type` discriminator identifies the event kind, while `data`
/// holds all other fields via serde `flatten`:
///
/// | `message_type`        | Extra fields                                  |
/// |-----------------------|-----------------------------------------------|
/// | `stream_start`        | `conversation_id`, `run_id` (optional)        |
/// | `assistant_message`   | `content` (string, possibly incremental)      |
/// | `reasoning_message`   | `reasoning` (string)                          |
/// | `tool_call_message`   | `tool_call` `{ id, name, arguments }`         |
/// | `tool_result_message` | `tool_result` `{ id, name, output, is_error }`|
/// | `usage_statistics`    | `input_tokens`, `output_tokens`, `model`…     |
/// | `finish_reason`       | `reason` (string)                             |
/// | `error`               | `error` (string)                              |
///
/// The wire format always carries `message_type` as a top-level key; extra
/// fields are merged into `data` so the GUI can access them without needing
/// a dedicated variant per type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEvent {
    #[serde(default)]
    pub message_type: String,
    /// Catch-all for every field other than `message_type`.
    #[serde(flatten)]
    pub data: serde_json::Value,
}

impl StreamEvent {
    pub fn msg_type(&self) -> &str {
        self.message_type.as_str()
    }

    /// Extract `content` from an `assistant_message` (or any event carrying it).
    pub fn content(&self) -> Option<&str> {
        self.data.get("content").and_then(|v| v.as_str())
    }

    /// Extract `reasoning` from a `reasoning_message`.
    pub fn reasoning(&self) -> Option<&str> {
        self.data.get("reasoning").and_then(|v| v.as_str())
    }

    /// Extract the error string from an `error` event.
    pub fn error(&self) -> Option<&str> {
        self.data.get("error").and_then(|v| v.as_str())
    }

    /// Extract `tool_name` from a `tool_call_message` or `tool_result_message`.
    pub fn tool_name(&self) -> Option<&str> {
        self.data.get("tool_name").and_then(|v| v.as_str())
    }

    /// Extract `tool_args` from a `tool_call_message`.
    pub fn tool_args(&self) -> Option<&str> {
        self.data.get("tool_args").and_then(|v| v.as_str())
    }

    /// Extract `tool_call_id` from a `tool_call_message` / `tool_result_message`.
    pub fn tool_call_id(&self) -> Option<&str> {
        self.data.get("tool_call_id").and_then(|v| v.as_str())
    }

    /// Deserialize the `tool_call` object (id, name, arguments).
    pub fn tool_call(&self) -> Option<ToolCallData> {
        self.data
            .get("tool_call")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Deserialize the `tool_result` object (id, name, output, is_error).
    pub fn tool_result(&self) -> Option<ToolResultData> {
        self.data
            .get("tool_result")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }
}

/// A tool call within a `tool_call_message` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallData {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// A tool result within a `tool_result_message` event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultData {
    pub id: String,
    pub name: String,
    pub output: String,
    #[serde(default)]
    pub is_error: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_info_parses_server_shape() {
        // Exact shape returned by get_health() in cade-server.
        let wire = r#"{"status":"ok","server":"cade-server","version":"0.2.0"}"#;
        let h: HealthInfo = serde_json::from_str(wire).expect("parse");
        assert_eq!(h.status, "ok");
        assert_eq!(h.server.as_deref(), Some("cade-server"));
        assert_eq!(h.version.as_deref(), Some("0.2.0"));
    }

    #[test]
    fn health_info_tolerates_missing_optional_fields() {
        // Future-proof: older servers may only return `status`.
        let wire = r#"{"status":"ok"}"#;
        let h: HealthInfo = serde_json::from_str(wire).expect("parse");
        assert_eq!(h.status, "ok");
        assert_eq!(h.server, None);
        assert_eq!(h.version, None);
    }

    #[test]
    fn agent_info_round_trips_via_json() {
        // -- Fixture
        let src = AgentInfo {
            id: "agent-abc".to_string(),
            name: "Test Agent".to_string(),
            model: Some("gpt-4o".to_string()),
            provider: None,
            theme: None,
        };

        // -- Exec
        let wire = serde_json::to_string(&src).expect("serialize");
        let back: AgentInfo = serde_json::from_str(&wire).expect("deserialize");

        // -- Check
        assert_eq!(back, src);
        // `provider: None` must be omitted on the wire.
        assert!(
            !wire.contains("\"provider\""),
            "None fields must be skipped: {wire}"
        );
    }

    #[test]
    fn agent_info_parses_server_shape_without_optional_fields() {
        // Server returns agents with missing model/provider for not-yet-configured rows.
        let wire = r#"{"id":"a","name":"n"}"#;
        let a: AgentInfo = serde_json::from_str(wire).expect("tolerant parse");
        assert_eq!(a.id, "a");
        assert_eq!(a.name, "n");
        assert_eq!(a.model, None);
        assert_eq!(a.provider, None);
    }

    #[test]
    fn chat_message_parses_server_shape() {
        // Exact shape returned by GET /v1/agents/:id/messages in cade-server.
        let wire = r#"{"id":"msg-1","role":"user","content":"hello","conversation_id":"conv-1"}"#;
        let m: ChatMessage = serde_json::from_str(wire).expect("parse");
        assert_eq!(m.id, "msg-1");
        assert_eq!(m.role, "user");
        assert_eq!(m.content, serde_json::Value::String("hello".into()));
        assert_eq!(m.conversation_id.as_deref(), Some("conv-1"));
    }

    #[test]
    fn chat_message_tolerates_missing_optional_fields() {
        let wire = r#"{"id":"m","role":"assistant","content":"hi"}"#;
        let m: ChatMessage = serde_json::from_str(wire).expect("tolerant parse");
        assert_eq!(m.id, "m");
        assert_eq!(m.role, "assistant");
        assert_eq!(m.conversation_id, None);
    }

    #[test]
    fn chat_message_content_can_be_structured_json() {
        // The server sometimes stores content as a JSON object (tool calls, etc.)
        let wire = r#"{"id":"m","role":"tool","content":{"tool":"bash","output":"ok"}}"#;
        let m: ChatMessage = serde_json::from_str(wire).expect("parse structured content");
        assert!(m.content.is_object(), "content should be a JSON object");
    }

    // -- StreamEvent

    #[test]
    fn stream_event_parses_assistant_message() {
        let wire = r#"{"message_type":"assistant_message","content":"Hello, world!"}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "assistant_message");
        assert_eq!(e.content(), Some("Hello, world!"));
    }

    #[test]
    fn stream_event_parses_stream_start() {
        let wire = r#"{"message_type":"stream_start","conversation_id":"conv-1","run_id":"run-1"}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "stream_start");
        assert_eq!(
            e.data.get("conversation_id").and_then(|v| v.as_str()),
            Some("conv-1")
        );
        assert_eq!(
            e.data.get("run_id").and_then(|v| v.as_str()),
            Some("run-1")
        );
    }

    #[test]
    fn stream_event_parses_reasoning_message() {
        let wire = r#"{"message_type":"reasoning_message","reasoning":"thinking step..."}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "reasoning_message");
        assert_eq!(e.reasoning(), Some("thinking step..."));
    }

    #[test]
    fn stream_event_parses_error() {
        let wire = r#"{"message_type":"error","error":"LLM call failed"}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "error");
        assert_eq!(e.error(), Some("LLM call failed"));
    }

    #[test]
    fn stream_event_parses_tool_call() {
        let wire = r#"{"message_type":"tool_call_message","tool_call":{"id":"tc1","name":"bash","arguments":"{}"}}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), "tool_call_message");
        let tc = e.data.get("tool_call").expect("tool_call present");
        assert_eq!(tc["name"].as_str(), Some("bash"));
    }

    #[test]
    fn stream_event_defaults_message_type() {
        let wire = r#"{"some":"thing"}"#;
        let e: StreamEvent = serde_json::from_str(wire).expect("parse");
        assert_eq!(e.msg_type(), ""); // default empty
        assert_eq!(e.data.get("some").and_then(|v| v.as_str()), Some("thing"));
    }
}
