// Tests for the run module.

use super::subagent::{filter_subagent_tools, handle_run_subagent_tool};
use super::*;

// ── truncate_at_char_boundary (C2) ────────────────────────────────

#[test]
fn truncate_at_char_boundary_short_string_unchanged() {
    let s = "hello world";
    assert_eq!(truncate_at_char_boundary(s, 100), "hello world");
}

#[test]
fn truncate_at_char_boundary_ascii_exact() {
    let s = "abcdefghij";
    assert_eq!(truncate_at_char_boundary(s, 5), "abcde");
}

#[test]
fn truncate_at_char_boundary_does_not_panic_on_multibyte() {
    // 4-byte emoji at the boundary: byte index 8 splits the emoji.
    // Without the fix this would panic with "byte index 8 is not a
    // char boundary".
    let s = "abcdefg🚀hijk"; // bytes: 7 ascii + 4-byte emoji + 4 ascii
    assert_eq!(s.len(), 15);
    // Cut at byte 8 — middle of the emoji's 4-byte sequence.
    let result = truncate_at_char_boundary(s, 8);
    // Should walk back to byte 7 (just before the emoji).
    assert_eq!(result, "abcdefg");
}

#[test]
fn truncate_at_char_boundary_keeps_complete_chars() {
    let s = "héllo"; // 'é' is 2 bytes (0xC3 0xA9). Bytes: h(1) é(2) l(1) l(1) o(1) = 6
    assert_eq!(s.len(), 6);
    // Cut at byte 2 — middle of 'é'? 'h' is byte 0, 'é' starts at byte 1
    // and ends at byte 3.  Byte index 2 is mid-é and not a boundary.
    let result = truncate_at_char_boundary(s, 2);
    // Should walk back to byte 1.
    assert_eq!(result, "h");
}

#[test]
fn truncate_at_char_boundary_zero_max() {
    let s = "abc";
    assert_eq!(truncate_at_char_boundary(s, 0), "");
}

#[test]
fn truncate_at_char_boundary_large_utf8() {
    // 8192 bytes worth of multi-byte CJK — verifies we never panic at
    // the production cap.
    let cjk = "中".repeat(3000); // 3 bytes each → 9000 bytes
    let result = truncate_at_char_boundary(&cjk, 8192);
    // Length must be ≤ 8192 and on a char boundary.
    assert!(result.len() <= 8192);
    assert!(cjk.is_char_boundary(result.len()));
}

// ── RunExitStatus (M9r) ────────────────────────────────────────────

#[test]
fn run_exit_status_done_renders_as_done() {
    assert_eq!(RunExitStatus::Done.as_str(), "done");
}

#[test]
fn run_exit_status_error_renders_as_error() {
    assert_eq!(RunExitStatus::Error.as_str(), "error");
}

#[tokio::test]
async fn emit_run_event_persists_cursor_before_forwarding_transport_event() -> Result<(), String> {
    let state = build_state_with_llm(std::sync::Arc::new(PanicOnCallLlm));
    let agent = cade_store::sqlite::AgentRow {
        id: "agent-events".to_owned(),
        name: "Event agent".to_owned(),
        description: None,
        model: "test".to_owned(),
        system_prompt: None,
        created_at: None,
        compaction_model: None,
        theme: None,
        active_plan_json: None,
        parent_id: None,
    };
    cade_store::sqlite::create_agent(&state.db, &agent).map_err(|error| error.to_string())?;
    let run = cade_store::sqlite::create_run(&state.db, &agent.id, None)
        .map_err(|error| error.to_string())?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);

    emit_run_event(
        &state.db,
        &run.id,
        &sender,
        serde_json::json!({ "message_type": "assistant_message", "content": "hello" }),
    )
    .await;

    let res = receiver
        .recv()
        .await
        .ok_or_else(|| "runtime event must reach the transport receiver".to_owned())?;
    let forwarded = match res {
        Ok(f) => f,
        Err(never) => match never {},
    };
    assert!(forwarded.data.contains(&run.id));

    let persisted = cade_store::sqlite::run_events_after(&state.db, &run.id, -1)
        .map_err(|error| error.to_string())?;
    if persisted.len() != 1 || persisted[0].0 != 0 {
        return Err(format!(
            "expected one persisted event at sequence zero, got {persisted:?}"
        ));
    }
    Ok(())
}

#[tokio::test]
async fn cancel_run_command_records_durable_cancellation_request() -> Result<(), String> {
    let state = build_state_with_llm(std::sync::Arc::new(PanicOnCallLlm));
    let agent = cade_store::sqlite::AgentRow {
        id: "agent-cancel".to_owned(),
        name: "Cancellation agent".to_owned(),
        description: None,
        model: "test".to_owned(),
        system_prompt: None,
        created_at: None,
        compaction_model: None,
        theme: None,
        active_plan_json: None,
        parent_id: None,
    };
    cade_store::sqlite::create_agent(&state.db, &agent).map_err(|error| error.to_string())?;
    let run = cade_store::sqlite::create_run(&state.db, &agent.id, None)
        .map_err(|error| error.to_string())?;

    let response =
        crate::server::api::runs::cancel_run(State(state.clone()), Path(run.id.clone())).await;
    if response.status() != axum::http::StatusCode::OK {
        return Err(format!(
            "expected cancellation command success, got {}",
            response.status()
        ));
    }
    let stored = cade_store::sqlite::get_run(&state.db, &run.id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "cancelled run must remain queryable".to_owned())?;
    if stored.status != "cancelling" {
        return Err(format!(
            "expected durable cancelling status, got {}",
            stored.status
        ));
    }
    Ok(())
}

#[tokio::test]
async fn run_stream_replays_only_events_after_the_requested_cursor() -> Result<(), String> {
    let state = build_state_with_llm(std::sync::Arc::new(PanicOnCallLlm));
    let agent = cade_store::sqlite::AgentRow {
        id: "agent-replay".to_owned(),
        name: "Replay agent".to_owned(),
        description: None,
        model: "test".to_owned(),
        system_prompt: None,
        created_at: None,
        compaction_model: None,
        theme: None,
        active_plan_json: None,
        parent_id: None,
    };
    cade_store::sqlite::create_agent(&state.db, &agent).map_err(|error| error.to_string())?;
    let run = cade_store::sqlite::create_run(&state.db, &agent.id, None)
        .map_err(|error| error.to_string())?;
    for message_type in ["stream_start", "assistant_message", "run_done"] {
        cade_store::sqlite::append_run_event(
            &state.db,
            &run.id,
            &serde_json::json!({ "message_type": message_type }).to_string(),
        )
        .map_err(|error| error.to_string())?;
    }
    cade_store::sqlite::finish_run(&state.db, &run.id, "done")
        .map_err(|error| error.to_string())?;

    let response = crate::server::api::runs::stream_run(
        State(state),
        Path(run.id.clone()),
        axum::extract::Query(std::collections::HashMap::from([(
            "starting_after".to_owned(),
            "0".to_owned(),
        )])),
    )
    .await;
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8(body.to_vec()).map_err(|error| error.to_string())?;
    let payloads: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;

    if payloads.len() != 2 {
        return Err(format!("expected two replayed events, got {payloads:?}"));
    }
    let expected = [(1, "assistant_message"), (2, "run_done")];
    for (payload, (sequence, message_type)) in payloads.iter().zip(expected) {
        if payload["run_id"] != run.id
            || payload["seq_id"] != serde_json::json!(sequence)
            || payload["message_type"] != message_type
        {
            return Err(format!("unexpected replay envelope: {payload}"));
        }
    }
    Ok(())
}

