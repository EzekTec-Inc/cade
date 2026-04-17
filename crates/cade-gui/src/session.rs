//! Post-login session state machine for the cade-gui WASM app.
//!
//! **Pure logic, no browser dependencies.**  After the user submits a
//! token via [`crate::login::LoginState`], the app transitions into
//! this machine which tracks the connection lifecycle:
//!
//! ```text
//!   LoginState::Submitted { key }
//!          │
//!          ▼
//!   SessionState::Connecting { server_url, token }
//!          │
//!     ┌────┴────┐
//!     ▼         ▼
//!  Connected  ConnectionFailed { error }
//!                    │
//!                    ▼ (on_retry)
//!              back to LoginState
//! ```
//!
//! The wasm render loop (`app.rs`) drives this machine by spawning
//! async tasks that call `http_wasm::{get_health, get_agents}` and
//! feeding the results back via `on_health` / `on_agents` / `on_error`.

use cade_api_types::{AgentInfo, ChatMessage, HealthInfo};

/// Post-login session state.
///
/// Created from `LoginState::Submitted` — the token and server URL are
/// captured at construction and never mutated.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionState {
    /// Token submitted, waiting for health + agent-list responses.
    Connecting {
        server_url: String,
        token: String,
    },
    /// Health check succeeded; waiting for agent list.
    HealthOk {
        server_url: String,
        token: String,
        health: HealthInfo,
    },
    /// Both health and agent list succeeded — session is live.
    Connected {
        server_url: String,
        token: String,
        health: HealthInfo,
        agents: Vec<AgentInfo>,
        /// Index into `agents` of the currently selected agent, if any.
        selected_agent: Option<usize>,
        /// Messages for the selected agent (empty until an agent is selected
        /// and the fetch completes).
        messages: Vec<ChatMessage>,
    },
    /// One of the bootstrap requests failed.
    ConnectionFailed {
        server_url: String,
        token: String,
        error: String,
    },
}

impl SessionState {
    /// Begin a new session after the user submits their token.
    ///
    /// `server_url` is the base URL of the cade-server instance (from
    /// `Config::server_url`).  `token` is the trimmed API key from
    /// `LoginState::Submitted { key }`.
    pub fn start(server_url: &str, token: &str) -> Self {
        Self::Connecting {
            server_url: server_url.to_string(),
            token: token.to_string(),
        }
    }

    /// The server URL this session targets.
    pub fn server_url(&self) -> &str {
        match self {
            Self::Connecting { server_url, .. }
            | Self::HealthOk { server_url, .. }
            | Self::Connected { server_url, .. }
            | Self::ConnectionFailed { server_url, .. } => server_url,
        }
    }

    /// The bearer token for this session.
    pub fn token(&self) -> &str {
        match self {
            Self::Connecting { token, .. }
            | Self::HealthOk { token, .. }
            | Self::Connected { token, .. }
            | Self::ConnectionFailed { token, .. } => token,
        }
    }

    /// Feed a successful health-check result.
    ///
    /// Only transitions from `Connecting` → `HealthOk`.
    /// No-op in any other state (idempotent against duplicate calls).
    pub fn on_health(&mut self, health: HealthInfo) {
        if let Self::Connecting {
            server_url, token, ..
        } = self
        {
            *self = Self::HealthOk {
                server_url: std::mem::take(server_url),
                token: std::mem::take(token),
                health,
            };
        }
    }

    /// Feed a successful agent-list result.
    ///
    /// Only transitions from `HealthOk` → `Connected`.
    /// No-op in any other state.
    pub fn on_agents(&mut self, agents: Vec<AgentInfo>) {
        if let Self::HealthOk {
            server_url,
            token,
            health,
            ..
        } = self
        {
            *self = Self::Connected {
                server_url: std::mem::take(server_url),
                token: std::mem::take(token),
                health: health.clone(),
                agents,
                selected_agent: None,
                messages: Vec::new(),
            };
        }
    }

    /// Feed an error from either the health or agent-list request.
    ///
    /// Transitions from `Connecting` or `HealthOk` → `ConnectionFailed`.
    /// No-op if already `Connected` or `ConnectionFailed`.
    pub fn on_error(&mut self, error: String) {
        match self {
            Self::Connecting {
                server_url, token, ..
            }
            | Self::HealthOk {
                server_url, token, ..
            } => {
                *self = Self::ConnectionFailed {
                    server_url: std::mem::take(server_url),
                    token: std::mem::take(token),
                    error,
                };
            }
            _ => {}
        }
    }

