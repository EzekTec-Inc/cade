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
    vec![
        AgentInfo {
            id: "agent-1".to_string(),
            name: "Test Agent".to_string(),
            model: Some("gpt-4o".to_string()),
            provider: None,
            theme: None,
        },
        AgentInfo {
            id: "agent-2".to_string(),
            name: "Second Agent".to_string(),
            model: None,
            provider: None,
            theme: None,
        },
    ]
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
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession { agents, health, .. } = &**session;

            assert_eq!(agents.len(), 2);
            assert_eq!(agents[0].id, "agent-1");
            assert_eq!(agents[1].id, "agent-2");
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
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession { health, .. } = &**session;
            assert_eq!(health.status, "ok");
        }
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
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession { agents, .. } = &**session;
            assert!(agents.is_empty());
        }
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
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                selected_agent,
                messages,
                ..
            } = &**session;

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
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession { messages, .. } = &**session;

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
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { agents, .. } = &mut **session;
        agents.push(AgentInfo {
            id: "agent-2".to_string(),
            name: "Second".to_string(),
            model: None,
            provider: None,
            theme: None,
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
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession { messages, .. } = &**session;

            assert!(
                messages.is_empty(),
                "messages should be cleared on agent switch"
            );
        }
        other => panic!("expected Connected, got {other:?}"),
    }
}

#[test]
fn selected_agent_id_none_when_no_selection() {
    let s = make_connected();
    assert_eq!(s.selected_agent_id(), None);
}

// ── Input / Send / Stream ───────────────────────────────────────────

fn make_connected_with_agent_selected() -> SessionState {
    let mut s = make_connected();
    s.on_select_agent(0);
    s
}

#[test]
fn on_send_returns_trimmed_input_and_appends_user_message() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "  hello world  ".to_string();
    }
    let result = s.on_send();
    assert_eq!(result.as_deref(), Some("hello world"));
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            messages,
            input_buffer,
            streaming,
            ..
        } = &**session;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(
            messages[0].content,
            serde_json::Value::String("hello world".to_string())
        );
        assert!(input_buffer.is_empty());
        assert!(*streaming);
    } else {
        panic!("expected Connected");
    }
}

#[test]
fn on_send_noop_when_no_agent_selected() {
    let mut s = make_connected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "hello".to_string();
    }
    assert_eq!(s.on_send(), None);
}

#[test]
fn on_send_noop_when_empty_buffer() {
    let mut s = make_connected_with_agent_selected();
    assert_eq!(s.on_send(), None);
}

#[test]
fn on_send_noop_when_whitespace_only() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "   ".to_string();
    }
    assert_eq!(s.on_send(), None);
}

#[test]
fn on_send_noop_while_streaming() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession {
            input_buffer,
            streaming,
            ..
        } = &mut **session;
        *input_buffer = "hello".to_string();
        *streaming = true;
    }
    assert_eq!(s.on_send(), None);
}

#[test]
fn on_stream_chunk_creates_then_appends_assistant_message() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { streaming, .. } = &mut **session;
        *streaming = true;
    }
    s.on_stream_chunk("Hello");
    s.on_stream_chunk(", world!");

    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(
            messages[0].content,
            serde_json::Value::String("Hello, world!".to_string())
        );
    } else {
        panic!("expected Connected");
    }
}

#[test]
fn on_stream_chunk_noop_when_not_streaming() {
    let mut s = make_connected_with_agent_selected();
    s.on_stream_chunk("ignored");
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert!(messages.is_empty());
    }
}

#[test]
fn auto_scroll_true_by_default() {
    let s = make_connected();
    assert!(s.auto_scroll());
}

#[test]
fn disable_auto_scroll_sets_false() {
    let mut s = make_connected();
    s.disable_auto_scroll();
    assert!(!s.auto_scroll());
}

#[test]
fn enable_auto_scroll_restores_true() {
    let mut s = make_connected();
    s.disable_auto_scroll();
    s.enable_auto_scroll();
    assert!(s.auto_scroll());
}

#[test]
fn on_stream_chunk_re_enables_auto_scroll() {
    let mut s = make_connected_with_agent_selected();
    s.disable_auto_scroll();
    assert!(!s.auto_scroll());
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "hi".into();
    }
    s.on_send().unwrap();
    s.on_stream_chunk("Hello");
    assert!(s.auto_scroll(), "first chunk should re-enable auto_scroll");
}

#[test]
fn on_stream_done_clears_streaming_flag() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { streaming, .. } = &mut **session;
        *streaming = true;
    }
    assert!(s.is_streaming());
    s.on_stream_done();
    assert!(!s.is_streaming());
}

#[test]
fn full_send_stream_cycle() {
    let mut s = make_connected_with_agent_selected();
    // Type and send.
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "What is Rust?".to_string();
    }
    let input = s.on_send().expect("should send");
    assert_eq!(input, "What is Rust?");
    assert!(s.is_streaming());

    // Stream chunks arrive.
    s.on_stream_chunk("Rust is ");
    s.on_stream_chunk("a systems programming language.");
    s.on_stream_done();

    assert!(!s.is_streaming());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert_eq!(messages.len(), 2); // user + assistant
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(
            messages[1].content,
            serde_json::Value::String("Rust is a systems programming language.".to_string())
        );
    } else {
        panic!("expected Connected");
    }
}

// ── Error toast ────────────────────────────────────────────────────

#[test]
fn push_error_stores_message() {
    let mut s = make_connected_with_agent_selected();
    s.push_error("stream failed");
    assert_eq!(s.error_toast(), Some("stream failed"));
}

#[test]
fn dismiss_error_clears_toast() {
    let mut s = make_connected_with_agent_selected();
    s.push_error("oops");
    s.dismiss_error();
    assert_eq!(s.error_toast(), None);
}

#[test]
fn push_error_replaces_previous() {
    let mut s = make_connected_with_agent_selected();
    s.push_error("first");
    s.push_error("second");
    assert_eq!(s.error_toast(), Some("second"));
}

#[test]
fn error_toast_none_when_no_error() {
    let s = make_connected_with_agent_selected();
    assert_eq!(s.error_toast(), None);
}

#[test]
fn push_error_also_clears_streaming() {
    let mut s = make_connected_with_agent_selected();
    // Start a stream, then an error arrives.
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "hello".to_string();
    }
    let _ = s.on_send();
    assert!(s.is_streaming());
    s.push_error("connection lost");
    assert!(!s.is_streaming(), "streaming should be cleared on error");
    assert_eq!(s.error_toast(), Some("connection lost"));
}

// ── Conversation ID ────────────────────────────────────────────────

#[test]
fn conversation_id_none_initially() {
    let s = make_connected_with_agent_selected();
    assert_eq!(s.conversation_id(), None);
}

#[test]
fn on_conversation_id_stores_id() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversation_id("conv-abc-123");
    assert_eq!(s.conversation_id(), Some("conv-abc-123"));
}

#[test]
fn on_conversation_id_replaces_previous() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversation_id("conv-1");
    s.on_conversation_id("conv-2");
    assert_eq!(s.conversation_id(), Some("conv-2"));
}

