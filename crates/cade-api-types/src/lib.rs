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
}