    /// Select an agent by index.  Clears messages so the UI can show a
    /// loading state while the fetch is in flight.
    ///
    /// Returns `true` if the selection changed (caller should spawn a
    /// message fetch), `false` if it was a no-op (already selected, or
    /// index out of bounds, or not in `Connected` state).
    pub fn on_select_agent(&mut self, idx: usize) -> bool {
        if let Self::Connected {
            agents,
            selected_agent,
            messages,
            ..
        } = self
        {
            if idx >= agents.len() {
                return false;
            }
            if *selected_agent == Some(idx) {
                return false;
            }
            *selected_agent = Some(idx);
            messages.clear();
            true
        } else {
            false
        }
    }

    /// Feed the message list fetched for the currently selected agent.
    ///
    /// Only applies when `Connected` and an agent is selected.  No-op
    /// otherwise.
    pub fn on_messages(&mut self, msgs: Vec<ChatMessage>) {
        if let Self::Connected { messages, .. } = self {
            *messages = msgs;
        }
    }

    /// The currently selected agent's ID, if any.
    pub fn selected_agent_id(&self) -> Option<&str> {
        if let Self::Connected {
            agents,
            selected_agent: Some(idx),
            ..
        } = self
        {
            agents.get(*idx).map(|a| a.id.as_str())
        } else {
            None
        }
    }