#[test]
fn select_agent_clears_conversation_id() {
    let mut s = make_connected_with_agent_selected();
    // Add a second agent so we can actually switch.
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { agents, .. } = &mut **session;
        agents.push(AgentInfo {
            id: "agent-2".to_string(),
            name: "Second Agent".to_string(),
            model: None,
            provider: None,
            theme: None,
        });
    }
    s.on_conversation_id("conv-old");
    assert!(s.on_select_agent(1)); // switch to agent-2
    assert_eq!(s.conversation_id(), None);
}

// ── Reasoning stream ───────────────────────────────────────────────

#[test]
fn on_stream_reasoning_creates_reasoning_message() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "explain".to_string();
    }
    let _ = s.on_send();

    s.on_stream_reasoning("Let me think");
    s.on_stream_reasoning(" about this.");

    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        // user + reasoning
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, "reasoning");
        assert_eq!(
            messages[1].content,
            serde_json::Value::String("Let me think about this.".to_string())
        );
    } else {
        panic!("expected Connected");
    }
}

#[test]
fn reasoning_then_assistant_are_separate_messages() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "hello".to_string();
    }
    let _ = s.on_send();

    s.on_stream_reasoning("thinking...");
    s.on_stream_chunk("The answer is 42.");

    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert_eq!(messages.len(), 3); // user + reasoning + assistant
        assert_eq!(messages[1].role, "reasoning");
        assert_eq!(messages[2].role, "assistant");
    } else {
        panic!("expected Connected");
    }
}

// ── Tool call stream ───────────────────────────────────────────────

#[test]
fn on_stream_tool_call_creates_tool_call_message() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "search".to_string();
    }
    let _ = s.on_send();

    s.on_stream_tool_call("tc-1", "web_search", r#"{"query":"rust"}"#);

    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert_eq!(messages.len(), 2); // user + tool_call
        assert_eq!(messages[1].role, "tool_call");
        let tc = &messages[1].content;
        assert_eq!(tc["name"], "web_search");
        assert_eq!(tc["id"], "tc-1");
        assert_eq!(tc["arguments"], r#"{"query":"rust"}"#);
    } else {
        panic!("expected Connected");
    }
}

#[test]
fn multiple_tool_calls_are_separate_messages() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "do stuff".to_string();
    }
    let _ = s.on_send();

    s.on_stream_tool_call("tc-1", "read_file", r#"{"path":"a.rs"}"#);
    s.on_stream_tool_call("tc-2", "write_file", r#"{"path":"b.rs"}"#);

    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert_eq!(messages.len(), 3); // user + 2 tool_calls
        assert_eq!(messages[1].content["name"], "read_file");
        assert_eq!(messages[2].content["name"], "write_file");
    } else {
        panic!("expected Connected");
    }
}

// ── Usage / finish reason ──────────────────────────────────────────

#[test]
fn on_usage_stores_stats() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "hi".to_string();
    }
    let _ = s.on_send();
    s.on_usage(100, 50, Some("gpt-4o"));
    assert_eq!(s.last_usage(), Some((100, 50, Some("gpt-4o"))));
}

#[test]
fn on_finish_reason_stores_reason() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "hi".to_string();
    }
    let _ = s.on_send();
    s.on_finish_reason("stop");
    assert_eq!(s.last_finish_reason(), Some("stop"));
}

#[test]
fn usage_and_finish_reason_none_initially() {
    let s = make_connected_with_agent_selected();
    assert_eq!(s.last_usage(), None);
    assert_eq!(s.last_finish_reason(), None);
}

#[test]
fn on_send_clears_usage_and_finish_reason() {
    let mut s = make_connected_with_agent_selected();
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "first".to_string();
    }
    let _ = s.on_send();
    s.on_usage(10, 5, None);
    s.on_finish_reason("stop");
    s.on_stream_done();

    // Send again — usage/finish should reset.
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "second".to_string();
    }
    let _ = s.on_send();
    assert_eq!(s.last_usage(), None);
    assert_eq!(s.last_finish_reason(), None);
}

// ── Conversation management tests ───────────────────────────────

fn test_conversations() -> Vec<crate::api::ConversationInfo> {
    vec![
        crate::api::ConversationInfo {
            id: "conv-1".to_string(),
            title: "First chat".to_string(),
            message_count: 3,
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        },
        crate::api::ConversationInfo {
            id: "conv-2".to_string(),
            title: "Second chat".to_string(),
            message_count: 0,
            updated_at: "2025-01-02T00:00:00Z".to_string(),
        },
    ]
}

#[test]
fn on_conversations_stores_list() {
    let mut s = make_connected_with_agent_selected();
    assert!(s.conversations().is_empty());
    s.on_conversations(test_conversations());
    assert_eq!(s.conversations().len(), 2);
    assert_eq!(s.conversations()[0].id, "conv-1");
}

#[test]
fn on_select_conversation_returns_true_when_changed() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversations(test_conversations());
    assert!(s.on_select_conversation(0));
    assert_eq!(s.selected_conversation(), Some(0));
}

#[test]
fn on_select_conversation_returns_false_when_same() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversations(test_conversations());
    s.on_select_conversation(0);
    assert!(!s.on_select_conversation(0));
}

#[test]
fn on_select_conversation_clears_messages() {
    let mut s = make_connected_with_agent_selected();
    s.on_messages(vec![ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: serde_json::Value::String("hi".into()),
        conversation_id: None,
    }]);
    s.on_conversations(test_conversations());
    s.on_select_conversation(1);
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert!(messages.is_empty());
    } else {
        panic!("not connected");
    }
}

#[test]
fn on_select_conversation_sets_conversation_id() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversations(test_conversations());
    s.on_select_conversation(1);
    assert_eq!(s.conversation_id(), Some("conv-2"));
}

#[test]
fn on_select_conversation_out_of_bounds_is_noop() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversations(test_conversations());
    assert!(!s.on_select_conversation(99));
    assert_eq!(s.selected_conversation(), None);
}

#[test]
fn on_new_conversation_clears_state() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversations(test_conversations());
    s.on_select_conversation(0);
    s.on_conversation_id("conv-1");
    s.on_messages(vec![ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: serde_json::Value::String("hi".into()),
        conversation_id: None,
    }]);
    s.on_new_conversation();
    assert_eq!(s.conversation_id(), None);
    assert_eq!(s.selected_conversation(), None);
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert!(messages.is_empty());
    }
}

#[test]
fn on_conversation_deleted_removes_entry() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversations(test_conversations()); // 2 conversations
    assert_eq!(s.conversations().len(), 2);
    s.on_conversation_deleted(0);
    assert_eq!(s.conversations().len(), 1);
}

#[test]
fn on_conversation_deleted_out_of_bounds_is_noop() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversations(test_conversations());
    s.on_conversation_deleted(99);
    assert_eq!(s.conversations().len(), 2);
}

#[test]
fn on_conversation_deleted_clears_state_when_active() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversations(test_conversations());
    s.on_select_conversation(0);
    s.on_conversation_id("conv-1");
    s.on_messages(vec![ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: serde_json::Value::String("hi".into()),
        conversation_id: None,
    }]);
    // Delete the active conversation
    s.on_conversation_deleted(0);
    assert_eq!(s.selected_conversation(), None);
    assert_eq!(s.conversation_id(), None);
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert!(messages.is_empty());
    }
}

