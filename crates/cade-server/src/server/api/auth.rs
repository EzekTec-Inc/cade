use crate::server::state::AppState;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Bearer-token auth middleware.
///
/// If `CADE_API_KEY` is set (non-empty), every request must include:
///   `Authorization: Bearer <token>`
///
/// If the env var is not set, the middleware is a no-op (local dev mode).
/// The `/v1/health` endpoint is always allowed through without auth.
pub async fn auth_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // Health check is always public
    if req.uri().path() == "/v1/health" {
        return next.run(req).await;
    }

    let expected = match state.config.api_key.as_deref().filter(|k| !k.is_empty()) {
        Some(k) => k.to_string(),
        None => return next.run(req).await, // no key configured → open (local dev)
    };

    let provided = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    use subtle::ConstantTimeEq;
    match provided {
        Some(token)
            if token.len() == expected.len()
                && token.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() == 1 =>
        {
            next.run(req).await
        }
        _ => {
            tracing::warn!(
                "Unauthorized request to {} — missing or invalid Bearer token",
                req.uri().path()
            );
            (
                StatusCode::UNAUTHORIZED,
                [("WWW-Authenticate", "Bearer realm=\"cade-server\"")],
                "Unauthorized: missing or invalid API key",
            )
                .into_response()
        }
    }
}