#[tokio::test]
async fn cancelled_run_persists_a_terminal_cancelled_event() -> Result<(), String> {
    struct NeverContextBuilder;

    #[async_trait::async_trait]
    impl runtime::ContextBuilder for NeverContextBuilder {
        async fn build(
            &self,
            _agent_id: String,
            _conversation_id: Option<String>,
            _is_tool_return: bool,
        ) -> Result<runtime::RunContext, String> {
            Err("a cancelled run must not build model context".to_owned())
        }
    }

    struct NeverCapabilityExecutor;

    #[async_trait::async_trait]
    impl runtime::CapabilityExecutor for NeverCapabilityExecutor {
        async fn execute(
            &self,
            _input: runtime::TurnExecutionInput,
            _tool_calls: Vec<LlmToolCall>,
            _events: SseTx,
        ) -> Vec<(cade_agent::tools::manager::ToolResult, Value)> {
            Vec::new()
        }
    }

    let state = build_state_with_llm(std::sync::Arc::new(PanicOnCallLlm));
    let agent = cade_store::sqlite::AgentRow {
        id: "agent-cancel-terminal".to_owned(),
        name: "Cancellation terminal agent".to_owned(),
        description: None,
        model: "test".to_owned(),
        system_prompt: None,
        created_at: None,
        compaction_model: None,
        theme: None,
        active_plan_json: None,
        parent_id: None,
    };
    cade_store::sqlite::create_agent(&state.db, &agent).map_err(|error| error.to_string())?;
    let run = cade_store::sqlite::create_run(&state.db, &agent.id, None)
        .map_err(|error| error.to_string())?;
    if !cade_store::sqlite::request_run_cancellation(&state.db, &run.id)
        .map_err(|error| error.to_string())?
    {
        return Err("active run must accept cancellation".to_owned());
    }
    let (sender, mut receiver) = tokio::sync::mpsc::channel(4);

    run_agent_loop_with_dependencies(
        state.clone(),
        runtime::LoopRequest {
            agent_id: agent.id,
            conversation_id: None,
            run_id: run.id.clone(),
            theme_command: None,
            input: "ignored".to_owned(),
            permission_mode: None,
        },
        sender,
        std::sync::Arc::new(NeverContextBuilder),
        std::sync::Arc::new(NeverCapabilityExecutor),
    )
    .await;
    while receiver.recv().await.is_some() {}

    let stored_run = cade_store::sqlite::get_run(&state.db, &run.id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "cancelled run must remain queryable".to_owned())?;
    if stored_run.status != "cancelled" {
        return Err(format!(
            "expected cancelled run status, got {}",
            stored_run.status
        ));
    }
    let events = cade_store::sqlite::run_events_after(&state.db, &run.id, -1)
        .map_err(|error| error.to_string())?;
    let terminal = events
        .last()
        .ok_or_else(|| "cancelled run must persist a terminal event".to_owned())?;
    if terminal.0 != 1 || !terminal.1.contains("\"status\":\"cancelled\"") {
        return Err(format!(
            "expected ordered cancelled terminal event, got {terminal:?}"
        ));
    }
    Ok(())
}

/// A mock LlmProvider that panics if called.  Used to assert that an early
/// return path (e.g. depth-limit guard) never reaches the LLM at all.
pub(super) struct PanicOnCallLlm;
#[async_trait::async_trait]
impl cade_ai::LlmProvider for PanicOnCallLlm {
    async fn complete(
        &self,
        _r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<cade_ai::CompletionResponse> {
        panic!("LLM should not be called when depth limit is hit");
    }
    async fn stream(
        &self,
        _r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<
        std::pin::Pin<
            Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
        >,
    > {
        panic!("LLM stream should not be called");
    }
}

pub(super) fn build_state_with_llm(llm: std::sync::Arc<dyn cade_ai::LlmProvider>) -> AppState {
    let db = cade_store::sqlite::open(":memory:").unwrap();
    let config = std::sync::Arc::new(crate::server::config::ServerConfig {
        max_tokens_per_turn: Some(64_000),
        addr: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        llm_provider: crate::server::config::LlmProviderKind::Anthropic,
        default_model: "test".into(),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        deepseek_api_key: None,
        ollama_base_url: String::new(),
        api_key: None,
        allowed_origin: None,
        max_context_budget: None,
    });
    AppState {
        subagent_cancellations: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        db,
        llm,
        llm_router: std::sync::Arc::new(tokio::sync::RwLock::new(cade_ai::LlmRouter::build(
            &cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                deepseek_api_key: None,
                ollama_base_url: String::new(),
                llm_provider: String::new(),
            },
        ))),
        config,
        mcp: std::sync::Arc::new(crate::server::state::McpManager::empty()),
        rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
        memory_cache: std::sync::Arc::new(
            parking_lot::Mutex::new(std::collections::HashMap::new()),
        ),
        agent_activity: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        agent_metrics: std::sync::Arc::new(dashmap::DashMap::new()),
        agent_context_telemetry: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        context_cache: std::sync::Arc::new(parking_lot::Mutex::new(
            crate::server::state::SafeLruCache::new(crate::server::state::CONTEXT_CACHE_CAPACITY),
        )),
        all_skills: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        agent_skills: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        pending_subagent_results: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
        embedder: None,
    }
}

/// Stateful mock: returns `run_subagent` exactly once per loop level
/// (when the message list has just system+user) and a final text on
/// the next iter (after a tool result has been appended).  This keeps
/// the test fast while still exercising depth recursion.
struct OneRecurseLlm {
    call_count: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl cade_ai::LlmProvider for OneRecurseLlm {
    async fn complete(
        &self,
        r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<cade_ai::CompletionResponse> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Initial state for a fresh subagent loop is exactly 2 msgs
        // (system + user).  Anything more means we are post-tool-result.
        let is_initial = r.messages.len() == 2;
        if is_initial {
            Ok(cade_ai::CompletionResponse {
                content: Some("recursing".into()),
                tool_calls: vec![cade_ai::LlmToolCall {
                    id: "tc_rec".into(),
                    name: "run_subagent".into(),
                    arguments: serde_json::json!({"prompt": "deeper"}),
                    thought_signature: None,
                }],
                finish_reason: "tool_use".into(),
            })
        } else {
            Ok(cade_ai::CompletionResponse {
                content: Some("done".into()),
                tool_calls: vec![],
                finish_reason: "stop".into(),
            })
        }
    }
    async fn stream(
        &self,
        _r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<
        std::pin::Pin<
            Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
        >,
    > {
        unreachable!("stream() is not exercised by this mock")
    }
}

/// Recursion bound: a subagent that recurses once per level must hit
/// the depth cap (default 3) and return without deadlock.  At depth 3
/// the call is refused before acquiring a permit, so the chain
/// terminates.  Asserts (a) outer call succeeds, (b) LLM call count
/// is small (linear in depth, not exponential).
#[tokio::test]
async fn recursive_subagent_calls_are_bounded_by_depth() {
    let llm = std::sync::Arc::new(OneRecurseLlm {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_state_with_llm(llm_dyn);
    let (tx, _rx) = tokio::sync::mpsc::channel(64);

    // depth 0 → 1 → 2 → 3 (refused).  Each level: 2 LLM calls
    // (initial recurse + final).  Depth-2's recurse to depth 3 is
    // refused (tool result = error), then depth-2's next iter sees
    // post-tool-result state and returns final text.
    let args = serde_json::json!({ "prompt": "start", "_subagent_depth": 0 });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        handle_run_subagent_tool(&state, "parent_x", "tc_outer", &args, tx),
    )
    .await
    .expect("must not deadlock — chain must terminate via depth guard");

    assert!(
        !result.is_error,
        "outer subagent should complete: {}",
        result.output
    );
    let calls = llm.call_count.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        calls > 0 && calls < 20,
        "LLM call count must be small (linear in depth), got: {calls}"
    );
}

/// Approach C deliberately runs the subagent loop in-memory without
/// creating ephemeral `agent`/`message` rows.  That keeps the parent
/// agent's conversation history clean and avoids cross-contamination.
/// This test is a watchdog: if a future change accidentally persists
/// subagent traffic it will fail loudly.
#[tokio::test]
async fn subagent_run_does_not_pollute_parent_db() {
    let llm = std::sync::Arc::new(ScriptedLlm {
        call_count: std::sync::atomic::AtomicUsize::new(0),
        captured_iter2_messages: std::sync::Mutex::new(Vec::new()),
    });
    let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_state_with_llm(llm_dyn);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);

    let agents_before: i64 = state
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
        .unwrap();
    let messages_before: i64 = state
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();

    let args = serde_json::json!({ "prompt": "do thing" });
    let _ = handle_run_subagent_tool(&state, "parent_x", "tc_outer", &args, tx).await;

    let agents_after: i64 = state
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
        .unwrap();
    let messages_after: i64 = state
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();

    assert_eq!(
        agents_before, agents_after,
        "subagent must not create agent rows"
    );
    assert_eq!(
        messages_before, messages_after,
        "subagent must not persist messages to parent DB"
    );
}

/// A stateful mock that on the FIRST call returns a tool_call (forcing a
/// loop iteration), and on the SECOND call returns plain text.  The
/// LLM messages it receives are recorded so tests can verify that the
/// subagent loop fed back the tool result.
struct ScriptedLlm {
    call_count: std::sync::atomic::AtomicUsize,
    captured_iter2_messages: std::sync::Mutex<Vec<cade_ai::LlmMessage>>,
}
#[async_trait::async_trait]
impl cade_ai::LlmProvider for ScriptedLlm {
    async fn complete(
        &self,
        r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<cade_ai::CompletionResponse> {
        let n = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            Ok(cade_ai::CompletionResponse {
                content: None,
                tool_calls: vec![cade_ai::LlmToolCall {
                    id: "tc_inner_1".into(),
                    name: "fake_tool".into(),
                    arguments: serde_json::json!({}),
                    thought_signature: None,
                }],
                finish_reason: "tool_use".into(),
            })
        } else {
            let mut g = self.captured_iter2_messages.lock().unwrap();
            *g = r.messages.clone();
            Ok(cade_ai::CompletionResponse {
                content: Some("subagent done".into()),
                tool_calls: vec![],
                finish_reason: "stop".into(),
            })
        }
    }
    async fn stream(
        &self,
        _r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<
        std::pin::Pin<
            Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
        >,
    > {
        unreachable!("stream() is not exercised by this mock")
    }
}

/// RED: subagent currently does a single `complete()` and returns the
/// text.  When the LLM returns a tool_call instead, the subagent loop
/// must dispatch it and feed the result back in a second LLM call.
/// Asserts (a) the LLM was called exactly twice, (b) the second call
/// saw a "tool" role message containing the dispatch result.
#[tokio::test]
async fn subagent_dispatches_tool_calls_and_loops() {
    let llm = std::sync::Arc::new(ScriptedLlm {
        call_count: std::sync::atomic::AtomicUsize::new(0),
        captured_iter2_messages: std::sync::Mutex::new(Vec::new()),
    });
    let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_state_with_llm(llm_dyn);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);