#[test]
fn on_conversation_deleted_shifts_selection_down() {
    let mut s = make_connected_with_agent_selected();
    s.on_conversations(test_conversations());
    // Select second conversation (idx 1)
    s.on_select_conversation(1);
    assert_eq!(s.selected_conversation(), Some(1));
    // Delete first conversation (idx 0) — selection should shift to 0
    s.on_conversation_deleted(0);
    assert_eq!(s.selected_conversation(), Some(0));
    assert_eq!(s.conversations().len(), 1);
}

#[test]
fn on_select_agent_clears_conversations() {
    let mut s = make_connected();
    s.on_select_agent(0);
    s.on_conversations(test_conversations());
    s.on_select_conversation(0);
    // Now switch agent — should clear conversations.
    s.on_select_agent(1);
    assert!(s.conversations().is_empty());
    assert_eq!(s.selected_conversation(), None);
}

// ── Pagination tests ────────────────────────────────────────────

#[test]
fn on_messages_paged_stores_has_more() {
    let mut s = make_connected_with_agent_selected();
    let msgs = vec![ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: serde_json::Value::String("hi".into()),
        conversation_id: None,
    }];
    s.on_messages_paged(msgs, true);
    assert!(s.has_more_messages());
    assert_eq!(s.message_count(), 1);
}

#[test]
fn on_messages_paged_no_more() {
    let mut s = make_connected_with_agent_selected();
    s.on_messages_paged(vec![], false);
    assert!(!s.has_more_messages());
    assert_eq!(s.message_count(), 0);
}

#[test]
fn on_prepend_messages_adds_to_front() {
    let mut s = make_connected_with_agent_selected();
    let recent = vec![ChatMessage {
        id: "m2".into(),
        role: "assistant".into(),
        content: serde_json::Value::String("hello".into()),
        conversation_id: None,
    }];
    s.on_messages_paged(recent, true);

    let older = vec![ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: serde_json::Value::String("hi".into()),
        conversation_id: None,
    }];
    s.on_prepend_messages(older, false);

    assert_eq!(s.message_count(), 2);
    assert!(!s.has_more_messages());
    // The older message should be first.
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession { messages, .. } = &**session;
        assert_eq!(messages[0].id, "m1");
        assert_eq!(messages[1].id, "m2");
    }
}

#[test]
fn parse_messages_paged_with_has_more() {
    let body = r#"{"messages":[
        {"id":"m1","role":"user","content":"hi","conversation_id":null}
    ],"has_more":true,"query":""}"#;
    let (msgs, has_more) = crate::api::parse_messages_paged(200, body).unwrap();
    assert_eq!(msgs.len(), 1);
    assert!(has_more);
}

#[test]
fn parse_messages_paged_without_has_more() {
    let body = r#"{"messages":[],"query":""}"#;
    let (msgs, has_more) = crate::api::parse_messages_paged(200, body).unwrap();
    assert!(msgs.is_empty());
    assert!(!has_more); // defaults to false when missing
}

// ── Palette (M15) ──────────────────────────────────────────────

fn connected_session() -> SessionState {
    let mut s = SessionState::start("http://localhost:8080", "tok");
    s.on_health(test_health());
    s.on_agents(test_agents());
    s
}

#[test]
fn palette_starts_closed() {
    let s = connected_session();
    assert!(!s.is_palette_open());
    assert!(s.selected_palette_cmd().is_none());
}

#[test]
fn palette_open_and_close() {
    let mut s = connected_session();
    s.open_palette("");
    assert!(s.is_palette_open());
    s.close_palette();
    assert!(!s.is_palette_open());
}

#[test]
fn palette_open_preserves_initial_input() {
    let mut s = connected_session();
    s.open_palette("hel");
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            palette_input,
            palette_selection,
            ..
        } = &**session;
        assert_eq!(palette_input, "hel");
        assert_eq!(*palette_selection, 0);
    } else {
        panic!("not connected");
    }
}

#[test]
fn palette_close_resets_input_and_selection() {
    let mut s = connected_session();
    s.open_palette("mem");
    s.move_palette_selection(1);
    s.close_palette();
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            palette_input,
            palette_selection,
            palette_open,
            ..
        } = &**session;
        assert!(!*palette_open);
        assert!(palette_input.is_empty());
        assert_eq!(*palette_selection, 0);
    } else {
        panic!("not connected");
    }
}

#[test]
fn palette_set_input_resets_selection() {
    let mut s = connected_session();
    s.open_palette("");
    s.move_palette_selection(3);
    s.set_palette_input("hel"); // typing new query — selection back to 0
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            palette_input,
            palette_selection,
            ..
        } = &**session;
        assert_eq!(palette_input, "hel");
        assert_eq!(*palette_selection, 0);
    } else {
        panic!("not connected");
    }
}

#[test]
fn palette_move_selection_clamps_to_bounds() {
    let mut s = connected_session();
    s.open_palette(""); // empty query → all commands
    s.move_palette_selection(-1); // can't go below 0
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            palette_selection, ..
        } = &**session;
        assert_eq!(*palette_selection, 0);
    } else {
        panic!("not connected");
    }

    // Move down past end should clamp.
    for _ in 0..100 {
        s.move_palette_selection(1);
    }
    let filtered_count = crate::palette::fuzzy_filter("").len();
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            palette_selection, ..
        } = &**session;
        assert_eq!(*palette_selection, filtered_count - 1);
    } else {
        panic!("not connected");
    }
}

#[test]
fn palette_selected_cmd_returns_first_match() {
    let mut s = connected_session();
    s.open_palette("help");
    // `help` is an exact trigger — first filtered entry should be Help.
    assert_eq!(
        s.selected_palette_cmd(),
        Some(crate::palette::PaletteCmd::Help)
    );
}

#[test]
fn palette_selected_cmd_respects_selection_index() {
    let mut s = connected_session();
    s.open_palette(""); // all entries
    s.move_palette_selection(1);
    // The second entry's trigger should be the one returned.
    let filtered = crate::palette::fuzzy_filter("");
    let expected = crate::palette::parse_palette_input(filtered[1].def.trigger);
    assert_eq!(s.selected_palette_cmd(), Some(expected));
}

#[test]
fn palette_selected_cmd_none_when_closed() {
    let s = connected_session();
    assert!(s.selected_palette_cmd().is_none());
}

#[test]
fn palette_selected_cmd_none_when_no_matches() {
    let mut s = connected_session();
    s.open_palette("zzznonexistentquery");
    assert!(s.selected_palette_cmd().is_none());
}

#[test]
fn palette_methods_noop_when_not_connected() {
    let mut s = SessionState::start("http://localhost:8080", "tok");
    // Still in Connecting — all palette methods should be no-ops.
    s.open_palette("foo");
    assert!(!s.is_palette_open());
    s.set_palette_input("bar");
    s.move_palette_selection(5);
    s.close_palette();
    assert!(s.selected_palette_cmd().is_none());
}

#[test]
fn clear_timeline_local_clears_messages_only() {
    let mut s = connected_session();
    s.on_select_agent(0);
    s.on_messages(vec![ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: serde_json::Value::String("hi".into()),
        conversation_id: Some("c1".into()),
    }]);
    // Set a conversation_id to verify it's NOT cleared.
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession {
            conversation_id, ..
        } = &mut **session;
        *conversation_id = Some("c1".into());
    }
    s.clear_timeline_local();
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            messages,
            conversation_id,
            ..
        } = &**session;
        assert!(messages.is_empty());
        assert_eq!(conversation_id.as_deref(), Some("c1")); // preserved
    } else {
        panic!("not connected");
    }
}

