//! RAII-managed isolated workspace guard for subagent sessions (ADR-0021 / Issue #50).
//!
//! Encapsulates temporary sandbox directory creation, conflict-aware merge back on task
//! completion, and automatic leak-free cleanup on drop or cancellation.

use std::io;
use std::path::Path;

use crate::tools::isolation::IsolatedWorkspace;

/// RAII Guard managing an isolated workspace lifecycle.
pub struct IsolatedWorkspaceGuard {
    workspace: Option<IsolatedWorkspace>,
    committed: bool,
    cleanup_result: Option<GuardResult>,
    finalization_result: Option<GuardResult>,
}

enum GuardResult {
    Completed,
    Failed(io::ErrorKind, String),
}

impl GuardResult {
    fn from_result(result: &io::Result<()>) -> Self {
        match result {
            Ok(()) => Self::Completed,
            Err(error) => Self::Failed(error.kind(), error.to_string()),
        }
    }

    fn result(&self) -> io::Result<()> {
        match self {
            Self::Completed => Ok(()),
            Self::Failed(kind, message) => Err(io::Error::new(*kind, message.clone())),
        }
    }
}

impl IsolatedWorkspaceGuard {
    /// Create a new isolated workspace cloned from the primary path.
    pub async fn new(primary_path: &Path, git_branch_name: Option<String>) -> io::Result<Self> {
        let mut ws = IsolatedWorkspace::clone_from(primary_path)?;
        if let Some(branch) = git_branch_name {
            ws = ws.with_git_branch(&branch).await;
        }
        Ok(Self {
            workspace: Some(ws),
            committed: false,
            cleanup_result: None,
            finalization_result: None,
        })
    }

    /// Return the isolated workspace path, or None if not isolated.
    pub fn path(&self) -> Option<&Path> {
        self.workspace.as_ref().map(|ws| ws.path())
    }

    /// Return the primary workspace root path.
    pub fn primary_dir(&self) -> Option<&Path> {
        self.workspace.as_ref().map(|ws| ws.primary_path())
    }

    /// Reconcile child deltas against the launch baseline and current host.
    /// Both success and failure are cached by the workspace: an attempt is never
    /// retried against a different parent state.
    pub async fn commit_and_merge(&mut self) -> io::Result<()> {
        if let Some(result) = &self.finalization_result {
            return result.result();
        }
        if self.committed {
            return Ok(());
        }
        if let Some(ref ws) = self.workspace {
            ws.merge_back().await?;
            self.committed = true;
        }
        Ok(())
    }

    /// Check if changes were committed.
    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// Explicitly discard isolated changes without merging to primary workspace.
    pub fn discard(&mut self) {
        if let Err(error) = self.close() {
            tracing::warn!(%error, "isolated workspace discard cleanup failed");
        }
        self.committed = false;
    }

    /// Release the workspace explicitly so cleanup errors enter the outcome.
    /// Repeated closes are no-ops.
    ///
    /// # Errors
    /// Returns a temporary-directory removal failure.
    pub fn close(&mut self) -> io::Result<()> {
        if let Some(result) = &self.cleanup_result {
            return result.result();
        }
        let result = self
            .workspace
            .take()
            .map_or(Ok(()), IsolatedWorkspace::close);
        self.cleanup_result = Some(GuardResult::from_result(&result));
        result
    }

    /// Merge only successful work and close on every terminal path.
    ///
    /// # Errors
    /// Returns merge and cleanup failures together when both occur.
    pub async fn finalize(&mut self, success: bool) -> io::Result<()> {
        if let Some(result) = &self.finalization_result {
            return result.result();
        }
        let merge = if success {
            self.commit_and_merge().await
        } else {
            Ok(())
        };
        let cleanup = self.close();
        let result = match (merge, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(io::Error::other(format!(
                "Workspace cleanup failed: {error}"
            ))),
            (Err(merge), Err(cleanup)) => Err(io::Error::other(format!(
                "{merge}; workspace cleanup also failed: {cleanup}"
            ))),
        };
        self.finalization_result = Some(GuardResult::from_result(&result));
        result
    }
}

