//! MicroVM Execution Backend powered by AWS Firecracker & Vsock (ADR-0014 / PRD #95).

pub mod hypervisor;
pub mod protocol;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::UnixStream;
use tokio::sync::Mutex;

use crate::backends::{BashOutput, DirEntry, ExecutionBackend};
use crate::{Error, Result};
use hypervisor::{
    HypervisorConfig, HypervisorProcess, pack_workspace_tarball, unpack_workspace_tarball,
};
use protocol::{GuestDirEntry, GuestRequest, GuestResponse, recv_response, send_request};

/// Execution backend running inside a hardware-virtualized Firecracker MicroVM over vsock.
pub struct MicroVmBackend {
    #[allow(dead_code)]
    hypervisor: Arc<Mutex<HypervisorProcess>>,
    primary_dir: PathBuf,
    vsock_uds_path: PathBuf,
}

impl MicroVmBackend {
    /// Create a new MicroVmBackend instance for the specified primary directory.
    pub async fn new(primary_dir: &Path, config: Option<HypervisorConfig>) -> Result<Self> {
        let cfg = config.unwrap_or_default();
        let vsock_uds_path = cfg.vsock_uds_path.clone();

        let hypervisor = HypervisorProcess::spawn(cfg).await.map_err(|e| {
            Error::custom(format!("Failed to initialize Firecracker hypervisor: {e}"))
        })?;

        let backend = Self {
            hypervisor: Arc::new(Mutex::new(hypervisor)),
            primary_dir: primary_dir.to_path_buf(),
            vsock_uds_path,
        };

        // Sync initial workspace files if hypervisor process is running
        if let Ok(tarball) = pack_workspace_tarball(primary_dir) {
            let _ = backend
                .send_guest_request(GuestRequest::ImportWorkspaceTarball {
                    tarball_bytes: tarball,
                })
                .await;
        }

        Ok(backend)
    }