#[test]
fn last_assistant_content_finds_most_recent() {
    let mut s = connected_session();
    s.on_select_agent(0);
    s.on_messages(vec![
        ChatMessage {
            id: "m1".into(),
            role: "user".into(),
            content: serde_json::Value::String("q1".into()),
            conversation_id: None,
        },
        ChatMessage {
            id: "m2".into(),
            role: "assistant".into(),
            content: serde_json::Value::String("a1".into()),
            conversation_id: None,
        },
        ChatMessage {
            id: "m3".into(),
            role: "user".into(),
            content: serde_json::Value::String("q2".into()),
            conversation_id: None,
        },
        ChatMessage {
            id: "m4".into(),
            role: "assistant".into(),
            content: serde_json::Value::String("a2 final".into()),
            conversation_id: None,
        },
    ]);
    assert_eq!(s.last_assistant_content().as_deref(), Some("a2 final"));
}

#[test]
fn last_assistant_content_none_when_no_assistant_messages() {
    let mut s = connected_session();
    s.on_select_agent(0);
    s.on_messages(vec![ChatMessage {
        id: "m1".into(),
        role: "user".into(),
        content: serde_json::Value::String("hi".into()),
        conversation_id: None,
    }]);
    assert!(s.last_assistant_content().is_none());
}

// ── Memory overlay (M16) ───────────────────────────────────────

fn test_blocks() -> Vec<crate::api::MemoryBlock> {
    vec![
        crate::api::MemoryBlock {
            label: "human".into(),
            value: "User loves Rust".into(),
            description: Some("User info".into()),
            tier: Some("short".into()),
        },
        crate::api::MemoryBlock {
            label: "project".into(),
            value: "CADE project".into(),
            description: None,
            tier: None,
        },
    ]
}

#[test]
fn memory_starts_closed() {
    let s = connected_session();
    assert!(!s.is_memory_open());
}

#[test]
fn open_memory_sets_flags() {
    let mut s = connected_session();
    s.open_memory_overlay();
    assert!(s.is_memory_open());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            memory_loading,
            memory_error,
            ..
        } = &**session;
        assert!(*memory_loading);
        assert!(memory_error.is_none());
    } else {
        panic!("not connected");
    }
}

#[test]
fn close_memory_resets_transient_flags() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_error("boom");
    assert_eq!(
        match &s {
            SessionState::Connected(session) => session.memory_error.clone(),
            _ => None,
        },
        Some("boom".to_string())
    );
    s.close_memory_overlay();
    assert!(!s.is_memory_open());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            memory_error,
            memory_saving,
            ..
        } = &**session;
        assert!(memory_error.is_none());
        assert!(!*memory_saving);
    } else {
        panic!("not connected");
    }
}

#[test]
fn memory_loaded_seeds_edit_buffer_with_first_block() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            memory_loading,
            ..
        } = &**session;
        assert_eq!(memory_blocks.len(), 2);
        assert_eq!(*memory_selection, 0);
        assert_eq!(memory_edit_buffer, "User loves Rust");
        assert!(!*memory_loading);
    } else {
        panic!("not connected");
    }
}

#[test]
fn memory_loaded_with_empty_list_keeps_empty_buffer() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(Vec::new());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            memory_blocks,
            memory_edit_buffer,
            memory_loading,
            ..
        } = &**session;
        assert!(memory_blocks.is_empty());
        assert!(memory_edit_buffer.is_empty());
        assert!(!*memory_loading);
    } else {
        panic!("not connected");
    }
}

#[test]
fn memory_error_clears_loading_and_saving() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_save_start();
    s.on_memory_error("nope");
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            memory_loading,
            memory_saving,
            memory_error,
            ..
        } = &**session;
        assert!(!*memory_loading);
        assert!(!*memory_saving);
        assert_eq!(memory_error.as_deref(), Some("nope"));
    } else {
        panic!("not connected");
    }
}

#[test]
fn select_memory_block_updates_buffer() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    // Edit the buffer — this simulates the user typing.
    s.set_memory_edit_buffer("unsaved edit");
    let changed = s.select_memory_block(1);
    assert!(changed);
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            memory_selection,
            memory_edit_buffer,
            ..
        } = &**session;
        assert_eq!(*memory_selection, 1);
        // Buffer is reset to the new block's value — unsaved edit is lost.
        assert_eq!(memory_edit_buffer, "CADE project");
    } else {
        panic!("not connected");
    }
}

#[test]
fn select_memory_block_same_index_returns_false() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    assert!(!s.select_memory_block(0));
}

#[test]
fn select_memory_block_out_of_bounds_returns_false() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    assert!(!s.select_memory_block(99));
}

#[test]
fn memory_save_ok_persists_buffer_into_block() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.set_memory_edit_buffer("User loves Rust AND Python");
    s.on_memory_save_start();
    s.on_memory_save_ok();
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            memory_blocks,
            memory_saving,
            memory_error,
            ..
        } = &**session;
        assert_eq!(memory_blocks[0].value, "User loves Rust AND Python");
        assert!(!*memory_saving);
        assert!(memory_error.is_none());
    } else {
        panic!("not connected");
    }
}

#[test]
fn memory_selected_label_value_returns_current() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.set_memory_edit_buffer("new content");
    assert_eq!(
        s.memory_selected_label_value(),
        Some(("human".to_string(), "new content".to_string()))
    );
}

#[test]
fn memory_selected_label_value_none_when_closed() {
    let mut s = connected_session();
    s.on_memory_loaded(test_blocks()); // noop because overlay closed
    assert!(s.memory_selected_label_value().is_none());
}

#[test]
fn memory_methods_noop_when_not_connected() {
    let mut s = SessionState::start("http://localhost:8080", "tok");
    // Still in Connecting — all memory methods should be no-ops.
    s.open_memory_overlay();
    assert!(!s.is_memory_open());
    s.on_memory_loaded(test_blocks());
    s.on_memory_error("nope");
    s.set_memory_edit_buffer("x");
    s.on_memory_save_start();
    s.on_memory_save_ok();
    s.close_memory_overlay();
    assert!(s.memory_selected_label_value().is_none());
    assert!(!s.select_memory_block(0));
}

// ── is_memory_dirty / memory_save_notice ──────────────────────

#[test]
fn is_memory_dirty_false_when_closed() {
    let s = connected_session();
    assert!(!s.is_memory_dirty());
}

#[test]
fn is_memory_dirty_false_right_after_load() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    assert!(
        !s.is_memory_dirty(),
        "fresh load should have buffer == block value, not dirty"
    );
}

#[test]
fn is_memory_dirty_true_after_edit() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.set_memory_edit_buffer("something different");
    assert!(s.is_memory_dirty());
}

#[test]
fn is_memory_dirty_false_after_save() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.set_memory_edit_buffer("edited");
    assert!(s.is_memory_dirty());
    s.on_memory_save_start();
    s.on_memory_save_ok();
    assert!(
        !s.is_memory_dirty(),
        "after save the block's saved value == buffer, no longer dirty"
    );
}

#[test]
fn is_memory_dirty_false_after_selecting_different_block() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.set_memory_edit_buffer("dirty here");
    assert!(s.is_memory_dirty());
    // Selecting another block seeds the buffer with its saved value,
    // so dirty should flip back to false.
    assert!(s.select_memory_block(1));
    assert!(!s.is_memory_dirty());
}

