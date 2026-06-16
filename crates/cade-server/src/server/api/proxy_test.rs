//! Tests for P1-3: SSRF proxy lockdown.
//!
//! The `/v1/stream` handler proxies outbound HTTP on behalf of the agent.
//! Without validation, an authenticated caller (or a malicious prompt fed
//! to the agent) can reach cloud metadata (169.254.169.254), loopback
//! services, file:// URLs, etc.  The validator enforces:
//!
//!   1. scheme ∈ {http, https}
//!   2. host is not a numeric IP literal
//!   3. host is on the allow-list
//!   4. no resolved IP is private/loopback/link-local
//!
//! These tests cover the validator in isolation (fast, deterministic,
//! no network).  Happy-path integration through the handler is covered
//! separately once a mocked upstream is wired up.

use crate::server::api::proxy::{UrlRejection, validate_outbound_url};

// ── scheme rejection ───────────────────────────────────────────────────

#[test]
fn rejects_file_scheme() {
    let err = validate_outbound_url("file:///etc/passwd").unwrap_err();
    assert_eq!(err, UrlRejection::BadScheme);
}

#[test]
fn rejects_ftp_scheme() {
    let err = validate_outbound_url("ftp://example.com/pub").unwrap_err();
    assert_eq!(err, UrlRejection::BadScheme);
}

#[test]
fn rejects_data_scheme() {
    let err = validate_outbound_url("data:text/plain,hi").unwrap_err();
    assert_eq!(err, UrlRejection::BadScheme);
}

// ── IP-literal rejection ───────────────────────────────────────────────

#[test]
fn rejects_ipv4_literal_loopback() {
    let err = validate_outbound_url("http://127.0.0.1/").unwrap_err();
    assert_eq!(err, UrlRejection::IpLiteralHost);
}

#[test]
fn rejects_ipv4_literal_private() {
    let err = validate_outbound_url("http://10.0.0.1/").unwrap_err();
    assert_eq!(err, UrlRejection::IpLiteralHost);
}

#[test]
fn rejects_ipv4_literal_metadata() {
    // AWS/GCP/Azure metadata endpoint.
    let err = validate_outbound_url("http://169.254.169.254/").unwrap_err();
    assert_eq!(err, UrlRejection::IpLiteralHost);
}

#[test]
fn rejects_ipv6_literal_loopback() {
    let err = validate_outbound_url("http://[::1]/").unwrap_err();
    assert_eq!(err, UrlRejection::IpLiteralHost);
}

#[test]
fn rejects_ipv6_literal_public() {
    // Even a public IPv6 literal bypasses the host allow-list.
    let err = validate_outbound_url("http://[2606:4700:4700::1111]/").unwrap_err();
    assert_eq!(err, UrlRejection::IpLiteralHost);
}

// ── host allow-list ────────────────────────────────────────────────────

#[test]
fn rejects_unknown_host() {
    let err = validate_outbound_url("https://evil.example.com/path").unwrap_err();
    assert_eq!(err, UrlRejection::HostNotAllowed);
}

#[test]
fn rejects_similar_but_not_matching_host() {
    // `api.anthropic.com.evil.com` must not be allowed just because it
    // contains `api.anthropic.com` as a substring.
    let err = validate_outbound_url("https://api.anthropic.com.evil.com/v1/messages").unwrap_err();
    assert_eq!(err, UrlRejection::HostNotAllowed);
}

#[test]
fn allows_exact_anthropic_host() {
    let allowed = validate_outbound_url("https://api.anthropic.com/v1/messages").unwrap();
    assert_eq!(allowed.host_str(), Some("api.anthropic.com"));
}

#[test]
fn allows_exact_openai_host() {
    let allowed = validate_outbound_url("https://api.openai.com/v1/chat/completions").unwrap();
    assert_eq!(allowed.host_str(), Some("api.openai.com"));
}

#[test]
fn allows_google_generativelanguage_host() {
    let allowed =
        validate_outbound_url("https://generativelanguage.googleapis.com/v1/models").unwrap();
    assert_eq!(
        allowed.host_str(),
        Some("generativelanguage.googleapis.com")
    );
}

#[test]
fn allows_openrouter_host() {
    let allowed = validate_outbound_url("https://openrouter.ai/api/v1/chat").unwrap();
    assert_eq!(allowed.host_str(), Some("openrouter.ai"));
}

#[test]
fn allows_googleapis_subdomain() {
    // *.googleapis.com is on the suffix allow-list.
    let allowed = validate_outbound_url("https://oauth2.googleapis.com/token").unwrap();
    assert_eq!(allowed.host_str(), Some("oauth2.googleapis.com"));
}

#[test]
fn allows_anthropic_subdomain() {
    let allowed = validate_outbound_url("https://console.anthropic.com/v1/health").unwrap();
    assert_eq!(allowed.host_str(), Some("console.anthropic.com"));
}

// ── scheme default / edge cases ────────────────────────────────────────

#[test]
fn rejects_bogus_missing_host_url() {
    // `http:///path-only` parses as domain="path-only", which correctly
    // falls through to HostNotAllowed.  The `MissingHost` variant is
    // reserved for URL kinds where `host_str()` is None (e.g. opaque
    // non-special schemes that pass the scheme check by accident in
    // future extensions).  Here we just confirm it's blocked at all.
    let err = validate_outbound_url("http:///path-only").unwrap_err();
    assert_eq!(err, UrlRejection::HostNotAllowed);
}

#[test]
fn rejects_malformed_url() {
    let err = validate_outbound_url("not a url at all").unwrap_err();
    assert_eq!(err, UrlRejection::Malformed);
}

#[test]
fn rejects_empty_url() {
    let err = validate_outbound_url("").unwrap_err();
    assert_eq!(err, UrlRejection::Malformed);
}