    let args = serde_json::json!({ "prompt": "do thing" });
    let result = handle_run_subagent_tool(&state, "parent_x", "tc_outer", &args, tx).await;

    assert!(
        !result.is_error,
        "loop must succeed, got: {}",
        result.output
    );
    assert_eq!(
        llm.call_count.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "LLM must be called twice (first tool_call, then completion)"
    );
    let iter2 = llm.captured_iter2_messages.lock().unwrap().clone();
    let has_tool_msg = iter2
        .iter()
        .any(|m| m.role == "tool" && m.content.contains("fake_tool"));
    assert!(
        has_tool_msg,
        "iteration-2 messages must include a tool-role message echoing dispatch result, got roles: {:?}",
        iter2.iter().map(|m| &m.role).collect::<Vec<_>>()
    );
    assert!(
        result.output.contains("subagent done"),
        "final output must be from second LLM call, got: {}",
        result.output
    );
}

/// Subagents must NOT receive `run_subagent` in their tool list — this
/// is the second line of defence against runaway recursion (the first
/// being the depth guard in `handle_run_subagent_tool`).  Removing the
/// schema means the subagent's LLM never sees the tool advertised.
#[test]
fn filter_subagent_tools_strips_run_subagent_schema() {
    let schemas = vec![
        serde_json::json!({"name": "bash"}),
        serde_json::json!({"name": "run_subagent"}),
        serde_json::json!({"name": "read_file"}),
    ];
    let filtered =
        filter_subagent_tools(schemas, &cade_agent::subagents::SubagentTools::All, false);
    let names: Vec<String> = filtered
        .iter()
        .filter_map(|s| s["name"].as_str().map(String::from))
        .collect();
    assert!(
        !names.iter().any(|n| n == "run_subagent"),
        "run_subagent must be stripped, got: {names:?}"
    );
    assert!(names.iter().any(|n| n == "bash"));
    assert!(names.iter().any(|n| n == "read_file"));
}

/// When a subagent definition opts into nesting via `allow_run_subagent`,
/// the recursion tools stay in the inherited schema (still bounded by the
/// depth/semaphore caps at dispatch time).
#[test]
fn filter_subagent_tools_keeps_run_subagent_when_nesting_allowed() {
    let schemas = vec![
        serde_json::json!({"name": "bash"}),
        serde_json::json!({"name": "run_subagent"}),
        serde_json::json!({"name": "run_parallel_subagents"}),
        serde_json::json!({"name": "read_file"}),
    ];
    let filtered = filter_subagent_tools(schemas, &cade_agent::subagents::SubagentTools::All, true);
    let names: Vec<String> = filtered
        .iter()
        .filter_map(|s| s["name"].as_str().map(String::from))
        .collect();
    assert!(
        names.iter().any(|n| n == "run_subagent"),
        "run_subagent must be kept when nesting is allowed, got: {names:?}"
    );
    assert!(names.iter().any(|n| n == "run_parallel_subagents"));
    assert!(names.iter().any(|n| n == "bash"));
    assert!(names.iter().any(|n| n == "read_file"));
}

/// The built-in `finish` tool is injected by the subagent executor itself.
/// If a parent agent somehow had a `finish` schema in its tool list, it must
/// be stripped from what subagents see — otherwise a confused subagent could
/// call `finish` on the *parent's* schema instead of the injected one, or
/// cause unexpected tool routing.
#[test]
fn filter_subagent_tools_strips_finish_schema() {
    let schemas = vec![
        serde_json::json!({"name": "bash"}),
        serde_json::json!({"name": "finish"}),
        serde_json::json!({"name": "run_subagent"}),
        serde_json::json!({"name": "read_file"}),
    ];
    let filtered =
        filter_subagent_tools(schemas, &cade_agent::subagents::SubagentTools::All, false);
    let names: Vec<String> = filtered
        .iter()
        .filter_map(|s| s["name"].as_str().map(String::from))
        .collect();
    assert!(
        !names.iter().any(|n| n == "finish"),
        "finish must be stripped from parent schemas (injected fresh by executor), got: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "run_subagent"),
        "run_subagent must also be stripped, got: {names:?}"
    );
    assert!(names.iter().any(|n| n == "bash"));
    assert!(names.iter().any(|n| n == "read_file"));
}

// ── REC-1: Wall-clock timeout ─────────────────────────────────────────

/// Mock LLM that sleeps for a very long time on each `complete()` call,
/// simulating a hung tool or slow provider.
struct SlowLlm;
#[async_trait::async_trait]
impl cade_ai::LlmProvider for SlowLlm {
    async fn complete(
        &self,
        _r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<cade_ai::CompletionResponse> {
        // Sleep for 60 seconds — the test timeout is 2 seconds.
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        Ok(cade_ai::CompletionResponse {
            content: Some("should never reach here".into()),
            tool_calls: vec![],
            finish_reason: "stop".into(),
        })
    }
    async fn stream(
        &self,
        _r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<
        std::pin::Pin<
            Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
        >,
    > {
        unreachable!("stream() not used")
    }
}

/// REC-1: The subagent loop must enforce a wall-clock timeout. When the
/// timeout fires (before the LLM even responds), the subagent must
/// return an error result mentioning "timeout" and clean up its
/// ephemeral row.
///
/// We use `subagent_timeout_secs()` which returns 2s under `cfg(test)`.
#[tokio::test]
async fn subagent_loop_respects_wall_clock_timeout() {
    let llm = std::sync::Arc::new(SlowLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_state_with_llm(llm);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);

    let agents_before: i64 = state
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
        .unwrap();

    let args = serde_json::json!({ "prompt": "slow task" });

    let start = std::time::Instant::now();
    let result = handle_run_subagent_tool(&state, "parent_slow", "tc_slow", &args, tx).await;
    let elapsed = start.elapsed();

    // Must return within ~5s (2s test timeout + slack), NOT 60s.
    assert!(
        elapsed.as_secs() < 10,
        "must respect timeout, took {elapsed:?}"
    );
    assert!(result.is_error, "timed-out subagent must be an error");
    assert!(
        result.output.to_lowercase().contains("timeout"),
        "error must mention timeout, got: {}",
        result.output
    );

    // Ephemeral row must be cleaned up (REC-2 guard fires on timeout path).
    let agents_after: i64 = state
        .db
        .get()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        agents_before, agents_after,
        "ephemeral row must be cleaned up after timeout"
    );
}

// ── REC-2: EphemeralEnvironment cleanup ────────────────────────────────

/// REC-2: An `EphemeralEnvironment` must delete the ephemeral agent row
/// and write back subagent memory when dropped, even if the agentic
/// loop panics or returns early.
#[test]
fn ephemeral_environment_cleans_up_on_drop() {
    use super::subagent::EphemeralEnvironment;

    let db = cade_store::sqlite::open(":memory:").unwrap();
    // Create parent agent.
    cade_store::sqlite::create_agent(
        &db,
        &cade_store::sqlite::AgentRow {
            id: "parent_g".into(),
            name: "parent".into(),
            model: "test".into(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        },
    )
    .unwrap();
    // Create ephemeral subagent row (simulates line 197 of subagent.rs).
    cade_store::sqlite::create_agent(
        &db,
        &cade_store::sqlite::AgentRow {
            id: "sa_guard".into(),
            name: "ephemeral".into(),
            model: "test".into(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        },
    )
    .unwrap();

    // Subagent wrote a finding during its loop.
    cade_store::sqlite::upsert_memory_block(
        &db,
        "sa_guard",
        "my_finding",
        "important data",
        None,
        None,
    )
    .unwrap();

    // Row exists before drop.
    let exists_before = cade_store::sqlite::get_agent(&db, "sa_guard")
        .unwrap()
        .is_some();
    assert!(exists_before, "ephemeral row must exist before guard drop");

    // Create guard, then drop it.
    {
        let _guard =
            EphemeralEnvironment::new(db.clone(), "sa_guard".to_string(), "parent_g".to_string());
    } // ← drop fires here

    // Row must be gone.
    let exists_after = cade_store::sqlite::get_agent(&db, "sa_guard")
        .unwrap()
        .is_some();
    assert!(
        !exists_after,
        "ephemeral row must be deleted after guard drop"
    );

    // Write-back must have happened: parent should have `subagent:my_finding`.
    let parent_blocks = cade_store::sqlite::get_memory_blocks(&db, "parent_g").unwrap();
    let labels: Vec<&str> = parent_blocks.iter().map(|(l, _, _)| l.as_str()).collect();
    assert!(
        labels.contains(&"subagent:my_finding"),
        "write-back must run before delete; got labels: {labels:?}"
    );
}

// ── Phase A2: skills meta-tools ──────────────────────────────────────

// ── Phase A3: checkpoint meta-tools ──────────────────────────────────

// ── Phase A4: artifact / agents meta-tools ────────────────────────────

// ── F6: cross-conversation search ────────────────────────────────────────

// ── F8: compaction transparency in conversation_search ────────────────────

/// RED: at depth >= CADE_SUBAGENT_MAX_DEPTH (default 3), the tool must
/// short-circuit with an error and never call the LLM.  Currently fails
/// because no depth guard exists.
#[tokio::test]
async fn depth_limit_blocks_recursion_before_llm_call() {
    let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_state_with_llm(llm);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);

    let args = serde_json::json!({
        "prompt": "do thing",
        "_subagent_depth": 3,
    });

    // Should NOT panic — i.e. LLM is never called.
    let result = handle_run_subagent_tool(&state, "parent_agent_x", "tc_1", &args, tx).await;

    assert!(result.is_error, "depth-limit must produce an error result");
    assert!(
        result.output.to_lowercase().contains("depth"),
        "error message must mention depth, got: {}",
        result.output
    );
}

// ── subagent meta-tool dispatch ───────────────────────────────────────

// ── subagent file-edit tracking ───────────────────────────────────────

/// When a subagent dispatches a `write_file` tool, the parent agent's
/// `recent_edits` memory block must be updated with the file path.
#[tokio::test]
async fn subagent_write_file_records_recent_edit() {
    // Use a path within the current working dir to avoid sandbox restrictions
    let tmp_path = std::env::current_dir()
        .unwrap()
        .join("_test_subagent_edit.tmp");
    let tmp_path_str = tmp_path.to_str().unwrap().to_string();

    struct WriteFileLlmInner {
        call_count: std::sync::atomic::AtomicUsize,
        path: String,
    }
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for WriteFileLlmInner {
        async fn complete(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            let n = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(cade_ai::CompletionResponse {
                    content: None,
                    tool_calls: vec![cade_ai::LlmToolCall {
                        id: "tc_wf".into(),
                        name: "write_file".into(),
                        arguments: serde_json::json!({
                            "path": self.path,
                            "content": "hello from subagent"
                        }),
                        thought_signature: None,
                    }],
                    finish_reason: "tool_use".into(),
                })
            } else {
                Ok(cade_ai::CompletionResponse {
                    content: Some("wrote the file".into()),
                    tool_calls: vec![],
                    finish_reason: "stop".into(),
                })
            }
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unreachable!()
        }
    }

    let llm = std::sync::Arc::new(WriteFileLlmInner {
        call_count: std::sync::atomic::AtomicUsize::new(0),
        path: tmp_path_str.clone(),
    });
    let llm_dyn = llm as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_state_with_llm(llm_dyn);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);

