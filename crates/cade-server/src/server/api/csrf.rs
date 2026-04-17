//! P2-5: Origin-based CSRF check.
//!
//! Defense in depth on top of bearer-token auth.  Mutating methods
//! (POST / PUT / PATCH / DELETE) whose `Origin` header is set must
//! have an origin on the localhost allow-list.  Requests with no
//! `Origin` header pass through — they are non-browser clients (the
//! CADE CLI, curl, CI harnesses) and the bearer token is the real gate.
//! Safe methods (GET / HEAD / OPTIONS) are never subject to this check.

use axum::{
    extract::Request,
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Pure, deterministic policy.  Returns `true` if `origin` matches one
/// of the two localhost schemes on any port (or on no port at all —
/// `http://localhost` and `http://127.0.0.1` bare are both allowed,
/// matching the existing tower-http CORS allow-list in
/// `src/bin/cade-server.rs`).
pub(crate) fn origin_is_allowed(origin: &str) -> bool {
    // Accept bare hosts and any :PORT suffix.
    const ALLOWED_PREFIXES: &[&str] = &["http://localhost", "http://127.0.0.1"];

    for prefix in ALLOWED_PREFIXES {
        if origin == *prefix {
            return true;
        }
        if let Some(rest) = origin.strip_prefix(prefix) {
            // Must start with ':' (port) and contain only port digits.
            if let Some(port_str) = rest.strip_prefix(':')
                && !port_str.is_empty()
                && port_str.chars().all(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
    }
    false
}

/// Returns `true` for methods that can mutate state and therefore
/// require the CSRF check.  Safe methods (GET / HEAD / OPTIONS) bypass.
fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// Axum middleware that enforces the Origin allow-list on mutating
/// requests.  See the module-level doc comment for the exact contract.
pub async fn csrf_middleware(req: Request, next: Next) -> Response {
    if !is_mutating(req.method()) {
        return next.run(req).await;
    }

    // Browsers always send `Origin` on cross-origin mutating requests.
    // Absent header ⇒ non-browser client ⇒ pass through (bearer auth
    // is the gate for those).
    let origin = match req
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        Some(o) => o.to_string(),
        None => return next.run(req).await,
    };

    if origin_is_allowed(&origin) {
        return next.run(req).await;
    }

    tracing::warn!(
        method = %req.method(),
        path = %req.uri().path(),
        origin = %origin,
        "CSRF: blocked mutating request from disallowed Origin"
    );
    (
        StatusCode::FORBIDDEN,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        r#"{"error":"forbidden","reason":"origin not allowed"}"#,
    )
        .into_response()
}

#[cfg(test)]
#[path = "csrf_test.rs"]
mod tests;
