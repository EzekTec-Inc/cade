//! `/v1/stream` — outbound HTTP proxy for agent-side streaming.
//!
//! ## P1-3 SSRF lockdown
//!
//! The original handler accepted any `?url=…` and forwarded it unchanged,
//! exposing loopback services, cloud metadata endpoints, and arbitrary
//! schemes.  Every outbound request must now pass [`validate_outbound_url`]:
//!
//!   1. scheme ∈ { `http`, `https` }
//!   2. host is **not** a numeric IP literal (v4 or v6)
//!   3. host is on the static allow-list
//!      (currently: LLM provider endpoints CADE streams from)
//!
//! Additionally, the `reqwest::Client` is built with redirects disabled
//! (so a 302 to a blocked host can't smuggle the request past the
//! validator) and a 30 s total timeout.

use crate::server::state::AppState;
use axum::{
    body::Body,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use reqwest::{Client, Url, redirect::Policy};
use serde::Deserialize;

/// Hosts allowed as exact matches.  Exact match means the URL's host
/// equals one of these strings byte-for-byte (case-insensitive per URL
/// spec — `url::Url::host_str` already lowercases).
const ALLOWED_HOSTS_EXACT: &[&str] = &[
    "api.anthropic.com",
    "api.openai.com",
    "generativelanguage.googleapis.com",
    "openrouter.ai",
];

/// Domain suffixes that are allowed for any subdomain.  Example:
/// `"anthropic.com"` allows `console.anthropic.com`, `api.anthropic.com`,
/// etc. but **not** `anthropic.com.evil.com` (enforced by the leading
/// dot match).
const ALLOWED_HOST_SUFFIXES: &[&str] = &[
    "anthropic.com",
    "openai.com",
    "googleapis.com",
];

/// Reason an outbound URL was rejected by [`validate_outbound_url`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlRejection {
    /// URL could not be parsed.
    Malformed,
    /// Parsed URL has no host component.
    MissingHost,
    /// Scheme is not `http` or `https`.
    BadScheme,
    /// Host is a numeric IP literal (IPv4 or IPv6).  We never allow this,
    /// because the allow-list is host-name based and an IP literal
    /// bypasses DNS.
    IpLiteralHost,
    /// Host is not on the static allow-list.
    HostNotAllowed,
}

impl UrlRejection {
    /// HTTP status to return to the caller for each rejection kind.
    pub fn status(self) -> StatusCode {
        match self {
            UrlRejection::Malformed | UrlRejection::MissingHost | UrlRejection::BadScheme => {
                StatusCode::BAD_REQUEST
            }
            UrlRejection::IpLiteralHost | UrlRejection::HostNotAllowed => StatusCode::FORBIDDEN,
        }
    }

    /// Human-readable message (safe for client response; no internals).
    pub fn message(self) -> &'static str {
        match self {
            UrlRejection::Malformed => "malformed url",
            UrlRejection::MissingHost => "url has no host",
            UrlRejection::BadScheme => "only http and https schemes are allowed",
            UrlRejection::IpLiteralHost => "ip-literal hosts are not allowed",
            UrlRejection::HostNotAllowed => "host is not on the outbound allow-list",
        }
    }
}

/// Validate an outbound URL against the SSRF policy.  Returns the parsed
/// `Url` on success so the caller doesn't have to parse twice.
pub fn validate_outbound_url(raw: &str) -> Result<Url, UrlRejection> {
    let url = Url::parse(raw).map_err(|_| UrlRejection::Malformed)?;

    match url.scheme() {
        "http" | "https" => {}
        _ => return Err(UrlRejection::BadScheme),
    }

    let host_str = url.host_str().ok_or(UrlRejection::MissingHost)?;

    // IP literals — v4 or v6 — are always rejected.  We never want the
    // agent reaching 127.0.0.1, 169.254.169.254, ::1, link-local, etc.,
    // and no legitimate provider endpoint uses a bare IP in its URL.
    //
    // `url::Url::host_str()` returns IPv6 literals WITH square brackets
    // (e.g. `"[::1]"`), so strip those before parsing.
    let ip_probe = host_str
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host_str);
    if ip_probe.parse::<std::net::IpAddr>().is_ok() {
        return Err(UrlRejection::IpLiteralHost);
    }

    if ALLOWED_HOSTS_EXACT.contains(&host_str) {
        return Ok(url);
    }

    // Suffix match: the host must end with `.suffix`.  We require the
    // leading dot so `anthropic.com.evil.com` does NOT match `anthropic.com`.
    for suffix in ALLOWED_HOST_SUFFIXES {
        let dotted = format!(".{suffix}");
        if host_str.ends_with(&dotted) || host_str == *suffix {
            return Ok(url);
        }
    }

    Err(UrlRejection::HostNotAllowed)
}

#[derive(Deserialize)]
pub struct StreamRequest {
    pub url: String,
}

pub async fn stream_http_handler(
    State(_state): State<AppState>,
    Query(params): Query<StreamRequest>,
) -> Response {
    // P1-3: validate BEFORE any network I/O.
    let url = match validate_outbound_url(&params.url) {
        Ok(u) => u,
        Err(rej) => return (rej.status(), rej.message()).into_response(),
    };

    // P1-3: disable auto-redirects so a 302 to a blocked host can't
    // bypass the allow-list.  Apply a bounded timeout so a slow upstream
    // can't tie up a connection indefinitely.
    let client = match Client::builder()
        .redirect(Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build http client: {e}"),
            )
                .into_response();
        }
    };

    let req = match client.get(url).send().await {
        Ok(res) => res,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("Failed to connect to upstream: {}", e),
            )
                .into_response();
        }
    };

    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let stream = req
        .bytes_stream()
        .map(|result| result.map_err(std::io::Error::other));

    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .unwrap_or_else(|e| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(format!("failed to build response: {e}")))
                .unwrap()
        })
}

#[cfg(test)]
#[path = "proxy_test.rs"]
mod tests;