#[test]
fn memory_save_notice_none_by_default() {
    let s = connected_session();
    assert!(s.memory_save_notice().is_none());
}

#[test]
fn memory_save_notice_set_on_save_ok() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.set_memory_edit_buffer("new val");
    s.on_memory_save_start();
    s.on_memory_save_ok();
    assert_eq!(s.memory_save_notice(), Some("Saved /human"));
}

#[test]
fn memory_save_notice_cleared_on_select() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.on_memory_save_start();
    s.on_memory_save_ok();
    assert!(s.memory_save_notice().is_some());
    assert!(s.select_memory_block(1));
    assert!(s.memory_save_notice().is_none());
}

#[test]
fn memory_save_notice_cleared_on_close() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.on_memory_save_start();
    s.on_memory_save_ok();
    assert!(s.memory_save_notice().is_some());
    s.close_memory_overlay();
    assert!(s.memory_save_notice().is_none());
}

#[test]
fn memory_save_notice_cleared_on_error() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.on_memory_save_start();
    s.on_memory_save_ok();
    assert!(s.memory_save_notice().is_some());
    s.on_memory_error("boom");
    assert!(s.memory_save_notice().is_none());
}

#[test]
fn memory_save_notice_cleared_on_save_start() {
    let mut s = connected_session();
    s.open_memory_overlay();
    s.on_memory_loaded(test_blocks());
    s.on_memory_save_start();
    s.on_memory_save_ok();
    assert!(s.memory_save_notice().is_some());
    // A second save begins — should clear the previous notice.
    s.on_memory_save_start();
    assert!(s.memory_save_notice().is_none());
}

// ── refresh_agents ─────────────────────────────────────────────

#[test]
fn refresh_agents_preserves_selection_by_id() {
    let mut s = connected_session();
    s.on_select_agent(1); // pick second agent
    let selected_id = s.selected_agent_id().unwrap().to_string();

    // Simulate server returning a reordered list with an extra agent.
    let mut new_agents = test_agents();
    new_agents.reverse();
    new_agents.push(AgentInfo {
        id: "agent-3".into(),
        name: "New Agent".into(),
        model: None,
        provider: None,
        theme: None,
    });
    s.refresh_agents(new_agents);
    // Selection should follow the id, so it's still the same agent.
    assert_eq!(s.selected_agent_id(), Some(selected_id.as_str()));
}

#[test]
fn refresh_agents_drops_selection_when_agent_removed() {
    let mut s = connected_session();
    s.on_select_agent(0);
    let new_agents = vec![AgentInfo {
        id: "different-agent".into(),
        name: "Different".into(),
        model: None,
        provider: None,
        theme: None,
    }];
    s.refresh_agents(new_agents);
    assert!(s.selected_agent_id().is_none());
}

#[test]
fn refresh_agents_noop_when_not_connected() {
    let mut s = SessionState::start("http://localhost:8080", "tok");
    s.refresh_agents(test_agents());
    // Still Connecting — no panic, no transition.
    assert!(!s.is_connected());
}

// ── Checkpoints overlay (M17) ──────────────────────────────────

fn test_checkpoint_rows() -> Vec<crate::api::CheckpointRow> {
    vec![
        crate::api::CheckpointRow {
            id: "cp-1".into(),
            agent_id: "agent-1".into(),
            conversation_id: None,
            branch_id: "main".into(),
            label: Some("before-refactor".into()),
            description: None,
            created_at: 1_700_000_000,
            git_commit_hash: Some("hash123".into()),
            parent_id: None,
        },
        crate::api::CheckpointRow {
            id: "cp-2".into(),
            agent_id: "agent-1".into(),
            conversation_id: None,
            branch_id: "main".into(),
            label: None,
            description: Some("auto-save".into()),
            created_at: 1_700_001_000,
            git_commit_hash: None,
            parent_id: Some("cp-1".into()),
        },
    ]
}

#[test]
fn checkpoints_starts_closed() {
    let s = connected_session();
    assert!(!s.is_checkpoints_open());
    assert!(s.checkpoints_snapshot().is_empty());
}

#[test]
fn open_checkpoints_sets_loading_and_clears_error() {
    let mut s = connected_session();
    s.on_checkpoints_error("stale");
    s.open_checkpoints_overlay();
    assert!(s.is_checkpoints_open());
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                checkpoints_loading,
                checkpoints_error,
                ..
            } = &**session;

            assert!(*checkpoints_loading);
            assert!(checkpoints_error.is_none());
        }
        _ => panic!("not connected"),
    }
}

#[test]
fn checkpoints_loaded_populates_list() {
    let mut s = connected_session();
    s.open_checkpoints_overlay();
    s.on_checkpoints_loaded(test_checkpoint_rows());
    assert_eq!(s.checkpoints_snapshot().len(), 2);
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                checkpoints_loading,
                ..
            } = &**session;
            assert!(!*checkpoints_loading);
        }
        _ => panic!(),
    }
}

#[test]
fn checkpoints_error_clears_loading_and_busy() {
    let mut s = connected_session();
    s.open_checkpoints_overlay();
    s.on_checkpoints_action_start();
    s.on_checkpoints_error("network down");
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                checkpoints_loading,
                checkpoints_busy,
                checkpoints_error,
                ..
            } = &**session;

            assert!(!*checkpoints_loading);
            assert!(!*checkpoints_busy);
            assert_eq!(checkpoints_error.as_deref(), Some("network down"));
        }
        _ => panic!(),
    }
}

#[test]
fn checkpoints_action_ok_sets_notice() {
    let mut s = connected_session();
    s.open_checkpoints_overlay();
    s.on_checkpoints_action_start();
    s.on_checkpoints_action_ok("Restored cp-1");
    assert_eq!(s.checkpoints_notice(), Some("Restored cp-1"));
}

#[test]
fn checkpoints_notice_cleared_on_new_action() {
    let mut s = connected_session();
    s.on_checkpoints_action_ok("Done");
    s.on_checkpoints_action_start();
    assert!(s.checkpoints_notice().is_none());
}

#[test]
fn checkpoints_notice_cleared_on_close() {
    let mut s = connected_session();
    s.on_checkpoints_action_ok("Done");
    s.close_checkpoints_overlay();
    assert!(s.checkpoints_notice().is_none());
}

#[test]
fn checkpoints_methods_noop_when_not_connected() {
    let mut s = SessionState::start("http://localhost:8080", "tok");
    s.open_checkpoints_overlay();
    assert!(!s.is_checkpoints_open());
    s.on_checkpoints_loaded(test_checkpoint_rows());
    assert!(s.checkpoints_snapshot().is_empty());
    s.on_checkpoints_error("x");
    s.on_checkpoints_action_start();
    s.on_checkpoints_action_ok("x");
    s.close_checkpoints_overlay();
}

// ── Artifacts overlay (M17) ────────────────────────────────────

fn test_artifact_rows() -> Vec<crate::api::ArtifactInfo> {
    vec![
        crate::api::ArtifactInfo {
            id: "art-1".into(),
            kind: "log".into(),
            content_type: "text/plain".into(),
            size_bytes: 42,
            created_at: 1_700_000_000,
            run_id: Some("run-1".into()),
        },
        crate::api::ArtifactInfo {
            id: "art-2".into(),
            kind: "diff".into(),
            content_type: "text/x-diff".into(),
            size_bytes: 128,
            created_at: 1_700_001_000,
            run_id: None,
        },
    ]
}

