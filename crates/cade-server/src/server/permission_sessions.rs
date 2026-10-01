//! Working Session lifetime and approval publication, shared by both execution adapters.

use cade_core::permissions::{PermissionManager, SessionGrantError, SessionGrants};
use cade_store::sqlite::{self, Db};
use parking_lot::Mutex;
use serde_json::json;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

/// A disconnected/crashed client cannot leave remembered permissions alive forever.
/// The live client renews every thirty seconds, independently of its current Run.
pub const WORKING_SESSION_LEASE: Duration = Duration::from_secs(300);

struct WorkingSession {
    workspace: PathBuf,
    grants: SessionGrants,
}

#[derive(Clone)]
struct PendingPermission {
    tool: String,
    grants: Option<SessionGrants>,
}

#[derive(Default)]
pub struct PermissionSessions {
    sessions: Mutex<HashMap<String, WorkingSession>>,
    pending: Mutex<HashMap<String, PendingPermission>>,
}

#[derive(Debug)]
pub enum DecisionError {
    InvalidScope(String),
    Storage(String),
}

impl std::fmt::Display for DecisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScope(message) | Self::Storage(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for DecisionError {}

impl PermissionSessions {
    pub fn open(&self, workspace: &Path) -> Result<String, String> {
        let workspace = workspace
            .canonicalize()
            .map_err(|e| format!("Invalid workspace: {e}"))?;
        if !workspace.is_dir() {
            return Err("Workspace must be a directory".into());
        }
        let mut sessions = self.sessions.lock();
        sessions.retain(|_, session| session.grants.is_active());
        let id = format!("ws-{}", uuid::Uuid::new_v4());
        sessions.insert(
            id.clone(),
            WorkingSession {
                workspace,
                grants: SessionGrants::new(WORKING_SESSION_LEASE),
            },
        );
        Ok(id)
    }

    pub fn for_run(&self, id: &str, workspace: &Path) -> Result<SessionGrants, String> {
        let sessions = self.sessions.lock();
        let session = sessions
            .get(id)
            .ok_or("Working session not found or ended")?;
        if !session.grants.is_active() {
            return Err("Working session has expired".into());
        }
        if session.workspace != workspace {
            return Err("Working session belongs to a different workspace".into());
        }
        Ok(session.grants.clone())
    }

    pub fn renew(&self, id: &str) -> Result<(), String> {
        let sessions = self.sessions.lock();
        let session = sessions
            .get(id)
            .ok_or("Working session not found or ended")?;
        if !session.grants.renew(WORKING_SESSION_LEASE) {
            return Err("Working session has expired".into());
        }
        Ok(())
    }

    pub fn close(&self, id: &str) {
        if let Some(session) = self.sessions.lock().remove(id) {
            session.grants.close();
        }
    }

    /// Register before making an approval visible; the registration is owned by
    /// the waiting execution and is removed on completion or cancellation.
    pub fn register(
        self: &Arc<Self>,
        id: &str,
        tool: &str,
        permissions: &PermissionManager,
    ) -> PendingPermissionGuard {
        self.pending.lock().insert(
            id.to_owned(),
            PendingPermission {
                tool: tool.to_owned(),
                grants: permissions.session_grants(),
            },
        );
        PendingPermissionGuard {
            owner: self.clone(),
            id: id.to_owned(),
        }
    }

    /// Resolve the durable request and publish the remembered grant before
    /// acknowledgement. A cancelled or already-resolved request grants nothing.
    pub fn resolve(
        &self,
        db: &Db,
        id: &str,
        status: &str,
        feedback: Option<&str>,
    ) -> Result<bool, DecisionError> {
        let resolve =
            || sqlite::resolve_pending_approval(db, id, status).map_err(|e| e.to_string());
        let changed = if status == "approved_session" {
            let pending = self.pending.lock().get(id).cloned().ok_or_else(|| {
                DecisionError::InvalidScope(
                    "Session approval requires a live tool permission request".into(),
                )
            })?;
            let grants = pending.grants.ok_or_else(|| {
                DecisionError::InvalidScope(
                    "Session approval requires an active Working Session".into(),
                )
            })?;
            grants
                .grant_if(&pending.tool, resolve)
                .map_err(|error| match error {
                    SessionGrantError::Ended => DecisionError::InvalidScope(error.to_string()),
                    SessionGrantError::Resolution(message) => DecisionError::Storage(message),
                })?
        } else {
            resolve().map_err(DecisionError::Storage)?
        };
        if changed {
            crate::server::api::agents::publish_global_event(
                Some(db),
                "approval_resolved",
                json!({
                    "id": id, "status": status, "feedback": feedback,
                }),
            );
        }
        Ok(changed)
    }

    /// A grant from another Run/child also releases already-waiting requests.
    pub fn resolve_remembered(&self, db: &Db, id: &str) -> Result<(), String> {
        let pending = self.pending.lock().get(id).cloned();
        if let Some(PendingPermission {
            tool,
            grants: Some(grants),
        }) = pending
        {
            let changed = grants
                .resolve_if_allowed(&tool, || {
                    sqlite::resolve_pending_approval(db, id, "approved_session")
                        .map_err(|error| error.to_string())
                })
                .map_err(|error| error.to_string())?;
            if changed {
                crate::server::api::agents::publish_global_event(
                    Some(db),
                    "approval_resolved",
                    json!({"id": id, "status": "approved_session", "feedback": null}),
                );
            }
        }
        Ok(())
    }
}

pub struct PendingPermissionGuard {
    owner: Arc<PermissionSessions>,
    id: String,
}

impl Drop for PendingPermissionGuard {
    fn drop(&mut self) {
        self.owner.pending.lock().remove(&self.id);
    }
}

/// Shared interpretation: question feedback remains handled by its question
/// owner, while both tool adapters preserve the same once/session/deny meaning.
pub fn approval_outcome(
    status: &str,
    tool: &str,
    permissions: &PermissionManager,
) -> Result<bool, String> {
    match status {
        "approved_session"
            if permissions
                .session_grants()
                .is_some_and(|grants| grants.allows(tool)) =>
        {
            Ok(true)
        }
        "approved_session" => Err("Working session grant is no longer active".into()),
        "approved" => Ok(true),
        status if status.starts_with("approved:") => Ok(true),
        "denied" => Ok(false),
        status if status.starts_with("denied:") => {
            Err(format!("Permission Denied: {}", &status[7..]))
        }
        _ => Err("Approval request is missing or has an invalid status".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cade_core::permissions::PermissionMode;

    #[test]
    fn session_identity_survives_runs_not_other_workspaces_or_new_clients() {
        let registry = PermissionSessions::default();
        let workspace = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let id = registry.open(&root).unwrap();
        let first = registry.for_run(&id, &root).unwrap();
        let later = registry.for_run(&id, &root).unwrap();
        first.grant_if("write_file", || Ok(true)).unwrap();
        assert!(later.allows("write_file"));
        registry.renew(&id).unwrap();
        assert!(registry.for_run(&id, other.path()).is_err());
        let fresh = registry.open(&root).unwrap();
        assert!(
            !registry
                .for_run(&fresh, &root)
                .unwrap()
                .allows("write_file")
        );
        registry.close(&id);
        assert!(!first.allows("write_file") && !later.allows("write_file"));
        assert!(registry.for_run(&id, &root).is_err());
        assert!(registry.renew(&id).is_err());
    }

    #[test]
    fn both_adapters_share_outcome_meaning_and_require_published_session_grants() {
        let grants = SessionGrants::new(WORKING_SESSION_LEASE);
        let permissions =
            PermissionManager::new(PermissionMode::Default).with_session_grants(grants.clone());
        assert!(approval_outcome("approved_session", "write_file", &permissions).is_err());
        grants.grant_if("write_file", || Ok(true)).unwrap();
        assert_eq!(
            approval_outcome("approved_session", "write_file", &permissions),
            Ok(true)
        );
        assert_eq!(
            approval_outcome("approved", "write_file", &permissions),
            Ok(true)
        );
        assert_eq!(
            approval_outcome("denied", "write_file", &permissions),
            Ok(false)
        );
        assert_eq!(
            approval_outcome("denied:explain first", "write_file", &permissions),
            Err("Permission Denied: explain first".into())
        );
        grants.close();
        assert!(approval_outcome("approved_session", "write_file", &permissions).is_err());
    }
}
