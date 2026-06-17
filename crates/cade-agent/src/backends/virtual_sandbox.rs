//! Virtual Restricted Local Sandbox Backend.
//!
//! Provides lightweight, fast, and secure local execution with strict path
//! isolation (chroot-like boundaries) and robust environment-variable sanitization.

use crate::Result;
use crate::backends::{BashOutput, DirEntry, ExecutionBackend};
use std::path::{Path, PathBuf};

pub struct VirtualSandboxBackend {
    workspace_root: PathBuf,
    allowed_env: Vec<String>,
}

impl VirtualSandboxBackend {
    pub fn new(workspace_root: PathBuf) -> Self {
        let canonical_root = workspace_root.canonicalize().unwrap_or(workspace_root);

        Self {
            workspace_root: canonical_root,
            allowed_env: vec![
                "PATH".to_string(),
                "HOME".to_string(),
                "LANG".to_string(),
                "TZ".to_string(),
                "TERM".to_string(),
            ],
        }
    }

    fn verify_path(&self, path: &Path) -> Result<PathBuf> {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace_root.join(path)
        };

        // Normalize path components manually to resolve any relative segments (.. or .)
        // without requiring the target file/directory to actually exist on disk.
        let mut normalized = PathBuf::new();
        for component in absolute.components() {
            match component {
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::CurDir => {}
                c => normalized.push(c.as_os_str()),
            }
        }

        // Attempt to canonicalize to resolve symlinks if the path exists
        let canonical = normalized.canonicalize().unwrap_or(normalized);

        if !canonical.starts_with(&self.workspace_root) {
            return Err(crate::Error::custom(format!(
                "Security Exception: Access denied to path '{:?}' outside sandbox boundary '{:?}'",
                path, self.workspace_root
            )));
        }

        Ok(canonical)
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for VirtualSandboxBackend {
    async fn exec_bash(&self, command: &str, cwd: &Path, timeout_secs: u64) -> Result<BashOutput> {
        // 1. Verify working directory boundary
        let safe_cwd = self.verify_path(cwd)?;

        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg("-c").arg(command);
        cmd.current_dir(safe_cwd);

        // 2. Clear environment and inherit ONLY safe allowlisted variables
        cmd.env_clear();
        for key in &self.allowed_env {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }

        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| crate::Error::custom(format!("Failed to spawn sandbox shell: {e}")))?;

        let mut stdout_stream = child.stdout.take().unwrap();
        let mut stderr_stream = child.stderr.take().unwrap();

        let stdout_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            use tokio::io::AsyncReadExt;
            let _ = stdout_stream.read_to_end(&mut buf).await;
            buf
        });

        let stderr_handle = tokio::spawn(async move {
            let mut buf = Vec::new();
            use tokio::io::AsyncReadExt;
            let _ = stderr_stream.read_to_end(&mut buf).await;
            buf
        });

        // Wait with a robust timeout
        let timeout = std::time::Duration::from_secs(timeout_secs);
        let mut timed_out = false;

        let exit_status = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(status_res) => status_res
                .map_err(|e| crate::Error::custom(format!("Failed to wait for process: {e}")))?,
            Err(_) => {
                timed_out = true;
                let _ = child.kill().await;
                child
                    .wait()
                    .await
                    .map_err(|e| crate::Error::custom(format!("Failed to wait after kill: {e}")))?
            }
        };

        let stdout_bytes = stdout_handle.await.unwrap_or_default();
        let stderr_bytes = stderr_handle.await.unwrap_or_default();

        Ok(BashOutput {
            stdout: String::from_utf8_lossy(&stdout_bytes).to_string(),
            stderr: String::from_utf8_lossy(&stderr_bytes).to_string(),
            exit_code: exit_status.code().unwrap_or(-1),
            timed_out,
        })
    }

    async fn read_file(&self, path: &Path) -> Result<String> {
        let safe_path = self.verify_path(path)?;
        let content = std::fs::read_to_string(safe_path)
            .map_err(|e| crate::Error::custom(format!("Failed to read file: {e}")))?;
        Ok(content)
    }

    async fn write_file(&self, path: &Path, content: &str) -> Result<()> {
        let safe_path = self.verify_path(path)?;
        if let Some(parent) = safe_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| crate::Error::custom(format!("Failed to create parent dirs: {e}")))?;
        }
        std::fs::write(safe_path, content)
            .map_err(|e| crate::Error::custom(format!("Failed to write file: {e}")))?;
        Ok(())
    }

    async fn path_exists(&self, path: &Path) -> bool {
        if let Ok(safe_path) = self.verify_path(path) {
            safe_path.exists()
        } else {
            false
        }
    }

    async fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>> {
        let safe_path = self.verify_path(path)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(safe_path)
            .map_err(|e| crate::Error::custom(format!("Failed to read directory: {e}")))?
        {
            let entry =
                entry.map_err(|e| crate::Error::custom(format!("Failed to read entry: {e}")))?;
            let metadata = entry
                .metadata()
                .map_err(|e| crate::Error::custom(format!("Failed to read metadata: {e}")))?;

            entries.push(DirEntry {
                path: entry.path(),
                is_dir: metadata.is_dir(),
                size: Some(metadata.len()),
            });
        }
        Ok(entries)
    }

    fn name(&self) -> &'static str {
        "virtual_sandbox"
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_virtual_sandbox_path_validation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().to_path_buf();
        let backend = VirtualSandboxBackend::new(root.clone());

        // Safe write inside sandbox
        let safe_file = root.join("test.txt");
        let write_res = backend.write_file(&safe_file, "hello").await;
        assert!(write_res.is_ok());

        // Read safe file
        let read_res = backend.read_file(&safe_file).await;
        assert_eq!(read_res.unwrap(), "hello");

        // Breakout attempt outside sandbox
        let unsafe_file = Path::new("/etc/passwd");
        let break_res = backend.read_file(unsafe_file).await;
        assert!(break_res.is_err());
        assert!(
            break_res
                .unwrap_err()
                .to_string()
                .contains("Security Exception")
        );
    }

    #[tokio::test]
    async fn test_virtual_sandbox_env_sanitization() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().to_path_buf();
        let backend = VirtualSandboxBackend::new(root.clone());

        // We run a bash command printing 'env'
        let out = backend.exec_bash("env", &root, 5).await.unwrap();

        // The sandbox process's env must NOT contain any typical CADE/sensitive keys
        // (like CADE_API_KEY, ANTHROPIC_API_KEY, GITHUB_TOKEN) even if they exist in the parent shell.
        assert!(!out.combined().contains("CADE_API_KEY"));
        assert!(!out.combined().contains("ANTHROPIC_API_KEY"));
        assert!(!out.combined().contains("GITHUB_TOKEN"));

        // It must still inherit PATH and be able to find basic binaries like echo/env
        let out_path = backend
            .exec_bash("which echo || which ls", &root, 5)
            .await
            .unwrap();
        assert_eq!(out_path.exit_code, 0);
        assert!(!out_path.stdout.is_empty());
    }


    #[tokio::test]
    async fn test_virtual_sandbox_nonexistent_path_traversal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path().to_path_buf();
        let backend = VirtualSandboxBackend::new(root.clone());

        // Attempt a breakout using relative path traversal on a nonexistent target
        let breakout_path = Path::new("../nonexistent_dir/exploit.txt");
        let write_res = backend.write_file(breakout_path, "malicious").await;
        
        assert!(write_res.is_err());
        assert!(
            write_res
                .unwrap_err()
                .to_string()
                .contains("Security Exception")
        );
    }
}
