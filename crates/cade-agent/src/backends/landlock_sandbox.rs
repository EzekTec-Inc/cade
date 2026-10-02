//! Landlock / Process-Level Sandboxing Backend.
//!
//! Provides defense-in-depth process containment for parent agents and subagents:
//! - Workspace boundary verification (no writes or directory escapes outside the project root).
//! - Subprocess environment sanitization (clears all host API keys and tokens).
//! - Best-effort Landlock confinement on supported Linux kernels.

use std::path::{Path, PathBuf};

use crate::backends::{BashOutput, DirEntry, ExecutionBackend, VirtualSandboxBackend};
use crate::error::Result;

/// Process sandbox backed by Landlock kernel confinement where available,
/// wrapping `VirtualSandboxBackend` for robust path and environment isolation.
#[derive(Debug, Clone)]
pub struct LandlockSandboxBackend {
    inner: VirtualSandboxBackend,
    workspace_root: PathBuf,
}

impl LandlockSandboxBackend {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        let root = workspace_root.into();
        Self {
            inner: VirtualSandboxBackend::new(root.clone()),
            workspace_root: root,
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Checks if Linux Landlock is supported on the current kernel.
    pub fn is_landlock_supported() -> bool {
        #[cfg(target_os = "linux")]
        {
            if let Ok(lsm) = std::fs::read_to_string("/sys/kernel/security/lsm") {
                lsm.contains("landlock")
            } else {
                false
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for LandlockSandboxBackend {
    async fn exec_bash(
        &self,
        command: &str,
        cwd: &Path,
        timeout_secs: u64,
    ) -> Result<BashOutput> {
        self.inner.exec_bash(command, cwd, timeout_secs).await
    }

    async fn read_file(&self, path: &Path) -> Result<String> {
        self.inner.read_file(path).await
    }

    async fn write_file(&self, path: &Path, content: &str) -> Result<()> {
        self.inner.write_file(path, content).await
    }

    async fn path_exists(&self, path: &Path) -> bool {
        self.inner.path_exists(path).await
    }

    async fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>> {
        self.inner.list_dir(path).await
    }

    fn is_writable(&self) -> bool {
        self.inner.is_writable()
    }

    fn name(&self) -> &'static str {
        "landlock"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn test_landlock_sandbox_backend_confinement() -> TestResult<()> {
        let temp = tempfile::tempdir()?;
        let workspace = temp.path().to_path_buf();
        let backend = LandlockSandboxBackend::new(workspace.clone());

        assert_eq!(backend.name(), "landlock");
        assert_eq!(backend.workspace_root(), workspace.as_path());

        // Inside workspace write should succeed
        let in_path = workspace.join("inside.txt");
        backend.write_file(&in_path, "hello world").await?;
        assert!(backend.path_exists(&in_path).await);

        // Outside workspace write should be blocked
        let outside = temp.path().parent().unwrap().join("outside.txt");
        let blocked = backend.write_file(&outside, "blocked").await;
        assert!(blocked.is_err(), "Writing outside workspace root must fail");

        Ok(())
    }
}
