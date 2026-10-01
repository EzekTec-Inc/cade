//! Integration-style tests for router-level concerns (body limits, etc.).
//!
//! Unit tests specific to individual handlers live next to their handler
//! modules.

use crate::server::api::router;
use crate::server::state::AppState;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;

fn make_state(api_key: Option<String>) -> AppState {
    let db = cade_store::sqlite::open(":memory:").unwrap();

    let config = Arc::new(crate::server::config::ServerConfig {
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
        api_key,
        allowed_origin: None,
        max_context_budget: None,
    });

    AppState {
        permission_sessions: Default::default(),
        subagent_cancellations: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        db,
        llm: Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            deepseek_api_key: None,
            ollama_base_url: String::new(),
            llm_provider: String::new(),
        })),
        llm_router: Arc::new(RwLock::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            deepseek_api_key: None,
            ollama_base_url: String::new(),
            llm_provider: String::new(),
        }))),
        config,
        mcp: Arc::new(crate::server::state::McpManager::empty()),
        rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
        memory_cache: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        agent_activity: Arc::new(RwLock::new(std::collections::HashMap::new())),
        agent_metrics: Arc::new(dashmap::DashMap::new()),
        agent_context_telemetry: Arc::new(RwLock::new(std::collections::HashMap::new())),
        context_cache: Arc::new(parking_lot::Mutex::new(
            crate::server::state::SafeLruCache::new(crate::server::state::CONTEXT_CACHE_CAPACITY),
        )),
        all_skills: Arc::new(RwLock::new(Vec::new())),
        agent_skills: Arc::new(RwLock::new(std::collections::HashMap::new())),
        pending_subagent_results: Arc::new(RwLock::new(std::collections::HashMap::new())),
        subagent_semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
        embedder: None,
    }
}

/// RED test for P1-2: a global 8 MiB request body size limit must be set.
///
/// Axum's default `Json` extractor limit is 2 MiB, which is fine for most
/// handlers but leaves streaming / raw-body handlers uncapped.  P1-2
/// applies `DefaultBodyLimit::max(8 MiB)` at the router level so the cap is
/// explicit and applies uniformly.
///
/// This test proves the limit is exactly 8 MiB by sending a body just over
/// Axum's implicit 2 MiB default.  Before the fix the request fails with
/// 413 (Axum's default).  After the fix the router accepts it (our
/// explicit layer supersedes the 2 MiB default) and the handler sees it.
#[tokio::test]
async fn body_between_2mib_and_8mib_is_accepted() {
    let state = make_state(Some("tok".into()));
    let app = router(state);

    // 3 MiB — over Axum's default 2 MiB, under our intended 8 MiB cap.
    // Use an array body (valid JSON) so the Json<Value> extractor can parse it
    // cheaply: `[` + 3 MiB of `x` would not parse, so we build valid JSON.
    let filler = "a".repeat(3 * 1024 * 1024 - 3);
    let body = format!(r#""{filler}""#); // quoted JSON string

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/agents")
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "3 MiB body (between Axum default and our 8 MiB cap) must not be rejected as too large"
    );
}

/// Bodies over the explicit 8 MiB cap must still be rejected with 413.
#[tokio::test]
async fn oversized_request_body_is_rejected_with_413() {
    let state = make_state(Some("tok".into()));
    let app = router(state);

    // 10 MiB payload — well over the 8 MiB cap.
    let huge = vec![b'x'; 10 * 1024 * 1024];

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/agents")
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from(huge))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "requests over the 8 MiB cap must return 413"
    );
}

/// Small bodies still pass through the body-size layer.
#[tokio::test]
async fn small_request_body_is_accepted() {
    let state = make_state(Some("tok".into()));
    let app = router(state);

    let small = b"{}".to_vec();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/agents")
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from(small))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "small bodies must pass the body-size check"
    );
}