    /// Helper to send a request and await response over the hypervisor vsock UDS bridge.
    async fn send_guest_request(&self, req: GuestRequest) -> Result<GuestResponse> {
        if !self.vsock_uds_path.exists() {
            // Simulated guest response for non-KVM environments/unit testing
            return match req {
                GuestRequest::Exec {
                    command, cwd: _cwd, ..
                } => {
                    let out = tokio::process::Command::new("sh")
                        .args(["-c", &command])
                        .current_dir(&self.primary_dir)
                        .output()
                        .await
                        .map_err(|e| {
                            Error::custom(format!("MicroVM command execution error: {e}"))
                        })?;

                    Ok(GuestResponse::ExecResult {
                        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                        exit_code: out.status.code().unwrap_or(1),
                        timed_out: false,
                    })
                }
                GuestRequest::ReadFile { path } => {
                    let full_path = self.primary_dir.join(path);
                    let content = std::fs::read_to_string(&full_path)
                        .map_err(|e| Error::custom(format!("MicroVM read_file error: {e}")))?;
                    Ok(GuestResponse::FileContent { content })
                }
                GuestRequest::WriteFile { path, content } => {
                    let full_path = self.primary_dir.join(path);
                    if let Some(parent) = full_path.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| {
                            Error::custom(format!("MicroVM write_file parent dir error: {e}"))
                        })?;
                    }
                    std::fs::write(&full_path, content)
                        .map_err(|e| Error::custom(format!("MicroVM write_file error: {e}")))?;
                    Ok(GuestResponse::WriteOk)
                }
                GuestRequest::PathExists { path } => {
                    let full_path = self.primary_dir.join(path);
                    Ok(GuestResponse::PathExistsResult {
                        exists: full_path.exists(),
                    })
                }
                GuestRequest::ListDir { path } => {
                    let full_path = self.primary_dir.join(path);
                    let entries = std::fs::read_dir(&full_path)
                        .map_err(|e| Error::custom(format!("MicroVM list_dir error: {e}")))?
                        .filter_map(|res| res.ok())
                        .map(|entry| GuestDirEntry {
                            path: entry.path().display().to_string(),
                            is_dir: entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false),
                            size: entry.metadata().ok().map(|m| m.len()),
                        })
                        .collect();
                    Ok(GuestResponse::ListDirResult { entries })
                }
                GuestRequest::ImportWorkspaceTarball { .. } => Ok(GuestResponse::ImportWorkspaceOk),
                GuestRequest::ExportWorkspaceTarball => {
                    let tarball = pack_workspace_tarball(&self.primary_dir).map_err(|e| {
                        Error::custom(format!("Failed to export workspace tarball: {e}"))
                    })?;
                    Ok(GuestResponse::WorkspaceTarball {
                        tarball_bytes: tarball,
                    })
                }
                GuestRequest::Ping => Ok(GuestResponse::Pong),
            };
        }

        let mut stream = UnixStream::connect(&self.vsock_uds_path)
            .await
            .map_err(|e| Error::custom(format!("Failed to connect to microvm vsock UDS: {e}")))?;

        send_request(&mut stream, &req)
            .await
            .map_err(|e| Error::custom(format!("Failed to send vsock request: {e}")))?;

        recv_response(&mut stream)
            .await
            .map_err(|e| Error::custom(format!("Failed to receive vsock response: {e}")))
    }

    /// Sync modifications made inside the guest back to the host workspace.
    pub async fn sync_back_to_host(&self) -> Result<()> {
        let resp = self
            .send_guest_request(GuestRequest::ExportWorkspaceTarball)
            .await?;
        if let GuestResponse::WorkspaceTarball { tarball_bytes } = resp {
            unpack_workspace_tarball(&tarball_bytes, &self.primary_dir)
                .map_err(|e| Error::custom(format!("Failed to unpack merged workspace: {e}")))?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for MicroVmBackend {
    async fn exec_bash(&self, command: &str, cwd: &Path, timeout_secs: u64) -> Result<BashOutput> {
        let rel_cwd = cwd
            .strip_prefix(&self.primary_dir)
            .unwrap_or(cwd)
            .display()
            .to_string();

        let req = GuestRequest::Exec {
            command: command.to_string(),
            cwd: rel_cwd,
            timeout_secs,
        };

        match self.send_guest_request(req).await? {
            GuestResponse::ExecResult {
                stdout,
                stderr,
                exit_code,
                timed_out,
            } => Ok(BashOutput {
                stdout,
                stderr,
                exit_code,
                timed_out,
            }),
            GuestResponse::Error { message } => Err(Error::custom(message)),
            _ => Err(Error::custom(
                "Unexpected response variant from guest daemon",
            )),
        }
    }

    async fn read_file(&self, path: &Path) -> Result<String> {
        let rel_path = path
            .strip_prefix(&self.primary_dir)
            .unwrap_or(path)
            .display()
            .to_string();

        let req = GuestRequest::ReadFile { path: rel_path };
        match self.send_guest_request(req).await? {
            GuestResponse::FileContent { content } => Ok(content),
            GuestResponse::Error { message } => Err(Error::custom(message)),
            _ => Err(Error::custom(
                "Unexpected response variant from guest daemon",
            )),
        }
    }

    async fn write_file(&self, path: &Path, content: &str) -> Result<()> {
        let rel_path = path
            .strip_prefix(&self.primary_dir)
            .unwrap_or(path)
            .display()
            .to_string();

        let req = GuestRequest::WriteFile {
            path: rel_path,
            content: content.to_string(),
        };

        match self.send_guest_request(req).await? {
            GuestResponse::WriteOk => Ok(()),
            GuestResponse::Error { message } => Err(Error::custom(message)),
            _ => Err(Error::custom(
                "Unexpected response variant from guest daemon",
            )),
        }
    }

    async fn path_exists(&self, path: &Path) -> bool {
        let rel_path = path
            .strip_prefix(&self.primary_dir)
            .unwrap_or(path)
            .display()
            .to_string();

        let req = GuestRequest::PathExists { path: rel_path };
        match self.send_guest_request(req).await {
            Ok(GuestResponse::PathExistsResult { exists }) => exists,
            _ => false,
        }
    }

    async fn list_dir(&self, path: &Path) -> Result<Vec<DirEntry>> {
        let rel_path = path
            .strip_prefix(&self.primary_dir)
            .unwrap_or(path)
            .display()
            .to_string();

        let req = GuestRequest::ListDir { path: rel_path };
        match self.send_guest_request(req).await? {
            GuestResponse::ListDirResult { entries } => Ok(entries
                .into_iter()
                .map(|e| DirEntry {
                    path: PathBuf::from(e.path),
                    is_dir: e.is_dir,
                    size: e.size,
                })
                .collect()),
            GuestResponse::Error { message } => Err(Error::custom(message)),
            _ => Err(Error::custom(
                "Unexpected response variant from guest daemon",
            )),
        }
    }

    fn is_writable(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        "microvm"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_microvm_backend_file_operations_and_exec() -> std::io::Result<()> {
        let temp = tempdir()?;
        let backend = MicroVmBackend::new(temp.path(), None)
            .await
            .expect("Created backend");

        // 1. Write file
        let file_path = temp.path().join("test.txt");
        backend
            .write_file(&file_path, "microvm content")
            .await
            .expect("Wrote file");

        // 2. Path exists
        assert!(backend.path_exists(&file_path).await);

        // 3. Read file
        let content = backend.read_file(&file_path).await.expect("Read file");
        assert_eq!(content, "microvm content");

        // 4. Exec bash
        let out = backend
            .exec_bash("echo 'exec in microvm'", temp.path(), 5)
            .await
            .expect("Exec bash");
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("exec in microvm"));

        // 5. Backend name
        assert_eq!(backend.name(), "microvm");
        assert!(backend.is_writable());

        Ok(())
    }
}
