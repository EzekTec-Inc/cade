//! P2-5: Origin-based CSRF middleware tests.

use super::{csrf_middleware, origin_is_allowed};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::{Router, middleware, routing};
use tower::ServiceExt;

// -- Pure policy

#[test]
fn origin_policy_accepts_bare_localhost_schemes() {
    assert!(origin_is_allowed("http://localhost"));
    assert!(origin_is_allowed("http://127.0.0.1"));
}

#[test]
fn origin_policy_accepts_any_port_on_allowed_hosts() {
    assert!(origin_is_allowed("http://localhost:8284"));
    assert!(origin_is_allowed("http://127.0.0.1:3000"));
    assert!(origin_is_allowed("http://localhost:1"));
}

#[test]
fn origin_policy_rejects_non_localhost() {
    // Classic CSRF origins.
    assert!(!origin_is_allowed("https://evil.com"));
    assert!(!origin_is_allowed("http://attacker.example"));
    // TLS on localhost — not in our CORS allow-list, so also rejected
    // by the CSRF middleware for consistency.
    assert!(!origin_is_allowed("https://localhost"));
    assert!(!origin_is_allowed("https://127.0.0.1:8284"));
    // Tricky prefix-confusion attempts.
    assert!(!origin_is_allowed("http://localhost.evil.com"));
    assert!(!origin_is_allowed("http://127.0.0.1.evil.com"));
    assert!(!origin_is_allowed("http://localhost-evil"));
    // Port must be numeric.
    assert!(!origin_is_allowed("http://localhost:abc"));
    assert!(!origin_is_allowed("http://localhost:"));
    // Scheme must be http (https on localhost isn't used by CADE).
    assert!(!origin_is_allowed("ftp://localhost"));
    assert!(!origin_is_allowed(""));
}

use crate::server::state::AppState;
use std::sync::Arc;

// -- Middleware integration

fn make_state(allowed_origin: Option<String>) -> AppState {
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
        ollama_base_url: String::new(),
        api_key: None,
        allowed_origin,
        max_context_budget: None,
    });

    AppState {
        subagent_cancellations: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        db,
        llm: std::sync::Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            llm_provider: String::new(),
        })),
        llm_router: std::sync::Arc::new(tokio::sync::RwLock::new(cade_ai::LlmRouter::build(
            &cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
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
        context_cache: std::sync::Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
            crate::server::state::CONTEXT_CACHE_CAPACITY,
        ))),
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

fn make_app(state: AppState) -> Router {
    async fn ok() -> &'static str {
        "ok"
    }
    Router::new()
        .route("/x", routing::get(ok).post(ok).put(ok).patch(ok).delete(ok))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, csrf_middleware))
}

async fn run_with_state(state: AppState, req: Request<Body>) -> StatusCode {
    make_app(state).oneshot(req).await.unwrap().status()
}

async fn run(req: Request<Body>) -> StatusCode {
    run_with_state(make_state(None), req).await
}

#[tokio::test]
async fn csrf_allows_custom_allowed_origin() {
    let state = make_state(Some("https://cade.mycompany.com".to_string()));
    let req = Request::builder()
        .method(Method::POST)
        .uri("/x")
        .header("Origin", "https://cade.mycompany.com")
        .body(Body::empty())
        .unwrap();
    assert_eq!(run_with_state(state, req).await, StatusCode::OK);
}

#[tokio::test]
async fn csrf_allows_post_with_allowed_origin() {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/x")
        .header("Origin", "http://localhost:8284")
        .body(Body::empty())
        .unwrap();
    assert_eq!(run(req).await, StatusCode::OK);
}

#[tokio::test]
async fn csrf_blocks_post_with_disallowed_origin() {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/x")
        .header("Origin", "https://evil.com")
        .body(Body::empty())
        .unwrap();
    assert_eq!(run(req).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn csrf_blocks_delete_with_disallowed_origin() {
    let req = Request::builder()
        .method(Method::DELETE)
        .uri("/x")
        .header("Origin", "https://attacker.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(run(req).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn csrf_passthrough_when_origin_absent() {
    // Non-browser clients (CADE CLI, curl, CI) don't send Origin.
    // Bearer-token auth is the real gate for them.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/x")
        .body(Body::empty())
        .unwrap();
    assert_eq!(run(req).await, StatusCode::OK);
}

#[tokio::test]
async fn csrf_does_not_block_get_even_with_hostile_origin() {
    // Safe methods never go through the CSRF gate.
    let req = Request::builder()
        .method(Method::GET)
        .uri("/x")
        .header("Origin", "https://evil.com")
        .body(Body::empty())
        .unwrap();
    assert_eq!(run(req).await, StatusCode::OK);
}

#[tokio::test]
async fn csrf_does_not_block_options_preflight() {
    // OPTIONS preflights must never be blocked — CORS handles them.
    let req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/x")
        .header("Origin", "https://evil.com")
        .body(Body::empty())
        .unwrap();
    // The stub router doesn't register OPTIONS explicitly; axum returns
    // 405 for unhandled methods.  What we care about is NOT 403 — the
    // CSRF gate must let OPTIONS through.
    assert_ne!(run(req).await, StatusCode::FORBIDDEN);
}