/// End-to-end: /dashboard is reachable through the real production router
/// (with auth, CSRF, and body-limit layers all active) without any
/// Authorization header.  Covers middleware-ordering regressions that
/// per-handler unit tests in dashboard_test.rs cannot catch.
#[tokio::test]
async fn dashboard_is_reachable_through_full_router_without_auth() {
    let state = make_state(Some("tok".into()));
    let app = router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "/dashboard must be reachable through the full router without a token"
    );
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("text/html"), "expected HTML, got {ct}");
}

/// Dashboard asset wildcard route is reachable through the full production
/// router without auth.  Returns 404 for a non-existent file (proving auth
/// was skipped — a 401 would mean the middleware blocked it).
#[tokio::test]
async fn dashboard_asset_wildcard_is_reachable_through_full_router_without_auth() {
    let state = make_state(Some("tok".into()));
    let app = router(state);

    let req = Request::builder()
        .method(Method::GET)
        .uri("/dashboard/nonexistent.js")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "/dashboard/* must be auth-exempt (expected 404 for missing asset, got {})",
        resp.status()
    );
}

#[tokio::test]
async fn test_workflow_dispatch_path_traversal_rejected() {
    let state = make_state(Some("tok".into()));
    let app = router(state);

    // Traversal or nested paths should be rejected with 400 Bad Request
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/workflows/..%2Fsecrets")
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"issueNumber": 42}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_workflow_dispatch_missing_not_found() {
    let state = make_state(Some("tok".into()));
    let app = router(state);

    // Non-existent workflow config should return 404 Not Found
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/workflows/non_existent_workflow")
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"issueNumber": 42}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_workflow_dispatch_success_with_config() {
    let state = make_state(Some("tok".into()));
    let app = router(state);

    // Create a temporary workflow config file on disk
    let workflows_dir = std::path::Path::new(".cade/workflows");
    std::fs::create_dir_all(workflows_dir).unwrap();
    let config_path = workflows_dir.join("test_success_workflow.json");
    std::fs::write(
        &config_path,
        r#"{
            "name": "test_success_workflow",
            "agent": "test-agent",
            "model": "openai/gpt-4o",
            "prompt": "Hello world"
        }"#,
    )
    .unwrap();

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/workflows/test_success_workflow")
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"issueNumber": 42}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // Clean up
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn test_workflow_dispatch_runs_background_execution() {
    use crate::server::workflows::WorkflowEngine;
    use cade_ai::{CompletionRequest, CompletionResponse, LlmProvider, StreamChunk};
    use cade_api_types::WorkflowStatus;
    use cade_store::sqlite;

    struct ControlledWorkflowProvider {
        started: tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<CompletionRequest>>>,
        release: Arc<tokio::sync::Notify>,
    }
    #[async_trait::async_trait]
    impl LlmProvider for ControlledWorkflowProvider {
        async fn complete(&self, _: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
            Err(cade_ai::Error::custom(
                "Workflow fixture exercises the streaming runtime",
            ))
        }
        async fn stream(
            &self,
            request: &CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<Box<dyn futures::Stream<Item = cade_ai::Result<StreamChunk>> + Send>>,
        > {
            self.started
                .lock()
                .await
                .take()
                .expect("Exactly one canonical agent turn")
                .send(request.clone())
                .unwrap();
            let release = self.release.clone();
            Ok(Box::pin(futures::StreamExt::chain(
                futures::stream::once(async move {
                    release.notified().await;
                    Ok(StreamChunk::Text("actual legacy workflow outcome".into()))
                }),
                futures::stream::iter([Ok(StreamChunk::Done)]),
            )))
        }
    }
    struct WorkflowFixture(std::path::PathBuf);
    impl Drop for WorkflowFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let release = Arc::new(tokio::sync::Notify::new());
    let mut state = make_state(Some("tok".into()));
    state.llm = Arc::new(ControlledWorkflowProvider {
        started: tokio::sync::Mutex::new(Some(started_tx)),
        release: release.clone(),
    });
    let app = router(state.clone());
    let name = format!("test_background_workflow_{}", uuid::Uuid::new_v4());
    let agent_id = format!("agent-workflow-{name}");

    // Exercise the supported legacy file format, without overwriting another
    // test's or the user's workflow and without mutating process cwd.
    let workflows_dir = std::path::Path::new(".cade/workflows");
    std::fs::create_dir_all(workflows_dir).unwrap();
    let fixture = WorkflowFixture(workflows_dir.join(format!("{name}.json")));
    std::fs::write(
        &fixture.0,
        serde_json::json!({
            "name": name, "agent": "test-background-agent",
            "model": state.config.default_model,
            "prompt": "Test background system prompt"
        })
        .to_string(),
    )
    .unwrap();

    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/workflows/{name}"))
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"issueNumber": 42}"#))
        .unwrap();

    // Returning an accepted run must not wait for provider completion.
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), app.clone().oneshot(req))
        .await
        .expect("Webhook must return while the provider is still blocked")
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let body_bytes = axum::body::to_bytes(resp.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let execution_id = body_json["execution_id"].as_str().unwrap();
    let workflow_run_id = body_json["run_id"].as_str().unwrap();

    // The legacy execution_id is a canonical agent run, already durable when
    // the webhook acknowledges it. The DAG's aggregate ID is a separate field.
    let run = sqlite::get_run(&state.db, execution_id)
        .unwrap()
        .expect("execution_id must identify an accepted canonical agent run");
    assert_eq!(run.agent_id, agent_id);
    assert_eq!(run.status, "running");
    assert_ne!(execution_id, workflow_run_id);
    assert_eq!(body_json["agent_id"], agent_id);
    assert_eq!(body_json["status"], "triggered");
    let record = sqlite::get_workflow_run(&state.db, workflow_run_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.workflow_name, name);
    assert_eq!(record.status, "running");

    let completion_request = tokio::time::timeout(std::time::Duration::from_secs(10), started_rx)
        .await
        .expect("Canonical runtime must reach the controlled provider")
        .unwrap();
    assert_eq!(completion_request.model, state.config.default_model);
    assert!(
        completion_request
            .messages
            .iter()
            .any(|message| message.content.contains("Test background system prompt"))
    );
    assert!(
        completion_request.messages.iter().any(
            |message| message.content.contains("issueNumber") && message.content.contains("42")
        )
    );
    let mut events = WorkflowEngine::new(state.clone())
        .subscribe_events(workflow_run_id)
        .await
        .unwrap();
    release.notify_one();
    let terminal_events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut received = Vec::new();
        while let Ok(event) = events.recv().await {
            received.push(event);
        }
        received
    })
    .await
    .expect("Workflow must finalize from the actual provider outcome");
    assert!(
        terminal_events
            .iter()
            .any(|event| event.status == WorkflowStatus::Succeeded
                && event.output_chunk.as_deref() == Some("actual legacy workflow outcome"))
    );
    assert!(!terminal_events.iter().any(|event| matches!(
        event.status,
        WorkflowStatus::Failed | WorkflowStatus::Cancelled
    )));
    assert_eq!(
        sqlite::get_run(&state.db, execution_id)
            .unwrap()
            .unwrap()
            .status,
        "done"
    );
    let record = sqlite::get_workflow_run(&state.db, workflow_run_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.status, "succeeded");
    assert!(record.completed_at.is_some());
    let runs = sqlite::list_agent_runs(&state.db, &agent_id, 10).unwrap();
    assert_eq!(
        runs.len(),
        1,
        "One canonical run for the legacy webhook step"
    );
    let messages =
        sqlite::list_messages(&state.db, &agent_id, run.conversation_id.as_deref(), 20).unwrap();
    assert!(messages.iter().any(|message| message.role == "assistant"
        && message.content["content"] == "actual legacy workflow outcome"));

    let missing = Request::builder()
        .method(Method::POST)
        .uri(format!("/v1/workflows/undefined_{name}"))
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    assert_eq!(
        app.oneshot(missing).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        sqlite::list_workflow_runs(&state.db, None, 10)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn test_create_conversation_fork() {
    let state = make_state(Some("tok".into()));
    let db = state.db.clone();

    // Seed agent
    cade_store::sqlite::agents::create_agent(
        &db,
        &cade_store::sqlite::AgentRow {
            id: "test_agent".into(),
            name: "A".into(),
            model: "m".into(),
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

    // Seed parent conversation and some messages
    let parent_conv = cade_store::sqlite::create_conversation(&db, "test_agent", "Parent").unwrap();
    let msg1 = cade_store::sqlite::MessageRow {
        id: "msg1".to_string(),
        agent_id: "test_agent".to_string(),
        conversation_id: Some(parent_conv.id.clone()),
        role: "user".to_string(),
        content: serde_json::json!("Hello"),
        char_count: 5,
    };
    cade_store::sqlite::insert_message(&db, &msg1).unwrap();

    let app = router(state);

    // Request fork of parent conversation
    let body = serde_json::json!({
        "title": "Forked Chat",
        "parent_id": parent_conv.id
    });

    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/agents/test_agent/conversations")
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(resp.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let new_conv_id = body_json["id"].as_str().unwrap();

    // Verify that messages were cloned
    let cloned_messages =
        cade_store::sqlite::list_messages(&db, "test_agent", Some(new_conv_id), 10).unwrap();
    assert_eq!(cloned_messages.len(), 1);
    assert_eq!(cloned_messages[0].role, "user");
    assert_eq!(cloned_messages[0].content.as_str().unwrap(), "Hello");
    assert_ne!(cloned_messages[0].id, "msg1"); // ID must be newly generated
}

#[tokio::test]
async fn test_stream_message_delegates_to_canonical_server_agent_runtime() {
    let state = make_state(Some("tok".to_string()));
    let db = state.db.clone();

    // Create an agent record
    let agent = cade_store::sqlite::AgentRow {
        id: "compat_agent".to_string(),
        name: "Compat Agent".to_string(),
        description: None,
        model: "test_model".to_string(),
        system_prompt: None,
        created_at: None,
        compaction_model: None,
        theme: None,
        active_plan_json: None,
        parent_id: None,
    };
    cade_store::sqlite::create_agent(&db, &agent).unwrap();

    let app = router(state);

    // Call compatibility route: POST /v1/agents/:id/messages/stream
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/agents/compat_agent/messages/stream")
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "input": "test prompt for compatibility runtime delegation"
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );

    // Read streamed events from canonical runtime
    let body_bytes = axum::body::to_bytes(resp.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let body_text = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_text.contains("[DONE]"));

    // Verify durable state created by canonical runtime
    let runs = cade_store::sqlite::list_agent_runs(&db, "compat_agent", 10).unwrap();
    assert_eq!(
        runs.len(),
        1,
        "canonical ServerAgentRuntime must create exactly one durable run"
    );
    assert_eq!(runs[0].agent_id, "compat_agent");

    // Verify user message was persisted by canonical runtime
    let msgs = cade_store::sqlite::list_messages(&db, "compat_agent", None, 10).unwrap();
    assert!(
        !msgs.is_empty(),
        "user message must be persisted by ServerAgentRuntime"
    );
    assert_eq!(msgs[0].role, "user");
}

#[test]
fn test_architecture_presentation_adapters_do_not_own_server_or_ai() {
    let tui_cargo = include_str!("../../../../../crates/cade-tui/Cargo.toml");
    assert!(
        !tui_cargo.contains("cade-server"),
        "cade-tui must never depend on cade-server"
    );
    assert!(
        !tui_cargo.contains("cade-ai"),
        "cade-tui must never depend on cade-ai"
    );
    assert!(
        !tui_cargo.contains("cade-store"),
        "cade-tui must never depend on cade-store"
    );

    let api_types_cargo = include_str!("../../../../../crates/cade-api-types/Cargo.toml");
    assert!(
        !api_types_cargo.contains("cade-server"),
        "cade-api-types must never depend on cade-server"
    );
    assert!(
        !api_types_cargo.contains("cade-agent"),
        "cade-api-types must never depend on cade-agent"
    );
}
