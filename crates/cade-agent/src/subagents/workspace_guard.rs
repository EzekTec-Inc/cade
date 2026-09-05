//! RAII-managed isolated workspace guard for subagent sessions (ADR-0021 / Issue #50).
//!
//! Encapsulates temporary sandbox directory creation, atomic merge back on task
//! completion, and automatic leak-free cleanup on drop or cancellation.

use std::io;
use std::path::Path;

use crate::tools::isolation::IsolatedWorkspace;

/// RAII Guard managing an isolated workspace lifecycle.
pub struct IsolatedWorkspaceGuard {
    workspace: Option<IsolatedWorkspace>,
    committed: bool,
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

    /// Atomically merge modified files back into the primary workspace and mark as committed.
    pub async fn commit_and_merge(&mut self) -> io::Result<()> {
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
        self.workspace = None;
        self.committed = false;
    }
}

impl Drop for IsolatedWorkspaceGuard {
    fn drop(&mut self) {
        if !self.committed && self.workspace.is_some() {
            tracing::debug!(
                "Isolated workspace dropped without commit — temporary sandbox discarded cleanly"
            );
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
}
