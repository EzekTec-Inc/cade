//! Live, revocable remembered permissions shared by a Working Session's Runs.

use parking_lot::Mutex;
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct SessionGrants {
    state: Arc<Mutex<GrantState>>,
}

#[derive(Debug)]
struct GrantState {
    deadline: Instant,
    closed: bool,
    tools: HashSet<String>,
}

#[derive(Debug)]
pub enum SessionGrantError {
    Ended,
    Resolution(String),
}

impl std::fmt::Display for SessionGrantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ended => f.write_str("Working session has ended"),
            Self::Resolution(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for SessionGrantError {}

impl SessionGrants {
    pub fn new(lifetime: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(GrantState {
                deadline: Instant::now() + lifetime,
                closed: false,
                tools: HashSet::new(),
            })),
        }
    }

    pub fn is_active(&self) -> bool {
        let state = self.state.lock();
        !state.closed && Instant::now() < state.deadline
    }

    /// Renewal never resurrects a closed or expired Working Session.
    pub fn renew(&self, lifetime: Duration) -> bool {
        let mut state = self.state.lock();
        if state.closed || Instant::now() >= state.deadline {
            return false;
        }
        state.deadline = Instant::now() + lifetime;
        true
    }

    pub fn allows(&self, tool: &str) -> bool {
        let state = self.state.lock();
        !state.closed
            && Instant::now() < state.deadline
            && state.tools.contains(super::canonical_tool_name(tool))
    }

    /// Publish a grant only if its pending decision is successfully resolved.
    /// Closing and publishing serialize on the same lock; a rejected/duplicate
    /// decision cannot accidentally authorize another tool.
    pub fn grant_if(
        &self,
        tool: &str,
        resolve: impl FnOnce() -> Result<bool, String>,
    ) -> Result<bool, SessionGrantError> {
        let mut state = self.state.lock();
        if state.closed || Instant::now() >= state.deadline {
            return Err(SessionGrantError::Ended);
        }
        if !resolve().map_err(SessionGrantError::Resolution)? {
            return Ok(false);
        }
        state
            .tools
            .insert(super::canonical_tool_name(tool).to_owned());
        Ok(true)
    }

    /// Resolve a waiting invocation under an existing grant without turning it
    /// into an irrevocable once-approval or re-creating a revoked grant.
    pub fn resolve_if_allowed(
        &self,
        tool: &str,
        resolve: impl FnOnce() -> Result<bool, String>,
    ) -> Result<bool, SessionGrantError> {
        let state = self.state.lock();
        if state.closed
            || Instant::now() >= state.deadline
            || !state.tools.contains(super::canonical_tool_name(tool))
        {
            return Ok(false);
        }
        resolve().map_err(SessionGrantError::Resolution)
    }

    pub fn revoke_prefix(&self, prefix: &str) {
        self.state
            .lock()
            .tools
            .retain(|tool| !tool.starts_with(prefix));
    }

    pub fn close(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        state.tools.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{PermissionManager, PermissionMode, PermissionRule};

    #[test]
    fn native_aliases_share_identity_but_qualified_tool_case_is_preserved() {
        let grants = SessionGrants::new(Duration::from_secs(300));
        grants.grant_if("RunShellCommand", || Ok(true)).unwrap();
        assert!(grants.allows("bash"));
        let manager = PermissionManager::new_with_strict_bash(PermissionMode::Default, true)
            .with_session_grants(grants.clone());
        assert!(
            manager
                .resolve(
                    "RunShellCommand",
                    &serde_json::json!({"command":"cargo test"}),
                    false
                )
                .is_allow()
        );
        grants.grant_if("ServerA__write_file", || Ok(true)).unwrap();
        assert!(!grants.allows("servera__write_file"));
        assert!(!grants.allows("ServerA__Write_File"));
        grants
            .grant_if("ServerA__RunShellCommand", || Ok(true))
            .unwrap();
        assert!(!grants.allows("ServerA__bash"));
        grants.revoke_prefix("ServerA__");
        assert!(!grants.allows("ServerA__write_file"));
    }

    #[test]
    fn inherited_resolution_serializes_with_close_and_cannot_recreate_grants() {
        let grants = SessionGrants::new(Duration::from_secs(300));
        grants.grant_if("write_file", || Ok(true)).unwrap();
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let resolving = grants.clone();
        let wait = entered.clone();
        let finish = release.clone();
        let resolver = std::thread::spawn(move || {
            resolving.resolve_if_allowed("write_file", || {
                wait.wait();
                finish.wait();
                Ok(true)
            })
        });
        entered.wait();
        let closing = grants.clone();
        let closed = std::thread::spawn(move || closing.close());
        release.wait();
        assert!(resolver.join().unwrap().unwrap());
        closed.join().unwrap();
        assert!(!grants.allows("write_file"));
        assert!(
            !grants
                .resolve_if_allowed("write_file", || panic!(
                    "closed grant cannot resolve another request"
                ))
                .unwrap()
        );
        let grants = SessionGrants::new(Duration::from_secs(300));
        grants.grant_if("mcp__write_file", || Ok(true)).unwrap();
        grants.revoke_prefix("mcp__");
        assert!(
            !grants
                .resolve_if_allowed("mcp__write_file", || panic!(
                    "revoked grant cannot be resurrected"
                ))
                .unwrap()
        );
    }

    #[test]
    fn late_tool_grants_are_live_across_managers_and_strict_bash() {
        let grants = SessionGrants::new(Duration::from_secs(300));
        let parent = PermissionManager::new_with_strict_bash(PermissionMode::Default, true)
            .with_session_grants(grants.clone());
        let child = PermissionManager::new_with_strict_bash(PermissionMode::Default, true)
            .with_session_grants(grants.clone());
        let before = serde_json::json!({"command":"cargo test"});
        let after = serde_json::json!({"command":"cargo build"});
        assert!(child.resolve("bash", &before, false).is_ask());
        grants.grant_if("bash", || Ok(true)).unwrap();
        assert!(parent.resolve("bash", &before, false).is_allow());
        assert!(child.resolve("bash", &after, false).is_allow());
        child.add_deny_rule(PermissionRule::parse("bash").unwrap());
        assert!(child.resolve("bash", &after, false).is_deny());
        parent.set_mode(PermissionMode::Plan);
        assert!(
            parent
                .resolve("bash", &serde_json::json!({"command":"rm file"}), false)
                .is_deny()
        );
    }

    #[test]
    fn grants_are_tool_wide_qualified_and_revoked_for_existing_descendants() {
        let grants = SessionGrants::new(Duration::from_secs(300));
        let manager =
            PermissionManager::new(PermissionMode::Default).with_session_grants(grants.clone());
        grants
            .grant_if("server_a__write_file", || Ok(true))
            .unwrap();
        assert!(
            manager
                .resolve(
                    "server_a__write_file",
                    &serde_json::json!({"path":"src/other.rs", "content":"changed"}),
                    true
                )
                .is_allow()
        );
        assert!(
            manager
                .resolve(
                    "server_b__write_file",
                    &serde_json::json!({"path":"src/other.rs"}),
                    true
                )
                .is_ask()
        );
        assert!(
            manager
                .resolve(
                    "server_a__write_file",
                    &serde_json::json!({"path":".env"}),
                    true
                )
                .is_deny()
        );
        grants.close();
        assert!(
            manager
                .resolve(
                    "server_a__write_file",
                    &serde_json::json!({"path":"src/other.rs"}),
                    true
                )
                .is_ask()
        );
        assert!(!grants.renew(Duration::from_secs(300)));
        assert!(
            grants
                .grant_if("bash", || panic!(
                    "closed session must not resolve decisions"
                ))
                .is_err()
        );
    }

    #[test]
    fn expired_failed_and_duplicate_decisions_never_publish_grants() {
        let expired = SessionGrants::new(Duration::ZERO);
        assert!(!expired.is_active());
        assert!(!expired.renew(Duration::from_secs(300)));
        assert!(expired.grant_if("bash", || Ok(true)).is_err());
        let grants = SessionGrants::new(Duration::from_secs(300));
        assert!(!grants.grant_if("bash", || Ok(false)).unwrap());
        assert!(
            grants
                .grant_if("bash", || Err("storage unavailable".into()))
                .is_err()
        );
        assert!(!grants.allows("bash"));
        grants.grant_if("server__write", || Ok(true)).unwrap();
        grants.revoke_prefix("server__");
        assert!(!grants.allows("server__write"));
    }
}