impl Drop for IsolatedWorkspaceGuard {
    fn drop(&mut self) {
        if self.workspace.is_some() {
            if let Err(error) = self.close() {
                tracing::warn!(%error, "isolated workspace drop cleanup failed");
            } else if !self.committed {
                tracing::debug!(
                    "Isolated workspace dropped without commit — temporary sandbox discarded"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_workspace_guard_creation_and_path() -> io::Result<()> {
        let temp_primary = tempdir()?;
        std::fs::write(temp_primary.path().join("file.txt"), "hello world")?;

        let mut guard = IsolatedWorkspaceGuard::new(temp_primary.path(), None).await?;
        assert!(guard.path().is_some());
        assert_ne!(guard.path().unwrap(), temp_primary.path());

        // Mutate in isolated workspace
        let isolated_file = guard.path().unwrap().join("file.txt");
        std::fs::write(&isolated_file, "modified content")?;

        // Primary should still have original content before commit
        let primary_file = temp_primary.path().join("file.txt");
        assert_eq!(std::fs::read_to_string(&primary_file)?, "hello world");

        // Commit and merge back
        guard.commit_and_merge().await?;
        assert!(guard.is_committed());

        // Primary should now have the modified content
        assert_eq!(std::fs::read_to_string(&primary_file)?, "modified content");
        Ok(())
    }

    #[tokio::test]
    async fn test_workspace_guard_discard() -> io::Result<()> {
        let temp_primary = tempdir()?;
        let primary_file = temp_primary.path().join("file.txt");
        std::fs::write(&primary_file, "original")?;

        let mut guard = IsolatedWorkspaceGuard::new(temp_primary.path(), None).await?;
        let isolated_file = guard.path().unwrap().join("file.txt");
        std::fs::write(&isolated_file, "discarded changes")?;

        // Discard explicitly
        guard.discard();
        assert!(!guard.is_committed());
        assert!(guard.path().is_none());

        // Primary file must remain untouched
        assert_eq!(std::fs::read_to_string(&primary_file)?, "original");

        // Merge after discard should be a no-op
        guard.commit_and_merge().await?;
        assert_eq!(std::fs::read_to_string(&primary_file)?, "original");
        Ok(())
    }

    #[tokio::test]
    async fn workspace_guard_failed_finalization_closes_and_preserves_parent() -> io::Result<()> {
        let host = tempdir()?;
        std::fs::write(host.path().join("file"), "baseline")?;
        let mut guard = IsolatedWorkspaceGuard::new(host.path(), None).await?;
        let child_path = guard.path().unwrap().to_path_buf();
        std::fs::write(child_path.join("file"), "child")?;
        std::fs::write(host.path().join("file"), "parent")?;
        let error = guard.finalize(true).await.unwrap_err();
        assert_eq!(
            guard.finalize(true).await.unwrap_err().to_string(),
            error.to_string()
        );
        assert!(!child_path.exists());
        assert!(guard.path().is_none());
        assert!(!guard.is_committed());
        assert_eq!(std::fs::read_to_string(host.path().join("file"))?, "parent");
        Ok(())
    }

    #[tokio::test]
    async fn workspace_guard_reports_cleanup_failure() -> io::Result<()> {
        let host = tempdir()?;
        let mut guard = IsolatedWorkspaceGuard::new(host.path(), None).await?;
        let child = guard.path().unwrap().to_path_buf();
        std::fs::remove_dir(&child)?;
        std::fs::write(&child, "not a directory anymore")?;
        let error = guard.finalize(false).await.unwrap_err();
        assert!(error.to_string().contains("cleanup"));
        assert_eq!(
            guard.finalize(false).await.unwrap_err().to_string(),
            error.to_string()
        );
        assert!(guard.path().is_none());
        std::fs::remove_file(child)?;
        Ok(())
    }
}
