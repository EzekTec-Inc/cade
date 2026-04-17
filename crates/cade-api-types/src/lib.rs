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

#[cfg(test)]
mod tests {
    use super::*;

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
}
