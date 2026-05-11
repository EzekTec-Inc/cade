use crate::server::state::AppState;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Bearer-token auth middleware.
///
/// Every request (except `/v1/health`) must include:
///   `Authorization: Bearer <token>`
///
/// The expected token comes from `config.api_key`.  When no token is
/// configured, all non-health requests are rejected with 401 — auth is
/// mandatory; the server bootstrap is responsible for providing a token.
pub async fn auth_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    // Public routes — served before auth check.
    //   /v1/health         — health probe (safe, no state touched)
    //   /dashboard         — static login page for the web GUI (M1).
    //                        The page itself never embeds the api_key;
    //                        the user pastes their key into the form.
    //   /dashboard/*       — future WASM asset bundle (M2+).
    let path = req.uri().path();
    if path == "/v1/health" || path == "/dashboard" || path.starts_with("/dashboard/") {
        return next.run(req).await;
    }

    let expected = match state.config.api_key.as_deref().filter(|k| !k.is_empty()) {
        Some(k) => k.to_string(),
        None => {
            tracing::warn!(
                "Unauthorized request to {} — server has no api_key configured",
                req.uri().path()
            );
            return (
                StatusCode::UNAUTHORIZED,
                [("WWW-Authenticate", "Bearer realm=\"cade-server\"")],
                "Unauthorized: server has no api_key configured",
            )
                .into_response();
        }
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

#[cfg(test)]
#[path = "auth_test.rs"]
mod tests;