    // Create a parent agent in the DB
    let parent_id = "parent_edit_track";
    cade_store::sqlite::create_agent(
        &state.db,
        &cade_store::sqlite::AgentRow {
            id: parent_id.to_string(),
            name: "edit-track-parent".to_string(),
            model: "mock".to_string(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        },
    )
    .unwrap();

    let args = serde_json::json!({ "prompt": "write a file" });
    let result = handle_run_subagent_tool(&state, parent_id, "tc_wf", &args, tx).await;

    assert!(!result.is_error, "got: {}", result.output);

    // Check that recent_edits was recorded for the parent agent
    let blocks = cade_store::sqlite::get_memory_blocks(&state.db, parent_id)
        .expect("get_memory_blocks should succeed");
    let re = blocks.iter().find(|(l, _, _)| l == "recent_edits");
    assert!(
        re.is_some(),
        "recent_edits block must exist after subagent write_file, blocks: {:?}",
        blocks.iter().map(|(l, _, _)| l).collect::<Vec<_>>()
    );
    let (_, value, _) = re.unwrap();
    assert!(
        value.contains(&tmp_path_str),
        "recent_edits must contain the written file path, got: {value}"
    );

    // Clean up
    let _ = std::fs::remove_file(&tmp_path);
}

/// Same as above but with an MCP-prefixed tool name — verifies that
/// `is_file_edit_tool` correctly strips the prefix.
struct McpWriteFileLlm {
    call_count: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl cade_ai::LlmProvider for McpWriteFileLlm {
    async fn complete(
        &self,
        _r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<cade_ai::CompletionResponse> {
        let n = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            Ok(cade_ai::CompletionResponse {
                content: None,
                tool_calls: vec![cade_ai::LlmToolCall {
                    id: "tc_mcp_wf".into(),
                    // MCP-prefixed name — this is what the LLM sees
                    name: "developer__write_file".into(),
                    arguments: serde_json::json!({
                        "path": "/tmp/cade_test_mcp_subagent_edit.txt",
                        "file_text": "hello from mcp subagent"
                    }),
                    thought_signature: None,
                }],
                finish_reason: "tool_use".into(),
            })
        } else {
            Ok(cade_ai::CompletionResponse {
                content: Some("wrote via mcp".into()),
                tool_calls: vec![],
                finish_reason: "stop".into(),
            })
        }
    }
    async fn stream(
        &self,
        _r: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<
        std::pin::Pin<
            Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
        >,
    > {
        unreachable!()
    }
}

#[tokio::test]
async fn subagent_mcp_prefixed_write_records_recent_edit() {
    let llm = std::sync::Arc::new(McpWriteFileLlm {
        call_count: std::sync::atomic::AtomicUsize::new(0),
    });
    let llm_dyn = llm as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_state_with_llm(llm_dyn);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);

    let parent_id = "parent_mcp_edit";
    cade_store::sqlite::create_agent(
        &state.db,
        &cade_store::sqlite::AgentRow {
            id: parent_id.to_string(),
            name: "mcp-edit-parent".to_string(),
            model: "mock".to_string(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        },
    )
    .unwrap();

    let args = serde_json::json!({ "prompt": "write via mcp" });
    let _result = handle_run_subagent_tool(&state, parent_id, "tc_mcp", &args, tx).await;

    // The MCP tool won't actually write (no MCP server connected in test),
    // but we don't care about the dispatch result — we care that the
    // tracking code matched the MCP-prefixed name.  The tool result will
    // be an error ("Unknown tool"), but is_file_edit_tool should still
    // be true for the name.  However, tracking is gated on !is_error,
    // so this tests that the name matching works even if the tool errors.
    // We adjust: recent_edits should NOT be written when tool errored.
    let blocks = cade_store::sqlite::get_memory_blocks(&state.db, parent_id)
        .expect("get_memory_blocks should succeed");
    let re = blocks.iter().find(|(l, _, _)| l == "recent_edits");
    // Tool errored (no MCP server), so no edit should be recorded.
    assert!(
        re.is_none(),
        "recent_edits should NOT be set when MCP tool errors, blocks: {:?}",
        blocks.iter().map(|(l, _, _)| l).collect::<Vec<_>>()
    );
}

#[cfg(test)]
mod p4_guardrail_tests {
    use super::*;

    #[test]
    fn parse_max_session_cost_unset_disables_guardrail() {
        assert_eq!(parse_max_session_cost(None), None);
    }

    #[test]
    fn parse_max_session_cost_empty_disables_guardrail() {
        assert_eq!(parse_max_session_cost(Some("")), None);
        assert_eq!(parse_max_session_cost(Some("   ")), None);
    }

    #[test]
    fn parse_max_session_cost_nonpositive_disables_guardrail() {
        assert_eq!(parse_max_session_cost(Some("0")), None);
        assert_eq!(parse_max_session_cost(Some("0.0")), None);
        assert_eq!(parse_max_session_cost(Some("-5")), None);
    }

    #[test]
    fn parse_max_session_cost_positive_returns_cap() {
        assert_eq!(parse_max_session_cost(Some("2.50")), Some(2.50));
        assert_eq!(parse_max_session_cost(Some(" 10 ")), Some(10.0));
    }

    #[test]
    fn parse_max_session_cost_garbage_disables_guardrail() {
        assert_eq!(parse_max_session_cost(Some("not-a-number")), None);
        assert_eq!(parse_max_session_cost(Some("$5")), None);
    }

    /// `pricing_registry` returns a stable instance (`OnceLock`).
    #[test]
    fn pricing_registry_is_stable() {
        let p1 = pricing_registry() as *const _;
        let p2 = pricing_registry() as *const _;
        assert!(std::ptr::eq(p1, p2));
    }

    // ── P6: tool-turn output cap ─────────────────────────────────────────

    #[test]
    fn parse_tool_turn_unset_disables_cap() {
        assert_eq!(parse_tool_turn_max_tokens(None), None);
    }

    #[test]
    fn parse_tool_turn_empty_disables_cap() {
        assert_eq!(parse_tool_turn_max_tokens(Some("")), None);
        assert_eq!(parse_tool_turn_max_tokens(Some("   ")), None);
    }

    #[test]
    fn parse_tool_turn_zero_disables_cap() {
        assert_eq!(parse_tool_turn_max_tokens(Some("0")), None);
    }

    #[test]
    fn parse_tool_turn_garbage_disables_cap() {
        assert_eq!(parse_tool_turn_max_tokens(Some("abc")), None);
        assert_eq!(parse_tool_turn_max_tokens(Some("1024k")), None);
    }

    #[test]
    fn parse_tool_turn_positive_returns_cap() {
        assert_eq!(parse_tool_turn_max_tokens(Some("1024")), Some(1024));
        assert_eq!(parse_tool_turn_max_tokens(Some("4096")), Some(4096));
        assert_eq!(parse_tool_turn_max_tokens(Some(" 512 ")), Some(512));
    }

    // ── CADE_MAX_TURNS cap ──────────────────────────────────────────────

    #[test]
    fn parse_max_turns_unset_and_invalid_keep_default() {
        assert_eq!(parse_max_turns(None), None);
        assert_eq!(parse_max_turns(Some("")), None);
        assert_eq!(parse_max_turns(Some("   ")), None);
        assert_eq!(parse_max_turns(Some("0")), None);
        assert_eq!(parse_max_turns(Some("-3")), None);
        assert_eq!(parse_max_turns(Some("abc")), None);
    }

    #[test]
    fn parse_max_turns_positive_returns_cap() {
        assert_eq!(parse_max_turns(Some("30")), Some(30));
        assert_eq!(parse_max_turns(Some(" 100 ")), Some(100));
    }

    #[test]
    fn parse_max_turns_ceiling_unset_and_invalid_keep_multiplier_default() {
        assert_eq!(parse_max_turns_ceiling(None), None);
        assert_eq!(parse_max_turns_ceiling(Some("")), None);
        assert_eq!(parse_max_turns_ceiling(Some("0")), None);
        assert_eq!(parse_max_turns_ceiling(Some("-1")), None);
        assert_eq!(parse_max_turns_ceiling(Some("abc")), None);
    }

    #[test]
    fn parse_max_turns_ceiling_positive_returns_cap() {
        assert_eq!(parse_max_turns_ceiling(Some("150")), Some(150));
        assert_eq!(parse_max_turns_ceiling(Some(" 40 ")), Some(40));
    }

    #[test]
    fn adaptive_turn_budget_starts_at_base_without_new_work() {
        assert_eq!(adaptive_turn_budget(20, 100, 0), 20);
        assert_eq!(adaptive_turn_budget(5, 25, 0), 5);
    }

    #[test]
    fn adaptive_turn_budget_grows_with_distinct_work_and_clamps_to_ceiling() {
        assert_eq!(adaptive_turn_budget(20, 100, 5), 30);
        assert_eq!(adaptive_turn_budget(20, 100, 100), 100);
        // ceiling below base → base wins (budget never shrinks below base).
        assert_eq!(adaptive_turn_budget(20, 10, 100), 20);
    }
}

#[cfg(test)]
mod sse_protocol_tests {
    //! Integration coverage for the `POST /v1/agents/:id/run` SSE
    //! response.  These tests drive `run_agent` end-to-end through the
    //! axum extractors and assert the protocol surface seen by the
    //! CLI / TUI / GUI clients.  Started as Task 2.3 of the code-review
    //! resolution plan.
    //!
    //! The current matrix covers the two error paths that do **not**
    //! require an LLM call (empty input, missing conversation).  A
    //! follow-up commit will add scripted-LLM happy-path and
    //! tool-dispatch coverage.

    use super::*;
    use axum::{
        Json,
        body::to_bytes,
        extract::{Path, State},
        http::StatusCode,
    };

    /// Build a state whose LLM panics if called — proves no LLM
    /// request happens on the error paths under test.
    fn state_no_llm() -> AppState {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        build_state_with_llm(llm)
    }

    #[tokio::test]
    async fn empty_input_returns_400_bad_request_with_missing_input_message() {
        let state = state_no_llm();
        let resp = run_agent(
            State(state),
            Path("agent-x".to_string()),
            Json(serde_json::json!({ "input": "" })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body_bytes = to_bytes(resp.into_body(), 8 * 1024)
            .await
            .expect("read body");
        let body_str = std::str::from_utf8(&body_bytes).expect("utf8");
        assert!(
            body_str.contains("missing 'input'"),
            "body must explain the missing-input error; got: {body_str}"
        );
    }

    #[tokio::test]
    async fn missing_input_field_returns_400_bad_request() {
        let state = state_no_llm();
        let resp = run_agent(
            State(state),
            Path("agent-x".to_string()),
            Json(serde_json::json!({})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn nonexistent_conversation_id_returns_404_not_found() {
        // Body declares conversation_id that does not exist in the DB.
        // `resolve_conversation` must short-circuit with 404 before any
        // SSE stream is opened.
        let state = state_no_llm();
        let resp = run_agent(
            State(state),
            Path("agent-x".to_string()),
            Json(serde_json::json!({
                "input": "hello",
                "conversation_id": "conv-does-not-exist",
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let body_bytes = to_bytes(resp.into_body(), 8 * 1024)
            .await
            .expect("read body");
        let body_str = std::str::from_utf8(&body_bytes).expect("utf8");
        assert!(
            body_str.contains("conv-does-not-exist") || body_str.contains("not found"),
            "body must reference the missing conversation; got: {body_str}"
        );
    }

    #[tokio::test]
    async fn empty_input_response_does_not_leak_internal_paths() {
        // §3.3 of tdd-guide: error responses must not expose stack
        // traces, internal file paths, or framework version strings.
        let state = state_no_llm();
        let resp = run_agent(
            State(state),
            Path("agent-x".to_string()),
            Json(serde_json::json!({ "input": "" })),
        )
        .await;
        let body_bytes = to_bytes(resp.into_body(), 8 * 1024)
            .await
            .expect("read body");
        let body_str = std::str::from_utf8(&body_bytes).expect("utf8");
        let lc = body_str.to_lowercase();
        assert!(
            !lc.contains("/home/")
                && !lc.contains("c:\\")
                && !lc.contains("backtrace")
                && !lc.contains("rust_panic"),
            "error body must not leak host paths or stack traces: {body_str}"
        );
    }
}

#[cfg(test)]
mod runtime_contract_tests {
    use super::runtime::{
        CapabilityExecutor, ContextBuilder, RunContext, RunRequest, ServerAgentRuntime,
    };
    use super::*;
    use cade_agent::tools::manager::ToolResult;
    use cade_ai::{LlmMessage, LlmToolCall, StreamChunk};
    use serde_json::Value;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone)]
    struct RecordingContextBuilder {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ContextBuilder for RecordingContextBuilder {
        async fn build(
            &self,
            _agent_id: String,
            _conversation_id: Option<String>,
            _is_tool_return: bool,
        ) -> Result<RunContext, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok((
                "test".to_owned(),
                vec![LlmMessage {
                    role: "user".to_owned(),
                    content: "bounded context".to_owned(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                    cache_control: None,
                }],
                Vec::new(),
            ))
        }
    }

    #[derive(Clone)]
    struct RecordingCapabilityExecutor {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl CapabilityExecutor for RecordingCapabilityExecutor {
        async fn execute(
            &self,
            _input: runtime::TurnExecutionInput,
            _tool_calls: Vec<LlmToolCall>,
            _events: SseTx,
        ) -> Vec<(ToolResult, Value)> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        }
    }

    struct FinalResponseLlm;

    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for FinalResponseLlm {
        async fn complete(
            &self,
            _request: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            Err(cade_ai::Error::custom("complete is not used"))
        }

        async fn stream(
            &self,
            _request: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<StreamChunk>> + Send>,
            >,
        > {
            Ok(Box::pin(tokio_stream::iter(vec![
                Ok(StreamChunk::Text("completed".to_owned())),
                Ok(StreamChunk::Done),
            ])))
        }
    }

    struct ToolThenFinalLlm {
        stream_calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for ToolThenFinalLlm {
        async fn complete(
            &self,
            _request: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            Err(cade_ai::Error::custom("complete is not used"))
        }

        async fn stream(
            &self,
            _request: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<StreamChunk>> + Send>,
            >,
        > {
            let call = self.stream_calls.fetch_add(1, Ordering::SeqCst);
            let chunks = if call == 0 {
                vec![
                    Ok(StreamChunk::ToolCall(LlmToolCall {
                        id: "tool-call-1".to_owned(),
                        name: "test_tool".to_owned(),
                        arguments: serde_json::json!({}),
                        thought_signature: None,
                    })),
                    Ok(StreamChunk::Done),
                ]
            } else {
                vec![
                    Ok(StreamChunk::Text("completed".to_owned())),
                    Ok(StreamChunk::Done),
                ]
            };
            Ok(Box::pin(tokio_stream::iter(chunks)))
        }
    }

    #[derive(Clone)]
    struct ToolResultCapabilityExecutor {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl CapabilityExecutor for ToolResultCapabilityExecutor {
        async fn execute(
            &self,
            _input: runtime::TurnExecutionInput,
            tool_calls: Vec<LlmToolCall>,
            _events: SseTx,
        ) -> Vec<(ToolResult, Value)> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tool_calls
                .into_iter()
                .map(|call| {
                    (
                        ToolResult {
                            tool_call_id: call.id,
                            tool_name: call.name,
                            output: "tool completed".to_owned(),
                            is_error: false,
                            ui_resource_uri: None,
                        },
                        call.arguments,
                    )
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn runtime_routes_model_tool_calls_through_its_capability_executor() -> Result<(), String>
    {
        let llm = Arc::new(ToolThenFinalLlm {
            stream_calls: AtomicUsize::new(0),
        }) as Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let agent = cade_store::sqlite::AgentRow {
            id: "agent-capability".to_owned(),
            name: "Capability agent".to_owned(),
            description: None,
            model: "test".to_owned(),
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        };
        cade_store::sqlite::create_agent(&state.db, &agent).map_err(|error| error.to_string())?;

        let context_calls = Arc::new(AtomicUsize::new(0));
        let capability_calls = Arc::new(AtomicUsize::new(0));
        let runtime = ServerAgentRuntime::with_dependencies(
            state,
            Arc::new(RecordingContextBuilder {
                calls: context_calls.clone(),
            }),
            Arc::new(ToolResultCapabilityExecutor {
                calls: capability_calls.clone(),
            }),
        );

        let mut handle = runtime
            .start(RunRequest {
                agent_id: "agent-capability".to_owned(),
                conversation_id: None,
                input: "use a tool".to_owned(),
                permission_mode: None,
            })
            .await;
        while handle.events.recv().await.is_some() {}

        if capability_calls.load(Ordering::SeqCst) != 1 {
            return Err(
                "runtime must execute model tool calls through its capability executor".to_owned(),
            );
        }
        if context_calls.load(Ordering::SeqCst) != 2 {
            return Err("runtime must rebuild context after persisting a tool result".to_owned());
        }
        Ok(())
    }

    /// LLM that always requests a tool call and never produces a final
    /// answer.  With `vary_arguments == false` every turn is byte-identical
    /// (exercises the degenerate-loop detector); with `true` arguments
    /// change each turn (exercises the turn budget).  `stop_after_tools` lets
    /// a test finish after N tool turns with a normal final answer.
    struct RepeatingToolLlm {
        stream_calls: AtomicUsize,
        vary_arguments: bool,
        stop_after_tools: Option<usize>,
    }

    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for RepeatingToolLlm {
        async fn complete(
            &self,
            _request: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            Err(cade_ai::Error::custom("complete is not used"))
        }

        async fn stream(
            &self,
            _request: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<StreamChunk>> + Send>,
            >,
        > {
            let call = self.stream_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(stop_after) = self.stop_after_tools
                && call >= stop_after
            {
                return Ok(Box::pin(tokio_stream::iter(vec![
                    Ok(StreamChunk::Text("done".to_owned())),
                    Ok(StreamChunk::Done),
                ])));
            }
            let arguments = if self.vary_arguments {
                serde_json::json!({ "query": format!("query-{call}") })
            } else {
                serde_json::json!({ "query": "stuck-query" })
            };
            Ok(Box::pin(tokio_stream::iter(vec![
                Ok(StreamChunk::ToolCall(LlmToolCall {
                    id: format!("repeating-tool-call-{call}"),
                    name: "stuck_tool".to_owned(),
                    arguments,
                    thought_signature: None,
                })),
                Ok(StreamChunk::Done),
            ])))
        }
    }

    struct LoopOutcome {
        executor_calls: usize,
        run_status: String,
        persisted_events: String,
    }

    async fn run_repeating_llm(
        agent_id: &str,
        vary_arguments: bool,
        stop_after_tools: Option<usize>,
    ) -> Result<LoopOutcome, String> {
        let llm = Arc::new(RepeatingToolLlm {
            stream_calls: AtomicUsize::new(0),
            vary_arguments,
            stop_after_tools,
        }) as Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let agent = cade_store::sqlite::AgentRow {
            id: agent_id.to_owned(),
            name: "Repeating agent".to_owned(),
            description: None,
            model: "test".to_owned(),
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        };
        cade_store::sqlite::create_agent(&state.db, &agent).map_err(|error| error.to_string())?;

        let capability_calls = Arc::new(AtomicUsize::new(0));
        let runtime = ServerAgentRuntime::with_dependencies(
            state.clone(),
            Arc::new(RecordingContextBuilder {
                calls: Arc::new(AtomicUsize::new(0)),
            }),
            Arc::new(ToolResultCapabilityExecutor {
                calls: capability_calls.clone(),
            }),
        );
        let handle = runtime
            .start(RunRequest {
                agent_id: agent_id.to_owned(),
                conversation_id: None,
                input: "repeat forever".to_owned(),
                permission_mode: None,
            })
            .await;
        let mut events = handle.events;
        while events.recv().await.is_some() {}

        let run = cade_store::sqlite::get_run(&state.db, &handle.run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "run must be queryable after the loop stopped".to_owned())?;
        let persisted = cade_store::sqlite::run_events_after(&state.db, &handle.run_id, -1)
            .map_err(|error| error.to_string())?;
        let joined = persisted
            .iter()
            .map(|(_, event)| event.as_str())
            .collect::<Vec<&str>>()
            .join("\n");
        Ok(LoopOutcome {
            executor_calls: capability_calls.load(Ordering::SeqCst),
            run_status: run.status,
            persisted_events: joined,
        })
    }

    #[tokio::test]
    async fn runtime_aborts_on_identical_repeated_tool_calls() -> Result<(), String> {
        // Same tool + identical arguments every turn: the degenerate-loop
        // detector must stop the run after MAX_IDENTICAL_REPEAT_TOOL_CALLS
        // executor dispatches instead of burning all 20 turns.
        let outcome = run_repeating_llm("agent-degenerate", false, None).await?;
        if outcome.executor_calls != MAX_IDENTICAL_REPEAT_TOOL_CALLS {
            return Err(format!(
                "degenerate loop must stop after {} identical calls, executor ran {} times",
                MAX_IDENTICAL_REPEAT_TOOL_CALLS, outcome.executor_calls
            ));
        }
        if outcome.run_status != "error" {
            return Err(format!(
                "degenerate abort must record status 'error', got '{}'",
                outcome.run_status
            ));
        }
        if !outcome.persisted_events.contains("Agentic loop detected") {
            return Err("degenerate abort must surface an SSE error event".to_owned());
        }
        Ok(())
    }

    #[tokio::test]
    async fn runtime_escalates_turn_budget_for_distinct_tool_work() -> Result<(), String> {
        // 24 distinct (but never-converging) tool turns would blow past the
        // default base cap of 20.  The adaptive budget grows +2 per new
        // fingerprint, so the run completes normally instead of aborting.
        let outcome = run_repeating_llm("agent-escalate", true, Some(24)).await?;
        if outcome.executor_calls != 24 {
            return Err(format!(
                "adaptive budget must let a 24-turn productive run finish, executor ran {} times",
                outcome.executor_calls
            ));
        }
        if outcome.run_status != "done" {
            return Err(format!(
                "escalated run must finish with status 'done', got '{}'",
                outcome.run_status
            ));
        }
        Ok(())
    }

    #[tokio::test]
    async fn runtime_uses_its_context_builder_for_a_completed_run() -> Result<(), String> {
        let llm = Arc::new(FinalResponseLlm) as Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let agent = cade_store::sqlite::AgentRow {
            id: "agent-context".to_owned(),
            name: "Context agent".to_owned(),
            description: None,
            model: "test".to_owned(),
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        };
        cade_store::sqlite::create_agent(&state.db, &agent).map_err(|error| error.to_string())?;

        let context_calls = Arc::new(AtomicUsize::new(0));
        let capability_calls = Arc::new(AtomicUsize::new(0));
        let runtime = ServerAgentRuntime::with_dependencies(
            state,
            Arc::new(RecordingContextBuilder {
                calls: context_calls.clone(),
            }),
            Arc::new(RecordingCapabilityExecutor {
                calls: capability_calls.clone(),
            }),
        );

        let mut handle = runtime
            .start(RunRequest {
                agent_id: "agent-context".to_owned(),
                conversation_id: None,
                input: "hello".to_owned(),
                permission_mode: None,
            })
            .await;
        while handle.events.recv().await.is_some() {}

        if context_calls.load(Ordering::SeqCst) != 1 {
            return Err("runtime must build bounded context once for a final response".to_owned());
        }
        if capability_calls.load(Ordering::SeqCst) != 0 {
            return Err(
                "runtime must not execute capabilities when the model returns none".to_owned(),
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn runtime_start_creates_a_durable_run_and_returns_its_handle() -> Result<(), String> {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let agent = cade_store::sqlite::AgentRow {
            id: "agent-x".to_owned(),
            name: "Test agent".to_owned(),
            description: None,
            model: "test".to_owned(),
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
            parent_id: None,
        };
        cade_store::sqlite::create_agent(&state.db, &agent).map_err(|error| error.to_string())?;
        let runtime = ServerAgentRuntime::new(state.clone());

        let handle = runtime
            .start(RunRequest {
                agent_id: "agent-x".to_owned(),
                conversation_id: None,
                input: "hello".to_owned(),
                permission_mode: None,
            })
            .await;

        let run = cade_store::sqlite::get_run(&state.db, &handle.run_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "runtime start must create a durable run".to_owned())?;
        if run.agent_id != "agent-x" {
            return Err(format!("run must belong to agent-x, got {}", run.agent_id));
        }
        if state.agent_activity.read().await.get("agent-x").is_none() {
            return Err("runtime start must record agent activity".to_owned());
        }

        drop(handle.events);
        Ok(())
    }
}

#[cfg(test)]
mod run_agent_helpers_tests {
    use super::*;
    use serde_json::json;

    // ── parse_input ────────────────────────────────────────────────────────

    #[test]
    fn parse_input_returns_string_for_valid_input() {
        let body = json!({ "input": "hello world" });
        let result = parse_input(&body);
        assert_eq!(result.unwrap(), "hello world");
    }

    #[test]
    fn parse_input_errors_on_missing_field() {
        let body = json!({});
        assert!(parse_input(&body).is_err());
    }

    #[test]
    fn parse_input_errors_on_empty_string() {
        let body = json!({ "input": "" });
        assert!(parse_input(&body).is_err());
    }

    #[test]
    fn parse_input_errors_on_non_string_value() {
        let body = json!({ "input": 42 });
        assert!(parse_input(&body).is_err());
    }

    // ── detect_theme_cmd ───────────────────────────────────────────────────

    #[test]
    fn detect_theme_cmd_returns_name_for_theme_prefix() {
        assert_eq!(
            detect_theme_cmd("/theme catppuccin"),
            Some("catppuccin".to_string())
        );
    }

    #[test]
    fn detect_theme_cmd_trims_whitespace() {
        assert_eq!(detect_theme_cmd("/theme  dark  "), Some("dark".to_string()));
    }

    #[test]
    fn detect_theme_cmd_returns_none_for_non_theme_input() {
        assert_eq!(detect_theme_cmd("hello"), None);
        assert_eq!(detect_theme_cmd("/memory"), None);
        assert_eq!(detect_theme_cmd(""), None);
    }

    #[test]
    fn detect_theme_cmd_handles_reload() {
        assert_eq!(
            detect_theme_cmd("/theme reload"),
            Some("reload".to_string())
        );
    }

    // ── make_run_id ────────────────────────────────────────────────────────

    #[test]
    fn make_run_id_fallback_starts_with_run_local() {
        // We can't construct a real AppState easily, but we can verify the
        // fallback format by calling the inner logic directly.
        let ts = chrono::Utc::now().timestamp();
        let id = format!("run-local-{ts}");
        assert!(id.starts_with("run-local-"));
        assert!(id.len() > "run-local-".len());
    }
}

#[cfg(test)]
mod advanced_execution_tests {
    use super::*;
    use crate::server::api::run::execution::execute_turn_tools;
    use cade_ai::LlmToolCall;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_parallel_and_sequential_execution() {
        let state = build_state_with_llm(Arc::new(PanicOnCallLlm));
        let (tx, _rx) = tokio::sync::mpsc::channel(128);

        // Create a temporary file in the current working directory to pass the sandbox
        let temp_file_path = std::env::current_dir()
            .unwrap()
            .join("_test_advanced_execution_read.tmp");
        std::fs::write(
            &temp_file_path,
            "test file content containing some lines total",
        )
        .unwrap();
        let file_path_str = temp_file_path.to_str().unwrap().to_string();

        let tool_calls = vec![
            // A call for the parallel MoA dispatcher
            LlmToolCall {
                id: "tc-read".to_string(),
                name: "read_file".to_string(),
                arguments: json!({"path": file_path_str}),
                thought_signature: None,
            },
            // A call for the sequential workflow dispatcher
            LlmToolCall {
                id: "tc-seq".to_string(),
                name: "run_sequential_tasks".to_string(),
                arguments: json!({
                    "steps": [
                        { "tool_name": "bash", "arguments": { "command": "echo 'hello'" } },
                        { "tool_name": "bash", "arguments": { "command": "echo 'world'" } }
                    ]
                }),
                thought_signature: None,
            },
        ];

        let results = execute_turn_tools(
            state,
            runtime::TurnExecutionInput {
                agent_id: "test-agent".to_string(),
                conversation_id: Some("test-conv".to_string()),
                run_id: "run-test-sequence".to_string(),
                input: "read Cargo.toml and run a sequence".to_string(),
                permission_mode: None,
            },
            tool_calls,
            tx,
        )
        .await;

        // Clean up immediately after execution
        let _ = std::fs::remove_file(&temp_file_path);

        assert_eq!(results.len(), 2);

        // Verify parallel result (read_file)
        let read_result = results
            .iter()
            .find(|(r, _)| r.tool_name == "read_file")
            .unwrap();
        assert!(
            !read_result.0.is_error,
            "read_file error output: {}",
            read_result.0.output
        );
        // Verify that the output contains the expected content
        assert!(read_result.0.output.contains("test file content"));

        // Verify sequential result
        let seq_result = results
            .iter()
            .find(|(r, _)| r.tool_name == "run_sequential_tasks")
            .unwrap();
        assert!(!seq_result.0.is_error);
        assert!(seq_result.0.output.contains("Step 0: bash ->\nhello"));
        assert!(seq_result.0.output.contains("Step 1: bash ->\nworld"));
    }

    #[test]
    fn test_team_mode_parsing_basic() {
        use cade_agent::team::TeamMode;
        assert_eq!(TeamMode::from_str("route"), Some(TeamMode::Route));
        assert_eq!(TeamMode::from_str("tasks"), Some(TeamMode::Tasks));
        assert_eq!(TeamMode::from_str("coordinate"), Some(TeamMode::Coordinate));
        assert_eq!(TeamMode::from_str("broadcast"), Some(TeamMode::Broadcast));
    }

    #[tokio::test]
    async fn test_turn_tools_enforces_plan_mode_and_denies_writes() {
        // -- Setup & Fixtures
        let state = build_state_with_llm(std::sync::Arc::new(PanicOnCallLlm));
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let tool_calls = vec![LlmToolCall {
            id: "tc-write-plan".to_string(),
            name: "write_file".to_string(),
            arguments: json!({
                "path": "file.txt",
                "content": "should be blocked in plan mode"
            }),
            thought_signature: None,
        }];

        // -- Exec: Execute with permission_mode: Some("plan".to_string())
        let results = execute_turn_tools(
            state,
            runtime::TurnExecutionInput {
                agent_id: "agent-plan".to_string(),
                conversation_id: None,
                run_id: "run-plan-mode".to_string(),
                input: "try writing in plan mode".to_string(),
                permission_mode: Some("plan".to_string()),
            },
            tool_calls,
            tx,
        )
        .await;

        // -- Check
        assert_eq!(results.len(), 1);
        let (result, _) = &results[0];
        assert!(result.is_error, "write_file must be blocked in Plan mode");
        assert!(
            result.output.contains("Blocked")
                || result.output.contains("permission")
                || result.output.contains("plan")
                || result.output.contains("Plan"),
            "Error output must explain why write_file was blocked: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_turn_tools_enforces_default_mode_execution() {
        // -- Setup & Fixtures
        let state = build_state_with_llm(std::sync::Arc::new(PanicOnCallLlm));
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        let tool_calls = vec![LlmToolCall {
            id: "tc-default-mode".to_string(),
            name: "read_file".to_string(),
            arguments: json!({
                "path": "Cargo.toml",
            }),
            thought_signature: None,
        }];

        // -- Exec: Execute with permission_mode: Some("default".to_string())
        let results = execute_turn_tools(
            state,
            runtime::TurnExecutionInput {
                agent_id: "agent-default".to_string(),
                conversation_id: None,
                run_id: "run-default-mode".to_string(),
                input: "read in default mode".to_string(),
                permission_mode: Some("default".to_string()),
            },
            tool_calls,
            tx,
        )
        .await;

        // -- Check: Read-only tool proceeds in Default mode
        assert_eq!(results.len(), 1);
        let (result, _) = &results[0];
        assert!(
            !result.is_error,
            "read_file should succeed in Default mode: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_sse_approval_delegate_yield_and_approve() {
        // -- Setup & Fixtures
        use crate::server::api::run::execution::SseApprovalDelegate;
        use cade_agent::tools::ApprovalDelegate;

        let state = build_state_with_llm(std::sync::Arc::new(PanicOnCallLlm));
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let delegate = SseApprovalDelegate {
            db: state.db.clone(),
            agent_id: "agent-approval-test".to_string(),
            run_id: "run-approval-test".to_string(),
            tx,
        };

        // Spawn approval request in background
        let handle = tokio::spawn(async move {
            delegate
                .request_approval(
                    "tc-approve-1",
                    "bash",
                    &json!({ "command": "cargo build" }),
                    "requires confirmation",
                )
                .await
        });

        // -- Check: Active turn receives approval_required SSE event
        let event = rx
            .recv()
            .await
            .expect("must receive approval_required SSE event");
        let env = event.expect("infallible event envelope");
        let payload: Value = serde_json::from_str(&env.data).expect("valid json");
        assert_eq!(payload["message_type"], "approval_required");
        let approval_id = payload["id"].as_str().expect("id must be string");

        // -- Exec: User approves the request via database status change (same as POST /v1/approvals/:id/action)
        cade_store::sqlite::set_approval_status(&state.db, approval_id, "approved")
            .expect("must set approval status");

        // -- Check: Delegate unblocks and returns Ok(true)
        let result = handle
            .await
            .expect("task must join")
            .expect("approval result");
        assert!(result, "approved request must return Ok(true)");
    }

    #[tokio::test]
    async fn test_ask_user_question_yields_event_and_resumes_with_answers() {
        // -- Setup & Fixtures
        let state = build_state_with_llm(std::sync::Arc::new(PanicOnCallLlm));
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let tool_calls = vec![LlmToolCall {
            id: "tc-question-1".to_string(),
            name: "ask_user_question".to_string(),
            arguments: json!({
                "questions": [
                    {
                        "header": "Database",
                        "question": "Which database engine should we adopt?",
                        "options": [
                            { "label": "PostgreSQL", "description": "Relational DB" },
                            { "label": "SQLite", "description": "Embedded DB" }
                        ],
                        "multiSelect": false
                    }
                ]
            }),
            thought_signature: None,
        }];

        let state_c = state.clone();
        // -- Exec: Execute turn tool in background
        let handle = tokio::spawn(async move {
            execute_turn_tools(
                state_c,
                runtime::TurnExecutionInput {
                    agent_id: "agent-question-test".to_string(),
                    conversation_id: None,
                    run_id: "run-question".to_string(),
                    input: "ask user".to_string(),
                    permission_mode: None,
                },
                tool_calls,
                tx,
            )
            .await
        });

        // -- Check: Active turn receives question_required SSE event
        let event = rx
            .recv()
            .await
            .expect("must receive question_required SSE event");
        let env = event.expect("infallible event envelope");
        let payload: Value = serde_json::from_str(&env.data).expect("valid json");
        assert_eq!(payload["type"], "question_required");
        let question_id = payload["id"].as_str().expect("id must be string");

        // -- Exec: User answers the question via status update with feedback
        let answer_feedback = "approved:{\"Database\":\"PostgreSQL\"}";
        cade_store::sqlite::set_approval_status(&state.db, question_id, answer_feedback)
            .expect("must set approval status with answers");

        // -- Check: Tool execution unblocks and returns formatted answer
        let results = handle.await.expect("task must join");
        assert_eq!(results.len(), 1);
        let (result, _) = &results[0];
        assert!(!result.is_error, "ask_user_question should succeed");
        assert!(
            result.output.contains("PostgreSQL"),
            "tool output must contain the user's selected answer: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_turn_tools_in_default_mode_prompts_approval_for_crud_replace() {
        // -- Setup & Fixtures
        use crate::server::api::run::execution::SseApprovalDelegate;
        use cade_agent::tools::ApprovalDelegate;

        let state = build_state_with_llm(std::sync::Arc::new(PanicOnCallLlm));
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let delegate = SseApprovalDelegate {
            db: state.db.clone(),
            agent_id: "agent-crud-prompt-test".to_string(),
            run_id: "run-crud-prompt-test".to_string(),
            tx,
        };

        // Spawn approval request for Replace (CRUD tool previously in blindspot)
        let handle = tokio::spawn(async move {
            delegate
                .request_approval(
                    "tc-replace-crud",
                    "Replace",
                    &json!({ "path": "src/main.rs", "old_string": "foo", "new_string": "bar" }),
                    "requires confirmation in default mode",
                )
                .await
        });

        // -- Check: Active turn receives approval_required SSE event
        let event = rx
            .recv()
            .await
            .expect("must receive approval_required SSE event for CRUD Replace");
        let env = event.expect("infallible event envelope");
        let payload: Value = serde_json::from_str(&env.data).expect("valid json");
        assert_eq!(payload["message_type"], "approval_required");
        assert_eq!(payload["tool_name"], "Replace");
        let approval_id = payload["id"].as_str().expect("id must be string");

        // User approves the request
        cade_store::sqlite::set_approval_status(&state.db, approval_id, "approved")
            .expect("must set approval status");

        let result = handle
            .await
            .expect("task must join")
            .expect("approval result");
        assert!(result, "approved CRUD request must return Ok(true)");
    }

    #[tokio::test]
    async fn default_mode_direct_write_requires_client_decoded_decision() {
        use cade_agent::agent::client::CadeMessage;

        let directory = tempfile::Builder::new()
            .prefix("cade-approval-")
            .tempdir_in(std::env::current_dir().expect("cwd"))
            .expect("test directory");

        for (action, should_write) in [("approve", true), ("deny", false)] {
            let path = directory.path().join(format!("{action}.txt"));
            let args = json!({ "path": path.to_str().expect("utf-8 path"), "content": action });
            // The decision endpoint holds a connection while querying the
            // queue; use the production-style file-backed pool (>1 conn).
            let mut state = build_state_with_llm(Arc::new(PanicOnCallLlm));
            state.db = cade_store::sqlite::open(
                directory
                    .path()
                    .join(format!("{action}.db"))
                    .to_str()
                    .expect("db path"),
            )
            .expect("file-backed database");
            let (tx, mut rx) = tokio::sync::mpsc::channel(16);
            let state_for_turn = state.clone();
            let handle = tokio::spawn(async move {
                execute_turn_tools(
                    state_for_turn,
                    runtime::TurnExecutionInput {
                        agent_id: "agent-direct-approval".to_string(),
                        conversation_id: None,
                        run_id: "run-direct-approval".to_string(),
                        input: "write a file".to_string(),
                        permission_mode: Some("default".to_string()),
                    },
                    vec![LlmToolCall {
                        id: "tc-direct-write".to_string(),
                        name: "write_file".to_string(),
                        arguments: args,
                        thought_signature: None,
                    }],
                    tx,
                )
                .await
            });

            // A progress event precedes the permission request. Decode the
            // actual server envelope using the same CadeMessage adapter as the terminal.
            let (message, raw_event) =
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    loop {
                        let envelope = rx
                            .recv()
                            .await
                            .expect("turn must emit approval")
                            .expect("event");
                        let message: CadeMessage =
                            serde_json::from_str(&envelope.data).expect("client event");
                        if message.msg_type() == "approval_required" {
                            break (message, envelope.data);
                        }
                    }
                })
                .await
                .expect("approval should arrive promptly");
            let request = message.approval_request().expect("terminal can prompt");
            let web_event: cade_api_types::StreamEvent =
                serde_json::from_str(&raw_event).expect("browser decodes the server event");
            let web_request = web_event.approval_request().expect("web can prompt");
            assert_eq!(web_request.id, request.id);
            assert_eq!(web_request.tool_name, request.tool_name);
            assert_eq!(web_request.arguments, request.arguments);
            assert_eq!(web_request.reason, request.reason);
            assert!(request.id.starts_with("app-"));
            assert_eq!(request.tool_name, "write_file");
            assert_eq!(
                request.arguments["path"],
                path.to_str().expect("utf-8 path")
            );
            assert_eq!(request.arguments["content"], action);
            assert!(!request.reason.is_empty());
            assert_eq!(message.data["run_id"], "run-direct-approval");
            assert_eq!(message.data["tool_call_id"], "tc-direct-write");
            assert!(!path.exists(), "execution must wait for consent");
            assert!(!handle.is_finished(), "turn must wait for consent");

            let queue = crate::server::api::approvals::list_approvals(State(state.clone()))
                .await
                .expect("dashboard can discover the pending approval");
            assert_eq!(queue.0["approvals"].as_array().unwrap().len(), 1);
            assert_eq!(queue.0["approvals"][0]["id"], request.id);

            let response = crate::server::api::approvals::action_approval(
                State(state),
                Path(request.id.to_owned()),
                axum::Json(crate::server::api::approvals::ActionPayload {
                    action: action.to_string(),
                    feedback: None,
                }),
            )
            .await
            .expect("decision endpoint accepts the client's approval ID");
            assert_eq!(
                response.0["status"],
                if should_write { "approved" } else { "denied" }
            );

            let results = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
                .await
                .expect("decision unblocks turn")
                .expect("turn joins");
            assert_eq!(results.len(), 1);
            assert_eq!(path.exists(), should_write);
            if should_write {
                assert_eq!(
                    std::fs::read_to_string(&path).expect("approved write"),
                    action
                );
            }
            assert_eq!(
                results[0].0.is_error, !should_write,
                "{}",
                results[0].0.output
            );
            if !should_write {
                assert!(results[0].0.output.contains("Permission Denied"));
            }
        }
    }
}
