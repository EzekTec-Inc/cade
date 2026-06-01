/// Pluggable execution backends.
///
/// All tool operations that touch the OS (bash, file read/write) go through
/// an `ExecutionBackend`.  Swapping the backend changes where commands run:
///
///  - [`LocalBackend`]   — current process, same machine (default)
///  - [`DockerBackend`]  — inside a Docker container via `docker exec`
///  - [`SshBackend`]     — on a remote host via SSH
///  - [`ReadOnlyBackend`] — wraps any backend, blocks all writes
///
/// The active backend is stored in [`crate::tools::ToolRuntime`] and selected
/// from [`cade_core::settings::ExecutionProfile`].
#[cfg(feature = "backend-docker")]
pub mod docker;
pub mod local;
pub mod readonly;
#[cfg(feature = "backend-ssh")]
pub mod ssh;
pub mod storage;
pub mod virtual_sandbox;

#[cfg(feature = "backend-docker")]
pub use docker::DockerBackend;
pub use local::LocalBackend;
pub use readonly::ReadOnlyBackend;
#[cfg(feature = "backend-ssh")]
pub use ssh::SshBackend;
pub use virtual_sandbox::VirtualSandboxBackend;

use std::path::{Path, PathBuf};

// region:    --- BashOutput

/// Output from executing a shell command.
#[derive(Debug, Clone, Default)]
pub struct BashOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

impl BashOutput {
    /// Build the combined output string that gets sent to the LLM.
    pub fn combined(&self) -> String {
        const MAX: usize = 16_384;
        let mut out = self.stdout.clone();
        if !self.stderr.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("STDERR:\n");
            out.push_str(&self.stderr);
        }
        if self.timed_out {
            out.push_str("\n(command timed out)");
        } else if self.exit_code != 0 {
            let msg = format!("\n(exit code {})", self.exit_code);
            if out.is_empty() {
                out = msg;
            } else {
                out.push_str(&msg);
            }
        }
        if out.len() > MAX {
            let truncated = &out[..MAX];
            out = format!(
                "{truncated}\n\n[...output truncated — {} chars omitted]",
                out.len() - MAX
            );
        }
        out
    }
}

// endregion: --- BashOutput

// region:    --- DirEntry

/// A single entry in a directory listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: Option<u64>,
}

// endregion: --- DirEntry

// region:    --- ExecutionBackend trait

/// Trait for pluggable execution environments.
///
/// All async methods take self by shared reference so the backend can be
/// stored in an `Arc<dyn ExecutionBackend>` and used from multiple tasks.
#[async_trait::async_trait]
pub trait ExecutionBackend: Send + Sync {
    // -- Shell execution

    /// Execute a shell command and return combined output.
    async fn exec_bash(
        &self,
        command: &str,
        cwd: &Path,
        timeout_secs: u64,
    ) -> crate::Result<BashOutput>;

    // -- Filesystem operations

    /// Read a file and return its contents.
    async fn read_file(&self, path: &Path) -> crate::Result<String>;

    /// Write content to a file (creates parent directories as needed).
    async fn write_file(&self, path: &Path, content: &str) -> crate::Result<()>;

    /// Check whether a path exists.
    async fn path_exists(&self, path: &Path) -> bool;

    /// List the contents of a directory.
    async fn list_dir(&self, path: &Path) -> crate::Result<Vec<DirEntry>>;

    // -- Capabilities

    /// Returns true if write operations are permitted on this backend.
    fn is_writable(&self) -> bool {
        true
    }

    /// Human-readable backend name for display in the footer/status.
    fn name(&self) -> &'static str;
}

// endregion: --- ExecutionBackend trait

// region:    --- Factory

use cade_core::settings::ExecutionProfile;

/// Build a boxed `ExecutionBackend` from the given profile.
pub fn backend_from_profile(profile: &ExecutionProfile) -> Box<dyn ExecutionBackend> {
    use cade_core::settings::ExecutionBackendKind;
    match profile.backend {
        ExecutionBackendKind::Docker => {
            #[cfg(feature = "backend-docker")]
            {
                Box::new(DockerBackend {
                    image: profile
                        .docker_image
                        .clone()
                        .unwrap_or_else(|| "ubuntu:22.04".to_string()),
                    extra_flags: profile.docker_flags.clone(),
                })
            }
            #[cfg(not(feature = "backend-docker"))]
            {
                tracing::warn!(
                    "CADE was compiled without Docker backend support. Reverting to local."
                );
                Box::new(LocalBackend)
            }
        }
        ExecutionBackendKind::Ssh => {
            #[cfg(feature = "backend-ssh")]
            {
                Box::new(SshBackend {
                    host: profile.ssh_host.clone().unwrap_or_default(),
                    user: profile.ssh_user.clone().unwrap_or_else(whoami_user),
                    key_path: profile.ssh_key_path.as_ref().map(std::path::PathBuf::from),
                    port: profile.ssh_port.unwrap_or(22),
                })
            }
            #[cfg(not(feature = "backend-ssh"))]
            {
                tracing::warn!(
                    "CADE was compiled without SSH backend support. Reverting to local."
                );
                Box::new(LocalBackend)
            }
        }
        ExecutionBackendKind::ReadOnly => Box::new(ReadOnlyBackend::new(LocalBackend)),
        ExecutionBackendKind::Local => Box::new(LocalBackend),
        ExecutionBackendKind::Virtual => Box::new(VirtualSandboxBackend::new(
            std::env::current_dir().unwrap_or_default(),
        )),
    }
}

#[cfg(feature = "backend-ssh")]
fn whoami_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .or_else(|_| std::env::var("USERNAME")) // Windows
        .unwrap_or_else(|_| "root".to_string())
}

// endregion: --- Factory
