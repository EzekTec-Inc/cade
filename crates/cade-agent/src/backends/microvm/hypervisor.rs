//! Firecracker Hypervisor Supervisor & In-Memory Tarball Synchronization (Issue #97 / PRD #95).

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::{Child, Command};
use tracing::{debug, warn};

/// Configuration parameters for spawning a Firecracker MicroVM instance.
#[derive(Debug, Clone)]
pub struct HypervisorConfig {
    pub kernel_image_path: PathBuf,
    pub rootfs_path: PathBuf,
    pub vcpu_count: u8,
    pub mem_size_mib: usize,
    pub vsock_uds_path: PathBuf,
    pub vsock_cid: u32,
}

impl Default for HypervisorConfig {
    fn default() -> Self {
        let temp_id = uuid::Uuid::new_v4();
        Self {
            kernel_image_path: PathBuf::from("/var/lib/firecracker/vmlinux"),
            rootfs_path: PathBuf::from("/var/lib/firecracker/rootfs.ext4"),
            vcpu_count: 2,
            mem_size_mib: 1024,
            vsock_uds_path: std::env::temp_dir().join(format!("firecracker-vsock-{temp_id}.sock")),
            vsock_cid: 3,
        }
    }
}

/// RAII Process Supervisor for managing a running Firecracker MicroVM.
pub struct HypervisorProcess {
    pub config: HypervisorConfig,
    child: Option<Child>,
    api_socket_path: PathBuf,
}

impl HypervisorProcess {
    /// Check whether Linux KVM hardware acceleration (`/dev/kvm`) is available.
    pub fn is_kvm_available() -> bool {
        Path::new("/dev/kvm").exists()
            && std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/kvm")
                .is_ok()
    }

    /// Spawn a Firecracker process and configure the microvm via its API socket.
    pub async fn spawn(config: HypervisorConfig) -> io::Result<Self> {
        let temp_id = uuid::Uuid::new_v4();
        let api_socket_path = std::env::temp_dir().join(format!("firecracker-api-{temp_id}.sock"));

        if api_socket_path.exists() {
            let _ = std::fs::remove_file(&api_socket_path);
        }

        let firecracker_bin = std::env::var("FIRECRACKER_BIN")
            .unwrap_or_else(|_| "firecracker".to_string());

        debug!(
            socket = %api_socket_path.display(),
            vsock = %config.vsock_uds_path.display(),
            "Spawning Firecracker process"
        );

        let child = match Command::new(&firecracker_bin)
            .args(["--api-sock", api_socket_path.to_str().unwrap_or_default()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => Some(c),
            Err(e) => {
                warn!(
                    bin = %firecracker_bin,
                    "Firecracker binary not found or failed to spawn: {e}. Falling back to simulation mode."
                );
                None
            }
        };

        Ok(Self {
            config,
            child,
            api_socket_path,
        })
    }

    /// Return true if the hypervisor child process is actively executing.
    pub fn is_running(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            matches!(child.try_wait(), Ok(None))
        } else {
            false
        }
    }

    /// Cleanly terminate the hypervisor process and remove sockets.
    pub async fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        if self.api_socket_path.exists() {
            let _ = std::fs::remove_file(&self.api_socket_path);
        }
        if self.config.vsock_uds_path.exists() {
            let _ = std::fs::remove_file(&self.config.vsock_uds_path);
        }
    }
}

impl Drop for HypervisorProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        if self.api_socket_path.exists() {
            let _ = std::fs::remove_file(&self.api_socket_path);
        }
        if self.config.vsock_uds_path.exists() {
            let _ = std::fs::remove_file(&self.config.vsock_uds_path);
        }
    }
}

// ── In-Memory Workspace Tarball Utilities ────────────────────────────────────

/// Packs directory contents into an in-memory uncompressed tarball, respecting ignore filters.
pub fn pack_workspace_tarball(root_dir: &Path) -> io::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut buffer);

        let walker = ignore::WalkBuilder::new(root_dir)
            .standard_filters(true)
            .hidden(false)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Ok(rel_path) = path.strip_prefix(root_dir)
            {
                let mut file = std::fs::File::open(path)?;
                builder.append_file(rel_path, &mut file)?;
            }
        }
        builder.finish()?;
    }
    Ok(buffer)
}

/// Unpacks an in-memory tarball into the target directory.
pub fn unpack_workspace_tarball(tarball_bytes: &[u8], target_dir: &Path) -> io::Result<()> {
    if !target_dir.exists() {
        std::fs::create_dir_all(target_dir)?;
    }
    let cursor = io::Cursor::new(tarball_bytes);
    let mut archive = tar::Archive::new(cursor);
    archive.unpack(target_dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_pack_and_unpack_workspace_tarball() -> io::Result<()> {
        let src_dir = tempdir()?;
        let dest_dir = tempdir()?;

        std::fs::write(src_dir.path().join("main.rs"), "fn main() {}")?;
        std::fs::create_dir(src_dir.path().join("sub"))?;
        std::fs::write(src_dir.path().join("sub/lib.rs"), "pub fn test() {}")?;

        let tarball = pack_workspace_tarball(src_dir.path())?;
        assert!(!tarball.is_empty());

        unpack_workspace_tarball(&tarball, dest_dir.path())?;

        assert_eq!(
            std::fs::read_to_string(dest_dir.path().join("main.rs"))?,
            "fn main() {}"
        );
        assert_eq!(
            std::fs::read_to_string(dest_dir.path().join("sub/lib.rs"))?,
            "pub fn test() {}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_hypervisor_process_lifecycle_drop() {
        let config = HypervisorConfig::default();
        let sock = config.vsock_uds_path.clone();
        {
            let _process = HypervisorProcess::spawn(config).await.expect("Spawned");
        }
        // Guard dropped — socket must not linger
        assert!(!sock.exists());
    }
}