fn test_artifact_detail(id: &str) -> crate::api::ArtifactDetail {
    crate::api::ArtifactDetail {
        id: id.into(),
        kind: "log".into(),
        content_type: "text/plain".into(),
        data_text: Some("hello".into()),
        metadata: serde_json::json!({}),
        size_bytes: 5,
        created_at: 1_700_000_000,
    }
}

#[test]
fn artifacts_starts_closed() {
    let s = connected_session();
    assert!(!s.is_artifacts_open());
    assert!(s.artifacts_snapshot().is_empty());
    assert!(s.artifact_detail().is_none());
}

#[test]
fn open_artifacts_clears_selection() {
    let mut s = connected_session();
    s.open_artifacts_overlay();
    s.on_artifacts_loaded(test_artifact_rows());
    s.select_artifact(0);
    s.on_artifact_detail_loaded(test_artifact_detail("art-1"));
    // Reopening (e.g. via palette) should reset selection.
    s.open_artifacts_overlay();
    assert!(s.selected_artifact_id().is_none());
    assert!(s.artifact_detail().is_none());
}

#[test]
fn artifacts_loaded_populates_list() {
    let mut s = connected_session();
    s.open_artifacts_overlay();
    s.on_artifacts_loaded(test_artifact_rows());
    assert_eq!(s.artifacts_snapshot().len(), 2);
}

#[test]
fn select_artifact_returns_id_and_sets_busy() {
    let mut s = connected_session();
    s.open_artifacts_overlay();
    s.on_artifacts_loaded(test_artifact_rows());
    let id = s.select_artifact(1);
    assert_eq!(id.as_deref(), Some("art-2"));
    assert_eq!(s.selected_artifact_id().as_deref(), Some("art-2"));
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession { artifacts_busy, .. } = &**session;
            assert!(*artifacts_busy);
        }
        _ => panic!(),
    }
}

#[test]
fn select_artifact_out_of_bounds_returns_none() {
    let mut s = connected_session();
    s.open_artifacts_overlay();
    s.on_artifacts_loaded(test_artifact_rows());
    assert!(s.select_artifact(99).is_none());
    assert!(s.selected_artifact_id().is_none());
}

#[test]
fn artifact_detail_loaded_clears_busy() {
    let mut s = connected_session();
    s.open_artifacts_overlay();
    s.on_artifacts_loaded(test_artifact_rows());
    s.select_artifact(0);
    s.on_artifact_detail_loaded(test_artifact_detail("art-1"));
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession { artifacts_busy, .. } = &**session;
            assert!(!*artifacts_busy);
        }
        _ => panic!(),
    }
    assert_eq!(s.artifact_detail().map(|d| d.id.as_str()), Some("art-1"));
}

#[test]
fn artifacts_error_clears_busy_and_loading() {
    let mut s = connected_session();
    s.open_artifacts_overlay();
    s.on_artifacts_action_start();
    s.on_artifacts_error("oops");
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                artifacts_loading,
                artifacts_busy,
                artifacts_error,
                ..
            } = &**session;

            assert!(!*artifacts_loading);
            assert!(!*artifacts_busy);
            assert_eq!(artifacts_error.as_deref(), Some("oops"));
        }
        _ => panic!(),
    }
}

#[test]
fn artifacts_methods_noop_when_not_connected() {
    let mut s = SessionState::start("http://localhost:8080", "tok");
    s.open_artifacts_overlay();
    assert!(!s.is_artifacts_open());
    s.on_artifacts_loaded(test_artifact_rows());
    assert!(s.artifacts_snapshot().is_empty());
    assert!(s.select_artifact(0).is_none());
    s.on_artifact_detail_loaded(test_artifact_detail("x"));
    s.on_artifacts_error("x");
    s.close_artifacts_overlay();
}

// ── Tools overlay (M18) ────────────────────────────────────────

#[test]
fn tools_starts_closed() {
    let s = connected_session();
    assert!(!s.is_tools_open());
    assert!(s.tools_snapshot().is_empty());
}

#[test]
fn open_tools_sets_loading() {
    let mut s = connected_session();
    s.open_tools_overlay();
    assert!(s.is_tools_open());
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession { tools_loading, .. } = &**session;
            assert!(*tools_loading);
        }
        _ => panic!(),
    }
}

#[test]
fn tools_loaded_populates_list() {
    let mut s = connected_session();
    s.open_tools_overlay();
    s.on_tools_loaded(vec![
        crate::api::AgentTool {
            id: "t1".into(),
            name: "bash".into(),
        },
        crate::api::AgentTool {
            id: "t2".into(),
            name: "read_file".into(),
        },
    ]);
    assert_eq!(s.tools_snapshot().len(), 2);
}

#[test]
fn tools_error_clears_loading() {
    let mut s = connected_session();
    s.open_tools_overlay();
    s.on_tools_error("net error");
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                tools_loading,
                tools_error,
                ..
            } = &**session;

            assert!(!*tools_loading);
            assert_eq!(tools_error.as_deref(), Some("net error"));
        }
        _ => panic!(),
    }
}

#[test]
fn tools_methods_noop_when_not_connected() {
    let mut s = SessionState::start("http://localhost", "tok");
    s.open_tools_overlay();
    assert!(!s.is_tools_open());
    s.on_tools_loaded(vec![]);
    s.on_tools_error("x");
    s.close_tools_overlay();
}

// ── Question widget (M18) ──────────────────────────────────────

fn test_question() -> crate::api::Question {
    crate::api::Question {
        header: "Choose".into(),
        question: "Pick one".into(),
        options: vec![
            crate::api::QuestionOption {
                label: "A".into(),
                description: "Alpha".into(),
            },
            crate::api::QuestionOption {
                label: "B".into(),
                description: "Beta".into(),
            },
            crate::api::QuestionOption {
                label: "C".into(),
                description: "Gamma".into(),
            },
        ],
        multi_select: false,
    }
}

#[test]
fn no_active_question_initially() {
    let s = connected_session();
    assert!(!s.has_active_question());
    assert!(s.active_question().is_none());
}

#[test]
fn set_active_question_initialises_cursor() {
    let mut s = connected_session();
    s.set_active_question(test_question());
    assert!(s.has_active_question());
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                question_cursor, ..
            } = &**session;
            assert_eq!(*question_cursor, 0);
        }
        _ => panic!(),
    }
}

#[test]
fn move_question_cursor_wraps() {
    let mut s = connected_session();
    s.set_active_question(test_question());
    s.move_question_cursor(-1); // 0 - 1 wraps to 2 (3 options)
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                question_cursor, ..
            } = &**session;
            assert_eq!(*question_cursor, 2);
        }
        _ => panic!(),
    }
    s.move_question_cursor(1);
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                question_cursor, ..
            } = &**session;
            assert_eq!(*question_cursor, 0);
        }
        _ => panic!(),
    }
}

#[test]
fn commit_question_answer_single_select() {
    let mut s = connected_session();
    s.set_active_question(test_question());
    s.move_question_cursor(1); // cursor at index 1 = "B"
    let answer = s.commit_question_answer();
    assert_eq!(answer.as_deref(), Some("B"));
}

