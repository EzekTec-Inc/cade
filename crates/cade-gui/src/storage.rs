//! Thin localStorage wrapper for session persistence.
//!
//! **Design:** The module exposes pure key definitions and a trait-like
//! API.  On wasm32 it delegates to `web_sys::Storage`; on native builds
//! the public functions are available but return `None` / no-op (useful
//! for tests that exercise call-sites without a browser).

/// Well-known keys stored in `localStorage`.
///
/// Using an enum prevents typo-based key collisions and makes it easy
/// to enumerate all persisted data for a "clear all" operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageKey {
    /// The bearer token (API key) entered on the login screen.
    ApiToken,
    /// The server URL the token was validated against.
    ServerUrl,
    /// Saved connection profiles JSON string.
    Profiles,
    /// Saved user profile name.
    ProfileName,
    /// Saved user profile email.
    ProfileEmail,
}

impl StorageKey {
    /// The raw string key written into `localStorage`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiToken => "cade_api_token",
            Self::ServerUrl => "cade_server_url",
            Self::Profiles => "cade_profiles",
            Self::ProfileName => "cade_profile_name",
            Self::ProfileEmail => "cade_profile_email",
        }
    }

    /// All defined keys, for bulk operations (e.g. logout / clear).
    pub fn all() -> &'static [StorageKey] {
        &[
            Self::ApiToken,
            Self::ServerUrl,
            Self::Profiles,
            Self::ProfileName,
            Self::ProfileEmail,
        ]
    }
}

// ── wasm32 implementation ──────────────────────────────────────────

/// Save a value under the given key in `localStorage`.
///
/// Returns `true` on success, `false` if storage is unavailable or the
/// write failed (e.g. quota exceeded, private browsing).
#[cfg(target_arch = "wasm32")]
pub fn save(key: StorageKey, value: &str) -> bool {
    local_storage()
        .and_then(|s| s.set_item(key.as_str(), value).ok())
        .is_some()
}

/// Load a value from `localStorage`.
///
/// Returns `None` if the key doesn't exist or storage is unavailable.
#[cfg(target_arch = "wasm32")]
pub fn load(key: StorageKey) -> Option<String> {
    local_storage().and_then(|s| s.get_item(key.as_str()).ok().flatten())
}

/// Remove a single key from `localStorage`.
#[cfg(target_arch = "wasm32")]
pub fn remove(key: StorageKey) {
    if let Some(s) = local_storage() {
        let _ = s.remove_item(key.as_str());
    }
}

/// Remove all CADE keys from `localStorage`.
#[cfg(target_arch = "wasm32")]
pub fn clear_all() {
    for &key in StorageKey::all() {
        remove(key);
    }
}

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

// ── native stubs (for compile + call-site testing) ─────────────────

/// No-op on native — always returns `false`.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(_key: StorageKey, _value: &str) -> bool {
    false
}

/// No-op on native — always returns `None`.
#[cfg(not(target_arch = "wasm32"))]
pub fn load(_key: StorageKey) -> Option<String> {
    None
}

/// No-op on native.
#[cfg(not(target_arch = "wasm32"))]
pub fn remove(_key: StorageKey) {}

/// No-op on native.
#[cfg(not(target_arch = "wasm32"))]
pub fn clear_all() {}

// ── tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_key_as_str_is_prefixed() {
        assert_eq!(StorageKey::ApiToken.as_str(), "cade_api_token");
        assert_eq!(StorageKey::ServerUrl.as_str(), "cade_server_url");
    }

    #[test]
    fn storage_key_all_contains_both() {
        let all = StorageKey::all();
        assert!(all.contains(&StorageKey::ApiToken));
        assert!(all.contains(&StorageKey::ServerUrl));
        assert!(all.contains(&StorageKey::Profiles));
        assert!(all.contains(&StorageKey::ProfileName));
        assert!(all.contains(&StorageKey::ProfileEmail));
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn storage_keys_are_unique() {
        let all = StorageKey::all();
        let mut strs: Vec<&str> = all.iter().map(|k| k.as_str()).collect();
        strs.sort();
        strs.dedup();
        assert_eq!(strs.len(), all.len(), "duplicate key strings detected");
    }

    #[test]
    fn native_stubs_are_safe_no_ops() {
        // On native, save returns false, load returns None.
        assert!(!save(StorageKey::ApiToken, "test-token"));
        assert_eq!(load(StorageKey::ApiToken), None);
        // remove / clear_all don't panic.
        remove(StorageKey::ApiToken);
        clear_all();
    }

    #[test]
    fn auto_login_flow_with_saved_token() {
        // Simulates the logic in CadeApp::new: if a saved token is found,
        // LoginState should transition to Submitted.
        use crate::login::LoginState;

        // Simulate: token found in storage.
        let saved_token = "saved-secret-key";
        let mut login = LoginState::new();
        login.on_input(saved_token);
        login.on_submit();
        match &login {
            LoginState::Submitted { key } => assert_eq!(key, saved_token),
            other => panic!("expected Submitted, got {other:?}"),
        }
    }

    #[test]
    fn auto_login_skipped_when_no_token() {
        // Simulates: no token in storage → stay in Entering.
        use crate::login::LoginState;

        let saved_token: Option<String> = load(StorageKey::ApiToken); // None on native
        let mut login = LoginState::new();
        if let Some(tok) = saved_token {
            if !tok.is_empty() {
                login.on_input(&tok);
                login.on_submit();
            }
        }
        assert!(matches!(login, LoginState::Entering { .. }));
    }

    #[test]
    fn clear_all_removes_all_keys() {
        // On native these are no-ops, but we verify no panics and
        // that load still returns None after clear_all.
        save(StorageKey::ApiToken, "tok");
        save(StorageKey::ServerUrl, "http://x");
        clear_all();
        assert_eq!(load(StorageKey::ApiToken), None);
        assert_eq!(load(StorageKey::ServerUrl), None);
    }
}
