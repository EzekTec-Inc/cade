//! Wasm-only HTTP client: thin `gloo-net` adapter that delegates all
//! logic to the pure `api` module.
//!
//! This file compiles only for `wasm32`.  It contains **no behaviour**
//! beyond:
//!   1. Issuing a `GET` via `gloo-net::http::Request`.
//!   2. Reading the response status + body text.
//!   3. Calling the corresponding `api::parse_*` function.
//!
//! Native `cargo test` therefore has nothing to execute here — but every
//! branch it would execute is covered by `api::tests::*`.  If you are
//! tempted to add logic here (e.g. retry, caching, header shaping),
//! push it down into `api` so it stays testable on native.

#![cfg(target_arch = "wasm32")]

use cade_api_types::{AgentInfo, HealthInfo};
use gloo_net::http::Request;

use crate::api::{self, ApiError};

/// `GET /v1/health` — returns the parsed health envelope or a typed error.
///
/// `base_url` must be the scheme + host (+ optional port) of the
/// cade-server instance; trailing slashes are tolerated.
pub async fn get_health(base_url: &str, token: &str) -> Result<HealthInfo, ApiError> {
    let url = api::build_url(base_url, "/v1/health");
    let (status, body) = send(&url, token).await?;
    api::parse_health(status, &body)
}

/// `GET /v1/agents` — returns the parsed agent list or a typed error.
pub async fn get_agents(base_url: &str, token: &str) -> Result<Vec<AgentInfo>, ApiError> {
    let url = api::build_url(base_url, "/v1/agents");
    let (status, body) = send(&url, token).await?;
    api::parse_agents(status, &body)
}

/// Internal: one GET round-trip, returning `(status, body)` or a
/// `Transport` error on any `gloo-net` failure.
async fn send(url: &str, token: &str) -> Result<(u16, String), ApiError> {
    let req = Request::get(url)
        .header("Authorization", &api::bearer_header(token))
        .build()
        .map_err(|e| ApiError::Transport {
            message: e.to_string(),
        })?;

    let resp = req.send().await.map_err(|e| ApiError::Transport {
        message: e.to_string(),
    })?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| ApiError::Transport {
        message: e.to_string(),
    })?;
    Ok((status, body))
}