#[test]
fn commit_question_answer_multi_select() {
    let mut s = connected_session();
    let mut q = test_question();
    q.multi_select = true;
    s.set_active_question(q);
    // Check options 0 and 2
    s.toggle_question_checked(); // cursor=0, check A
    s.move_question_cursor(1);
    s.move_question_cursor(1); // cursor=2
    s.toggle_question_checked(); // check C
    let answer = s.commit_question_answer();
    assert_eq!(answer.as_deref(), Some("A, C"));
}

#[test]
fn commit_question_multi_select_none_checked_returns_none() {
    let mut s = connected_session();
    let mut q = test_question();
    q.multi_select = true;
    s.set_active_question(q);
    assert!(s.commit_question_answer().is_none());
}

#[test]
fn clear_active_question_removes_it() {
    let mut s = connected_session();
    s.set_active_question(test_question());
    s.clear_active_question();
    assert!(!s.has_active_question());
}

#[test]
fn on_stream_tool_call_sets_question_for_ask_user_question() {
    let mut s = connected_session();
    s.on_select_agent(0);
    // Seed input buffer then send to enter streaming state
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "hello".to_string();
    }
    s.on_send().unwrap();
    let args = r#"{"questions":[{
        "header":"Auth","question":"Which?",
        "options":[{"label":"JWT","description":""},{"label":"Sessions","description":""}],
        "multiSelect":false
    }]}"#;
    s.on_stream_tool_call("tc-1", "ask_user_question", args);
    assert!(s.has_active_question());
    assert_eq!(s.active_question().map(|q| q.header.as_str()), Some("Auth"));
}

#[test]
fn on_stream_tool_call_non_question_does_not_set_widget() {
    let mut s = connected_session();
    s.on_select_agent(0);
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "hello".to_string();
    }
    s.on_send().unwrap();
    s.on_stream_tool_call("tc-1", "bash", r#"{"command":"ls"}"#);
    assert!(!s.has_active_question());
}

// ── Metrics (M19 item 2) ───────────────────────────────────────

#[test]
fn metrics_none_initially() {
    let s = connected_session();
    assert!(s.agent_metrics().is_none());
}

#[test]
fn on_metrics_loaded_stores_value() {
    let mut s = connected_session();
    s.on_metrics_loaded(crate::api::AgentMetrics {
        consolidation_runs: 5,
        ..Default::default()
    });
    assert_eq!(s.agent_metrics().map(|m| m.consolidation_runs), Some(5));
}

#[test]
fn metrics_noop_when_not_connected() {
    let mut s = SessionState::start("http://localhost", "tok");
    s.on_metrics_loaded(crate::api::AgentMetrics::default());
    assert!(s.agent_metrics().is_none());
}

// ── Cumulative token totals (M19 item 3) ──────────────────────

#[test]
fn total_tokens_zero_initially() {
    let s = connected_session();
    assert_eq!(s.total_token_usage(), (0, 0));
}

#[test]
fn total_tokens_accumulate_across_turns() {
    let mut s = connected_session();
    s.on_usage(100, 50, None);
    s.on_usage(200, 80, None);
    assert_eq!(s.total_token_usage(), (300, 130));
}

// ── Context overlay (M19 item 3) ──────────────────────────────

#[test]
fn context_starts_closed() {
    let s = connected_session();
    assert!(!s.is_context_open());
    assert!(s.context_stats().is_none());
}

#[test]
fn open_context_sets_loading() {
    let mut s = connected_session();
    s.open_context_overlay();
    assert!(s.is_context_open());
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                context_loading, ..
            } = &**session;
            assert!(*context_loading);
        }
        _ => panic!(),
    }
}

#[test]
fn context_loaded_stores_stats() {
    let mut s = connected_session();
    s.open_context_overlay();
    s.on_context_loaded(crate::api::ContextStats {
        window_tokens: 128000,
        ..Default::default()
    });
    assert_eq!(s.context_stats().map(|c| c.window_tokens), Some(128000));
}

#[test]
fn context_error_clears_loading() {
    let mut s = connected_session();
    s.open_context_overlay();
    s.on_context_error("timeout");
    match &s {
        SessionState::Connected(session) => {
            let crate::session::ConnectedSession {
                context_loading,
                context_error,
                ..
            } = &**session;

            assert!(!*context_loading);
            assert_eq!(context_error.as_deref(), Some("timeout"));
        }
        _ => panic!(),
    }
}

// ── Agents + stats overlays (M19 item 3) ──────────────────────

#[test]
fn agents_overlay_open_close() {
    let mut s = connected_session();
    assert!(!s.is_agents_open());
    s.open_agents_overlay();
    assert!(s.is_agents_open());
    s.close_agents_overlay();
    assert!(!s.is_agents_open());
}

#[test]
fn stats_overlay_open_close() {
    let mut s = connected_session();
    assert!(!s.is_stats_open());
    s.open_stats_overlay();
    assert!(s.is_stats_open());
    s.close_stats_overlay();
    assert!(!s.is_stats_open());
}

// ── Model picker tests ───────────────────────────────────────────

fn sample_models() -> Vec<crate::api::ModelInfo> {
    vec![
        crate::api::ModelInfo {
            provider: "anthropic".into(),
            id: "claude-3-5-sonnet".into(),
            display_name: "Claude 3.5 Sonnet".into(),
            context_window: 200_000,
        },
        crate::api::ModelInfo {
            provider: "openai".into(),
            id: "gpt-4o".into(),
            display_name: "GPT-4o".into(),
            context_window: 128_000,
        },
        crate::api::ModelInfo {
            provider: "anthropic".into(),
            id: "claude-3-haiku".into(),
            display_name: "Claude 3 Haiku".into(),
            context_window: 200_000,
        },
    ]
}

#[test]
fn model_picker_open_close() {
    let mut s = connected_session();
    assert!(!s.is_model_picker_open());
    s.open_model_picker();
    assert!(s.is_model_picker_open());
    s.close_model_picker();
    assert!(!s.is_model_picker_open());
}

#[test]
fn model_picker_loads_models() {
    let mut s = connected_session();
    s.open_model_picker();
    s.on_models_loaded(sample_models(), vec!["custom-local".into()]);
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            model_picker_models,
            model_picker_custom_providers,
            model_picker_loading,
            ..
        } = &**session;
        assert_eq!(model_picker_models.len(), 3);
        assert_eq!(model_picker_custom_providers, &["custom-local"]);
        assert!(!model_picker_loading);
    } else {
        panic!("expected Connected");
    }
}

#[test]
fn model_picker_error_state() {
    let mut s = connected_session();
    s.open_model_picker();
    s.on_models_error("network error".into());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            model_picker_loading,
            model_picker_error,
            ..
        } = &**session;
        assert!(!model_picker_loading);
        assert_eq!(model_picker_error.as_deref(), Some("network error"));
    } else {
        panic!("expected Connected");
    }
}

#[test]
fn model_picker_query_resets_selection() {
    let mut s = connected_session();
    s.open_model_picker();
    s.on_models_loaded(sample_models(), vec![]);
    s.set_model_picker_selection(2);
    s.set_model_picker_query("gpt".into());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            model_picker_selection,
            model_picker_query,
            ..
        } = &**session;
        assert_eq!(*model_picker_selection, 0);
        assert_eq!(model_picker_query, "gpt");
    } else {
        panic!("expected Connected");
    }
}

