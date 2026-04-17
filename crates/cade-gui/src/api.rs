//! Pure API-client helpers for the cade-gui WASM app.
//!
//! This module contains **no browser dependencies**.  It handles:
//!   * Building absolute request URLs from `base_url + path`.
//!   * Building the `Authorization: Bearer <token>` header value.
//!   * Parsing JSON response bodies into the `cade-api-types` wire types.
//!   * Classifying HTTP status codes into a small typed error enum.
//!
//! The actual network I/O (gloo-net / fetch) lives in `http_wasm.rs` and is
//! compiled only for `wasm32`.  Keeping the logic here pure means native
//! `cargo test` covers URL building, header construction, JSON parsing, and
//! error classification without a browser.

use cade_api_types::{AgentInfo, HealthInfo};

/// Typed error surface for API calls.  The wasm fetch wrapper produces
/// `Transport`; the pure logic here produces `Unauthorized`, `Server`, or
/// `Decode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 401 Unauthorized — token is missing or wrong.
    Unauthorized,
    /// 5xx or any non-2xx/non-401 response.  Carries the status code.
    Server { status: u16 },
    /// JSON body did not match the expected wire type.
    Decode { message: String },
    /// Network-level failure (wasm-side only; surfaced here for uniformity).
    Transport { message: String },
}

impl core::fmt::Display for ApiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "unauthorized"),
            Self::Server { status } => write!(f, "server error (status {status})"),
            Self::Decode { message } => write!(f, "decode error: {message}"),
            Self::Transport { message } => write!(f, "transport error: {message}"),
        }
    }
}

impl std::error::Error for ApiError {}

/// Build the absolute URL for an API path.
///
/// Rules:
///   * `base` may or may not end with `/`.  Both forms must produce the
///     same result.
///   * `path` must start with `/`; callers supply server-relative paths.
///   * No query-string handling here — callers that need `?foo=bar` pass
///     it as part of `path`.
pub fn build_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}{path}")
}

/// Build the value for the `Authorization` header.
///
/// The returned string is always of the form `"Bearer <token>"`.  Callers
/// are responsible for trimming the token before passing it in; this is a
/// zero-logic helper so it can be inlined.
pub fn bearer_header(token: &str) -> String {
    format!("Bearer {token}")
}

/// Map an HTTP status code + body into either a parsed value or a typed
/// error.  Keeps the pure logic together so wasm and native paths share it.
pub fn parse_health(status: u16, body: &str) -> Result<HealthInfo, ApiError> {
    decode_or_error(status, body)
}

/// Same as `parse_health`, but for the `GET /v1/agents` list.
pub fn parse_agents(status: u16, body: &str) -> Result<Vec<AgentInfo>, ApiError> {
    decode_or_error(status, body)
}

fn decode_or_error<T>(status: u16, body: &str) -> Result<T, ApiError>
where
    T: serde::de::DeserializeOwned,
{
    match status {
        200..=299 => serde_json::from_str::<T>(body).map_err(|e| ApiError::Decode {
            message: e.to_string(),
        }),
        401 => Err(ApiError::Unauthorized),
        s => Err(ApiError::Server { status: s }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- build_url

    #[test]
    fn build_url_joins_base_and_path() {
        assert_eq!(
            build_url("http://localhost:8284", "/v1/health"),
            "http://localhost:8284/v1/health"
        );
    }

    #[test]
    fn build_url_strips_single_trailing_slash() {
        assert_eq!(
            build_url("http://localhost:8284/", "/v1/health"),
            "http://localhost:8284/v1/health"
        );
    }

    #[test]
    fn build_url_strips_multiple_trailing_slashes() {
        // `trim_end_matches` collapses runs — keeps normalisation predictable.
        assert_eq!(
            build_url("http://x///", "/v1/agents"),
            "http://x/v1/agents"
        );
    }

    // -- bearer_header

    #[test]
    fn bearer_header_formats_bearer_prefix() {
        assert_eq!(bearer_header("abc"), "Bearer abc");
    }

    #[test]
    fn bearer_header_does_not_trim() {
        // Upstream code is responsible for trimming; this helper is literal so
        // the caller cannot accidentally lose the prefix or suffix.
        assert_eq!(bearer_header(" tok "), "Bearer  tok ");
    }

    // -- parse_health (2xx)

    #[test]
    fn parse_health_ok_decodes_server_shape() {
        let body = r#"{"status":"ok","server":"cade-server","version":"0.2.0"}"#;
        let h = parse_health(200, body).expect("decode");
        assert_eq!(h.status, "ok");
        assert_eq!(h.server.as_deref(), Some("cade-server"));
    }

    #[test]
    fn parse_health_accepts_any_2xx() {
        // 204 wouldn't have a body, but 200/201/202 should all decode.
        let body = r#"{"status":"ok"}"#;
        assert!(parse_health(200, body).is_ok());
        assert!(parse_health(202, body).is_ok());
    }

    // -- parse_health (errors)

    #[test]
    fn parse_health_401_returns_unauthorized() {
        let err = parse_health(401, "Unauthorized: missing or invalid API key")
            .expect_err("must error");
        assert_eq!(err, ApiError::Unauthorized);
    }

    #[test]
    fn parse_health_500_returns_server() {
        let err = parse_health(500, r#"{"error":"internal error"}"#).expect_err("must error");
        assert_eq!(err, ApiError::Server { status: 500 });
    }

    #[test]
    fn parse_health_malformed_json_returns_decode() {
        let err = parse_health(200, "not json").expect_err("must error");
        match err {
            ApiError::Decode { .. } => {}
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    // -- parse_agents

    #[test]
    fn parse_agents_ok_decodes_list() {
        let body = r#"[{"id":"a1","name":"A1"},{"id":"a2","name":"A2","model":"gpt-4o"}]"#;
        let agents = parse_agents(200, body).expect("decode");
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].id, "a1");
        assert_eq!(agents[1].model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn parse_agents_empty_list_ok() {
        let agents = parse_agents(200, "[]").expect("decode");
        assert!(agents.is_empty());
    }

    #[test]
    fn parse_agents_401_returns_unauthorized() {
        let err = parse_agents(401, "nope").expect_err("must error");
        assert_eq!(err, ApiError::Unauthorized);
    }

    #[test]
    fn parse_agents_503_returns_server() {
        let err = parse_agents(503, "down").expect_err("must error");
        assert_eq!(err, ApiError::Server { status: 503 });
    }

    // -- Display

    #[test]
    fn api_error_display_is_user_safe() {
        // Never leak stack traces or internal paths — the tdd-guide §3.3
        // rule applies here even though we're on the client side.
        assert_eq!(ApiError::Unauthorized.to_string(), "unauthorized");
        assert_eq!(
            ApiError::Server { status: 500 }.to_string(),
            "server error (status 500)"
        );
    }
}
