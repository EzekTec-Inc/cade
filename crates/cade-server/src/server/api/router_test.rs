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
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let db = Arc::new(parking_lot::Mutex::new(conn));

    let config = Arc::new(crate::server::config::ServerConfig {
        addr: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        llm_provider: crate::server::config::LlmProviderKind::Anthropic,
        default_model: "test".into(),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: String::new(),
        api_key,
        allowed_origin: None,
        max_context_budget: None,
    });

    AppState {
        db,
        llm: Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            llm_provider: String::new(),
        })),
        llm_router: Arc::new(RwLock::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            llm_provider: String::new(),
        }))),
        config,
        mcp: Arc::new(crate::server::state::McpManager::empty()),
        rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
        memory_cache: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        agent_activity: Arc::new(RwLock::new(std::collections::HashMap::new())),
        agent_metrics: Arc::new(RwLock::new(std::collections::HashMap::new())),
        agent_context_telemetry: Arc::new(RwLock::new(std::collections::HashMap::new())),
        context_cache: Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
            crate::server::state::CONTEXT_CACHE_CAPACITY,
        ))),
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