#[test]
fn filter_models_matches_id_provider_display() {
    let models = sample_models();
    assert_eq!(super::filter_models(&models, "").len(), 3);
    assert_eq!(super::filter_models(&models, "claude").len(), 2);
    assert_eq!(super::filter_models(&models, "openai").len(), 1);
    assert_eq!(super::filter_models(&models, "GPT").len(), 1);
    assert_eq!(super::filter_models(&models, "haiku").len(), 1);
    assert_eq!(super::filter_models(&models, "xyz").len(), 0);
}

#[test]
fn selected_model_id_returns_correct_id() {
    let mut s = connected_session();
    s.open_model_picker();
    s.on_models_loaded(sample_models(), vec![]);
    assert_eq!(s.selected_model_id(), Some("claude-3-5-sonnet".into()));
    s.set_model_picker_selection(1);
    assert_eq!(s.selected_model_id(), Some("gpt-4o".into()));
}

// ── MCP overlay ─────────────────────────────────────────────────────

fn sample_mcp_servers() -> Vec<crate::api::McpServerInfo> {
    vec![
        crate::api::McpServerInfo {
            key: "desktop-commander".into(),
            command: "npx @desktop-commander/mcp-server".into(),
            tools: vec![
                "desktop-commander__bash".into(),
                "desktop-commander__read_file".into(),
            ],
            disabled: false,
        },
        crate::api::McpServerInfo {
            key: "old-server".into(),
            command: "old-cmd".into(),
            tools: vec![],
            disabled: true,
        },
    ]
}

#[test]
fn mcp_overlay_open_sets_loading() {
    let mut s = connected_session();
    assert!(!s.is_mcp_open());
    s.open_mcp_overlay();
    assert!(s.is_mcp_open());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            mcp_loading,
            mcp_error,
            ..
        } = &**session;
        assert!(mcp_loading);
        assert!(mcp_error.is_none());
    } else {
        panic!("expected Connected");
    }
}

#[test]
fn mcp_overlay_close_resets_state() {
    let mut s = connected_session();
    s.open_mcp_overlay();
    s.close_mcp_overlay();
    assert!(!s.is_mcp_open());
}

#[test]
fn mcp_on_loaded_populates_servers() {
    let mut s = connected_session();
    s.open_mcp_overlay();
    s.on_mcp_loaded(sample_mcp_servers());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            mcp_servers,
            mcp_loading,
            mcp_error,
            ..
        } = &**session;
        assert_eq!(mcp_servers.len(), 2);
        assert_eq!(mcp_servers[0].key, "desktop-commander");
        assert_eq!(mcp_servers[0].tools.len(), 2);
        assert!(mcp_servers[1].disabled);
        assert!(!mcp_loading);
        assert!(mcp_error.is_none());
    } else {
        panic!("expected Connected");
    }
}

#[test]
fn mcp_on_error_sets_message() {
    let mut s = connected_session();
    s.open_mcp_overlay();
    s.on_mcp_error("connection refused".into());
    if let SessionState::Connected(session) = &s {
        let crate::session::ConnectedSession {
            mcp_loading,
            mcp_error,
            ..
        } = &**session;
        assert!(!mcp_loading);
        assert_eq!(mcp_error.as_deref(), Some("connection refused"));
    } else {
        panic!("expected Connected");
    }
}

// ── Plan panel tests ──────────────────────────────────────────

#[test]
fn no_active_plan_initially() {
    let s = connected_session();
    assert!(s.active_plan().is_none());
}

#[test]
fn set_plan_creates_steps() {
    let mut s = connected_session();
    s.set_plan(
        "Tasks".to_string(),
        vec!["Step 1".into(), "Step 2".into(), "Step 3".into()],
    );
    let plan = s.active_plan().unwrap();
    assert_eq!(plan.steps.len(), 3);
    assert!(plan.is_visible);
    assert_eq!(plan.steps[0].id, 1);
    assert_eq!(plan.steps[0].description, "Step 1");
    assert!(!plan.steps[0].is_done);
    assert_eq!(plan.steps[2].id, 3);
}

#[test]
fn set_plan_empty_clears() {
    let mut s = connected_session();
    s.set_plan("Tasks".to_string(), vec!["A".into()]);
    assert!(s.active_plan().is_some());
    s.set_plan("Tasks".to_string(), vec![]);
    assert!(s.active_plan().is_none());
}

#[test]
fn update_plan_step_marks_done() {
    let mut s = connected_session();
    s.set_plan("Tasks".to_string(), vec!["A".into(), "B".into()]);
    assert!(s.update_plan_step(1, true));
    let plan = s.active_plan().unwrap();
    assert!(plan.steps[0].is_done);
    assert!(!plan.steps[1].is_done);
}

#[test]
fn update_plan_step_invalid_id_returns_false() {
    let mut s = connected_session();
    s.set_plan("Tasks".to_string(), vec!["A".into()]);
    assert!(!s.update_plan_step(99, true));
}

#[test]
fn on_stream_tool_call_intercepts_set_plan() {
    let mut s = connected_session();
    s.on_select_agent(0);
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "go".to_string();
    }
    s.on_send();
    s.on_stream_tool_call("tc-1", "set_plan", r#"{"steps":["Read","Write","Test"]}"#);
    let plan = s.active_plan().unwrap();
    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].description, "Read");
}

#[test]
fn on_stream_tool_call_intercepts_update_plan() {
    let mut s = connected_session();
    s.on_select_agent(0);
    if let SessionState::Connected(session) = &mut s {
        let crate::session::ConnectedSession { input_buffer, .. } = &mut **session;
        *input_buffer = "go".to_string();
    }
    s.on_send();
    s.on_stream_tool_call("tc-1", "set_plan", r#"{"steps":["A","B"]}"#);
    s.on_stream_tool_call("tc-2", "UpdatePlan", r#"{"step_id":1,"done":true}"#);
    let plan = s.active_plan().unwrap();
    assert!(plan.steps[0].is_done);
    assert!(!plan.steps[1].is_done);
}

// ── Live output tests ─────────────────────────────────────────

#[test]
fn live_outputs_empty_initially() {
    let s = connected_session();
    assert!(s.live_outputs().is_empty());
}

#[test]
fn begin_live_output_creates_block() {
    let mut s = connected_session();
    s.begin_live_output("tc-1", "bash");
    assert_eq!(s.live_outputs().len(), 1);
    assert_eq!(s.live_outputs()[0].call_id, "tc-1");
    assert_eq!(s.live_outputs()[0].tool_name, "bash");
    assert!(!s.live_outputs()[0].done);
}

#[test]
fn append_live_output_adds_lines() {
    let mut s = connected_session();
    s.begin_live_output("tc-1", "bash");
    s.append_live_output("tc-1", "line 1".into());
    s.append_live_output("tc-1", "line 2".into());
    assert_eq!(s.live_outputs()[0].lines.len(), 2);
}

#[test]
fn finish_live_output_marks_done() {
    let mut s = connected_session();
    s.begin_live_output("tc-1", "bash");
    s.finish_live_output("tc-1");
    assert!(s.live_outputs()[0].done);
}

#[test]
fn append_to_unknown_call_id_is_noop() {
    let mut s = connected_session();
    s.begin_live_output("tc-1", "bash");
    s.append_live_output("tc-99", "orphan".into());
    assert!(s.live_outputs()[0].lines.is_empty());
}
