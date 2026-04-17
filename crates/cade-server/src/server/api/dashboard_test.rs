//! Tests for the /dashboard route (M1).
//!
//! Security contract under test:
//! - `GET /dashboard` returns 200 with HTML content even with no Authorization
//!   header and no `api_key` configured (unauth-exempt alongside /v1/health).
//! - The response body must NOT contain the server's api_key, any bearer
//!   token, or any stack trace / framework version string.  Users paste
//!   their key into the page; it is never embedded by the server.

use crate::server::api::auth::auth_middleware;
use crate::server::state::AppState;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::{Router, middleware, routing::get};
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;
use tower::ServiceExt;

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
        rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
        memory_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_activity: Arc::new(RwLock::new(std::collections::HashMap::new())),
        agent_metrics: Arc::new(RwLock::new(std::collections::HashMap::new())),
    }
}

/// Minimal app that mounts the dashboard route behind the real auth middleware,
/// matching the path-skip wiring the production router uses.
fn make_app(state: AppState) -> Router {
    Router::new()
        .route("/dashboard", get(super::get_dashboard))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, auth_middleware))
}

#[tokio::test]
async fn dashboard_returns_html_login_page_without_auth() {
    let app = make_app(make_state(Some("super-secret-key".into())));
    let req = Request::builder()
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "dashboard must be public (unauthenticated) so browsers can load the login page"
    );

    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("text/html"),
        "dashboard must return HTML, got content-type: {ct}"
    );

    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let body_str = std::str::from_utf8(&body).expect("utf-8 html");
    assert!(
        body_str.contains("<html"),
        "response must be an HTML document"
    );
}

#[tokio::test]
async fn dashboard_does_not_leak_server_api_key() {
    // Security-critical: even though the route is auth-exempt, it must
    // never embed the configured bearer token into the served HTML.
    let secret = "leaky-token-DO-NOT-LEAK";
    let app = make_app(make_state(Some(secret.into())));

    let req = Request::builder()
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let body_str = std::str::from_utf8(&body).unwrap();

    assert!(
        !body_str.contains(secret),
        "api_key must NEVER appear in dashboard HTML (this would leak it \
         to anyone who can GET /dashboard)"
    );
}

#[tokio::test]
async fn dashboard_error_page_has_no_stack_trace_or_framework_info() {
    // tdd-guide §3.3 — public-facing error responses must not leak
    // internals. Exercise the happy path too: the served HTML must not
    // include obvious framework/version fingerprints.
    let app = make_app(make_state(None));
    let req = Request::builder()
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let body_str = std::str::from_utf8(&body).unwrap();

    for forbidden in ["axum/", "tokio-", "panicked at", "RUST_BACKTRACE"] {
        assert!(
            !body_str.contains(forbidden),
            "dashboard HTML leaks internal info fragment: {forbidden}"
        );
    }
}

/// The dashboard page must carry a `<canvas id="cade_gui_canvas">` element
/// that matches the ID the cade-gui WASM boot code looks up.  Missing or
/// renamed canvas = white screen for every user.  This is the cheap
/// contract test that locks the two sides together.
#[tokio::test]
async fn dashboard_contains_canvas_with_expected_id() {
    let app = make_app(make_state(None));
    let req = Request::builder()
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let body_str = std::str::from_utf8(&body).unwrap();

    assert!(
        body_str.contains(r#"id="cade_gui_canvas""#),
        "dashboard HTML must expose <canvas id=\"cade_gui_canvas\"> for the \
         cade-gui WASM client to mount on"
    );
}
