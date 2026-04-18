use crate::server::api::auth::auth_middleware;
use crate::server::state::AppState;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::{Router, middleware, routing::get};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tower::ServiceExt; // for `oneshot`

/// Build a minimal AppState for middleware tests.
/// `api_key` controls the auth token configured on the server.
fn make_state(api_key: Option<String>) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let db = Arc::new(Mutex::new(conn));

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
        memory_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_activity: Arc::new(RwLock::new(std::collections::HashMap::new())),
        agent_metrics: Arc::new(RwLock::new(std::collections::HashMap::new())),
    }
}

/// Build a Router that wraps `auth_middleware` around trivial handlers so we
/// can exercise the middleware end-to-end with `oneshot`.
fn make_app(state: AppState) -> Router {
    async fn ok() -> &'static str { "passthrough" }
    Router::new()
        .route("/v1/health", get(ok))
        .route("/v1/agents", get(ok))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, auth_middleware))
}

// -- RED test for P1-1: mandatory auth
//
// Before the fix, api_key=None made the middleware a no-op — any request
// passed through.  After the fix, api_key=None must still reject anonymous
// requests to non-health routes with 401.

#[tokio::test]
async fn auth_rejects_anonymous_when_api_key_unset() {
    let app = make_app(make_state(None));
    let req = Request::builder()
        .uri("/v1/agents")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "anonymous requests must be rejected even when CADE_API_KEY is unset"
    );
}

#[tokio::test]
async fn auth_allows_health_even_when_api_key_unset() {
    let app = make_app(make_state(None));
    let req = Request::builder()
        .uri("/v1/health")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn auth_accepts_valid_bearer_token() {
    let app = make_app(make_state(Some("s3cret".into())));
    let req = Request::builder()
        .uri("/v1/agents")
        .header("Authorization", "Bearer s3cret")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 1024).await.unwrap();
    assert_eq!(&body[..], b"passthrough");
}

#[tokio::test]
async fn auth_rejects_wrong_bearer_token() {
    let app = make_app(make_state(Some("s3cret".into())));
    let req = Request::builder()
        .uri("/v1/agents")
        .header("Authorization", "Bearer wrong")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
