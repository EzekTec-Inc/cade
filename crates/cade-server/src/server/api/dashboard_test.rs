//! Tests for the /dashboard route (M1 → M8: now serves embedded WASM assets).
//!
//! Security contract under test:
//! - `GET /dashboard` returns 200 with HTML content even with no Authorization
//!   header and no `api_key` configured (unauth-exempt alongside /v1/health).
//! - The response body must NOT contain the server's api_key, any bearer
//!   token, or any stack trace / framework version string.  Users paste
//!   their key into the egui login form; the WASM app holds it in memory only.

use crate::server::api::auth::auth_middleware;
use crate::server::state::AppState;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::{Router, middleware, routing::get};
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
        ollama_base_url: String::new(),
        api_key,
        allowed_origin: None,
        max_context_budget: None,
    });
    AppState {
        subagent_cancellations: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
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
        agent_metrics: Arc::new(dashmap::DashMap::new()),
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

/// Minimal app that mounts both dashboard routes behind the real auth
/// middleware, matching the path-skip wiring the production router uses.
fn make_app(state: AppState) -> Router {
    Router::new()
        .route("/dashboard", get(super::get_dashboard))
        .route("/dashboard/{*path}", get(super::get_dashboard_asset))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, auth_middleware))
}

// ── index.html serving ──────────────────────────────────────────────

#[tokio::test]
async fn dashboard_returns_html_page_without_auth() {
    let app = make_app(make_state(Some("super-secret-key".into())));
    let req = Request::builder()
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "dashboard must be public (unauthenticated) so browsers can load the page"
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

    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body_str = std::str::from_utf8(&body).expect("utf-8 html");
    assert!(
        body_str.contains("<html"),
        "response must be an HTML document"
    );
}

#[tokio::test]
async fn dashboard_does_not_leak_server_api_key() {
    let secret = "leaky-token-DO-NOT-LEAK";
    let app = make_app(make_state(Some(secret.into())));

    let req = Request::builder()
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body_str = std::str::from_utf8(&body).unwrap();

    assert!(
        !body_str.contains(secret),
        "api_key must NEVER appear in dashboard HTML (this would leak it \
         to anyone who can GET /dashboard)"
    );
}

#[tokio::test]
async fn dashboard_error_page_has_no_stack_trace_or_framework_info() {
    let app = make_app(make_state(None));
    let req = Request::builder()
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
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
/// renamed canvas = white screen for every user.
#[tokio::test]
async fn dashboard_contains_canvas_with_expected_id() {
    let app = make_app(make_state(None));
    let req = Request::builder()
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body_str = std::str::from_utf8(&body).unwrap();

    assert!(
        body_str.contains(r#"id="cade_gui_canvas""#),
        "dashboard HTML must expose <canvas id=\"cade_gui_canvas\"> for the \
         cade-gui WASM client to mount on"
    );
}

/// The index.html sets `Cache-Control: no-cache` so browsers always
/// fetch the latest asset hashes after a rebuild.
#[tokio::test]
async fn dashboard_index_html_has_no_cache_header() {
    let app = make_app(make_state(None));
    let req = Request::builder()
        .uri("/dashboard")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let cc = resp
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(cc, "no-cache", "index.html must be no-cache");
}

// ── Asset serving ───────────────────────────────────────────────────

/// Requesting a non-existent asset returns 404, not 500.
#[tokio::test]
async fn dashboard_missing_asset_returns_404() {
    let app = make_app(make_state(None));
    let req = Request::builder()
        .uri("/dashboard/does-not-exist.js")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "missing dashboard asset should be 404, not 500"
    );
}

/// Dashboard assets must be served without an Authorization header
/// (same exemption as /dashboard itself).
#[tokio::test]
async fn dashboard_assets_do_not_require_auth() {
    let app = make_app(make_state(Some("secret-key".into())));
    // Request a non-existent asset — we care about the status (404 = unblocked
    // by auth), not the body.
    let req = Request::builder()
        .uri("/dashboard/some-file.wasm")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // If auth blocked us we'd get 401; 404 proves auth was skipped.
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "dashboard assets must be auth-exempt (got {} instead of 404 for missing asset)",
        resp.status()
    );
}

// ── MIME type inference ─────────────────────────────────────────────

#[test]
fn mime_for_returns_correct_types() {
    use super::mime_for;
    assert_eq!(mime_for("index.html"), "text/html; charset=utf-8");
    assert_eq!(mime_for("cade-gui-abc123.js"), "text/javascript");
    assert_eq!(mime_for("cade-gui-abc123_bg.wasm"), "application/wasm");
    assert_eq!(mime_for("style.css"), "text/css; charset=utf-8");
    assert_eq!(mime_for("data.json"), "application/json");
    assert_eq!(mime_for("logo.png"), "image/png");
    assert_eq!(mime_for("icon.svg"), "image/svg+xml");
    assert_eq!(mime_for("favicon.ico"), "image/x-icon");
    assert_eq!(mime_for("unknown.xyz"), "application/octet-stream");
}

#[tokio::test]
async fn dashboard_existing_asset_returns_200() {
    let app = make_app(make_state(None));
    let req = Request::builder()
        .uri("/dashboard/cade-gui-ee9a23395c7c3ad2.js")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "existing dashboard asset must return 200 OK"
    );
}