    /// Whether the caller should attempt a retry (re-enter the login flow).
    /// Only meaningful in `ConnectionFailed`.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::ConnectionFailed { .. })
    }

    /// Whether the session is fully established.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cade_api_types::HealthInfo;

    fn test_health() -> HealthInfo {
        HealthInfo {
            status: "ok".to_string(),
            server: Some("cade-server".to_string()),
            version: Some("0.2.0".to_string()),
        }
    }

    fn test_agents() -> Vec<AgentInfo> {
        vec![AgentInfo {
            id: "agent-1".to_string(),
            name: "Test Agent".to_string(),
            model: Some("gpt-4o".to_string()),
            provider: None,
        }]
    }

    // ── Construction ────────────────────────────────────────────────────

    #[test]
    fn start_enters_connecting_state() {
        let s = SessionState::start("http://localhost:8284", "my-token");
        assert!(matches!(s, SessionState::Connecting { .. }));
        assert_eq!(s.server_url(), "http://localhost:8284");
        assert_eq!(s.token(), "my-token");
    }

    // ── Happy path: Connecting → HealthOk → Connected ───────────────────

    #[test]
    fn on_health_transitions_connecting_to_health_ok() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        match &s {
            SessionState::HealthOk { health, .. } => {
                assert_eq!(health.status, "ok");
            }
            other => panic!("expected HealthOk, got {other:?}"),
        }
    }

    #[test]
    fn on_agents_transitions_health_ok_to_connected() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(test_agents());
        match &s {
            SessionState::Connected { agents, health, .. } => {
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0].id, "agent-1");
                assert_eq!(health.status, "ok");
            }
            other => panic!("expected Connected, got {other:?}"),
        }
        assert!(s.is_connected());
    }

    #[test]
    fn connected_preserves_server_url_and_token() {
        let mut s = SessionState::start("http://my-server:9000", "secret-key");
        s.on_health(test_health());
        s.on_agents(vec![]);
        assert_eq!(s.server_url(), "http://my-server:9000");
        assert_eq!(s.token(), "secret-key");
    }

    // ── Error path ──────────────────────────────────────────────────────

    #[test]
    fn on_error_from_connecting_transitions_to_failed() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_error("unauthorized".to_string());
        match &s {
            SessionState::ConnectionFailed { error, .. } => {
                assert_eq!(error, "unauthorized");
            }
            other => panic!("expected ConnectionFailed, got {other:?}"),
        }
        assert!(s.is_failed());
        assert!(!s.is_connected());
    }

    #[test]
    fn on_error_from_health_ok_transitions_to_failed() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_error("agent fetch failed".to_string());
        assert!(s.is_failed());
    }

    #[test]
    fn on_error_preserves_server_url_and_token() {
        let mut s = SessionState::start("http://y", "t");
        s.on_error("boom".to_string());
        assert_eq!(s.server_url(), "http://y");
        assert_eq!(s.token(), "t");
    }

    // ── Idempotency / no-op guards ─────────────────────────────────────

    #[test]
    fn on_health_is_noop_after_connected() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(test_agents());
        assert!(s.is_connected());
        // Second health call should be ignored.
        s.on_health(HealthInfo {
            status: "changed".to_string(),
            server: None,
            version: None,
        });
        // Still connected with original health.
        match &s {
            SessionState::Connected { health, .. } => assert_eq!(health.status, "ok"),
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn on_agents_is_noop_from_connecting() {
        let mut s = SessionState::start("http://x", "tok");
        // Calling on_agents before on_health should be a no-op.
        s.on_agents(test_agents());
        assert!(matches!(s, SessionState::Connecting { .. }));
    }

    #[test]
    fn on_error_is_noop_after_connected() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(test_agents());
        assert!(s.is_connected());
        s.on_error("late error".to_string());
        // Should still be connected — error after success is ignored.
        assert!(s.is_connected());
    }

    #[test]
    fn on_error_is_noop_after_already_failed() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_error("first".to_string());
        s.on_error("second".to_string());
        // First error sticks.
        match &s {
            SessionState::ConnectionFailed { error, .. } => assert_eq!(error, "first"),
            other => panic!("expected ConnectionFailed, got {other:?}"),
        }
    }

    // ── Empty agents list ───────────────────────────────────────────────

    #[test]
    fn connected_with_empty_agents_is_valid() {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(vec![]);
        assert!(s.is_connected());
        match &s {
            SessionState::Connected { agents, .. } => assert!(agents.is_empty()),
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    // ── Agent selection ─────────────────────────────────────────────────

    fn make_connected() -> SessionState {
        let mut s = SessionState::start("http://x", "tok");
        s.on_health(test_health());
        s.on_agents(test_agents());
        s
    }

    #[test]
    fn on_select_agent_sets_selection_and_clears_messages() {
        let mut s = make_connected();
        assert!(s.on_select_agent(0));
        assert_eq!(s.selected_agent_id(), Some("agent-1"));
        match &s {
            SessionState::Connected {
                selected_agent,
                messages,
                ..
            } => {
                assert_eq!(*selected_agent, Some(0));
                assert!(messages.is_empty());
            }
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn on_select_agent_same_index_is_noop() {
        let mut s = make_connected();
        assert!(s.on_select_agent(0));
        // Second call with same index returns false.
        assert!(!s.on_select_agent(0));
    }

    #[test]
    fn on_select_agent_out_of_bounds_is_noop() {
        let mut s = make_connected();
        assert!(!s.on_select_agent(99));
        assert_eq!(s.selected_agent_id(), None);
    }

    #[test]
    fn on_select_agent_not_connected_is_noop() {
        let mut s = SessionState::start("http://x", "tok");
        assert!(!s.on_select_agent(0));
    }

    #[test]
    fn on_messages_populates_messages() {
        let mut s = make_connected();
        s.on_select_agent(0);
        let msgs = vec![ChatMessage {
            id: "m1".to_string(),
            role: "user".to_string(),
            content: serde_json::Value::String("hello".to_string()),
            conversation_id: None,
        }];
        s.on_messages(msgs.clone());
        match &s {
            SessionState::Connected { messages, .. } => {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].id, "m1");
            }
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn on_select_agent_clears_previous_messages() {
        let mut s = make_connected();
        // Add a second agent so we can switch.
        if let SessionState::Connected { agents, .. } = &mut s {
            agents.push(AgentInfo {
                id: "agent-2".to_string(),
                name: "Second".to_string(),
                model: None,
                provider: None,
            });
        }
        s.on_select_agent(0);
        s.on_messages(vec![ChatMessage {
            id: "m1".to_string(),
            role: "user".to_string(),
            content: serde_json::Value::String("hi".to_string()),
            conversation_id: None,
        }]);
        // Switch to agent 2 — messages should be cleared.
        assert!(s.on_select_agent(1));
        assert_eq!(s.selected_agent_id(), Some("agent-2"));
        match &s {
            SessionState::Connected { messages, .. } => {
                assert!(messages.is_empty(), "messages should be cleared on agent switch");
            }
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn selected_agent_id_none_when_no_selection() {
        let s = make_connected();
        assert_eq!(s.selected_agent_id(), None);
    }
}
