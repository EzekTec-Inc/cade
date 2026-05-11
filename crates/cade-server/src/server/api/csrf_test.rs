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

// -- Middleware integration

fn make_app() -> Router {
    async fn ok() -> &'static str {
        "ok"
    }
    Router::new()
        .route("/x", routing::get(ok).post(ok).put(ok).patch(ok).delete(ok))
        .layer(middleware::from_fn(csrf_middleware))
}

async fn run(req: Request<Body>) -> StatusCode {
    make_app().oneshot(req).await.unwrap().status()
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
