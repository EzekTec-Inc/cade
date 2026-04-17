//! Boot-time configuration for the cade-gui WASM app.
//!
//! **Pure logic, no browser dependencies.**  Keeps the predicate testable on
//! native so it is covered by the project's standard `cargo test --workspace`
//! without requiring `wasm-bindgen-test` or a headless browser.
//!
//! Precedence for the API key, highest first:
//!   1. `?key=...` query-string parameter (explicit one-tab override)
//!   2. user-typed value from the login form (supplied by caller)
//!   3. empty (browser will render the login form)
//!
//! The `server_url` defaults to the origin the page was served from — the
//! dashboard and API are served by the same `cade-server` process, so the
//! client never talks to a different host unless explicitly told to.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Base URL of cade-server (usually the page origin).
    pub server_url: String,
    /// Bearer token. Empty string means "not yet entered".
    pub api_key: String,
}

impl Config {
    /// Resolve the runtime config from three inputs.  Keeping this pure makes
    /// the precedence rules testable on native.
    pub fn resolve(origin: &str, query: Option<&str>, user_typed: Option<&str>) -> Self {
        let api_key = query
            .and_then(parse_key_from_query)
            .or_else(|| user_typed.map(str::to_string).filter(|s| !s.is_empty()))
            .unwrap_or_default();

        Self {
            server_url: origin.to_string(),
            api_key,
        }
    }
}

fn parse_key_from_query(q: &str) -> Option<String> {
    let q = q.strip_prefix('?').unwrap_or(q);
    for pair in q.split('&') {
        if let Some(rest) = pair.strip_prefix("key=") {
            let decoded = percent_decode_plus(rest);
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    None
}

fn percent_decode_plus(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push((h << 4) | l);
                    i += 3;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Precedence

    #[test]
    fn resolve_uses_query_key_when_present() {
        let c = Config::resolve("http://localhost:8284", Some("?key=abc"), Some("ignored"));
        assert_eq!(c.server_url, "http://localhost:8284");
        assert_eq!(c.api_key, "abc");
    }

    #[test]
    fn resolve_uses_user_typed_when_no_query_key() {
        let c = Config::resolve("http://localhost:8284", None, Some("typed"));
        assert_eq!(c.api_key, "typed");
    }

    #[test]
    fn resolve_empty_key_when_nothing_provided() {
        let c = Config::resolve("http://localhost:8284", None, None);
        assert_eq!(c.api_key, "");
    }

    #[test]
    fn resolve_empty_query_value_falls_through_to_user_typed() {
        let c = Config::resolve("http://x", Some("?key="), Some("fallback"));
        assert_eq!(c.api_key, "fallback");
    }

    // -- Query parsing robustness

    #[test]
    fn resolve_percent_decodes_query_key() {
        let c = Config::resolve("http://x", Some("?key=a%20b%2Bc"), None);
        assert_eq!(c.api_key, "a b+c");
    }

    #[test]
    fn resolve_picks_key_even_when_other_params_come_first() {
        let c = Config::resolve("http://x", Some("?foo=1&key=tok&bar=2"), None);
        assert_eq!(c.api_key, "tok");
    }

    #[test]
    fn resolve_ignores_malformed_percent_sequences() {
        let c = Config::resolve("http://x", Some("?key=a%ZZb"), None);
        assert_eq!(c.api_key, "a%ZZb");
    }

    // -- Security: first occurrence wins

    #[test]
    fn resolve_takes_first_key_occurrence_only() {
        let c = Config::resolve("http://x", Some("?key=good&key=evil"), None);
        assert_eq!(c.api_key, "good");
    }
}
