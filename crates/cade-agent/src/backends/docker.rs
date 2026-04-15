/// Docker execution backend — runs commands inside a container.
///
/// All bash execution goes through `docker exec <container> bash -c <cmd>`.
/// File operations mount `/workspace` from the host; the default mount path
/// is the agent's cwd.
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use super::{BashOutput, DirEntry, ExecutionBackend};

// region:    --- DockerBackend

/// Runs commands inside a Docker container.
///
/// Can either start a fresh container per command (`run` mode, using `image`)
/// or exec into an existing container (`exec` mode, using `container_id`).
pub struct DockerBackend {
    /// Docker image to use for `docker run` invocations.
    pub image: String,
    /// Extra flags passed to `docker run` (e.g. `["--network=none"]`).
    pub extra_flags: Vec<String>,
}

impl DockerBackend {
    /// Build a `docker run --rm` invocation for one-shot commands.
    fn build_run_cmd(&self, command: &str, cwd: &Path) -> Command {
        let mut cmd = Command::new("docker");
        let cwd_str = cwd.to_string_lossy();
        cmd.args([
            "run",
            "--rm",
            "-v",
            &format!("{cwd_str}:/workspace"),
            "-w",
            "/workspace",
        ]);
        for flag in &self.extra_flags {
            cmd.arg(flag);
        }
        cmd.arg(&self.image);
        // Use POSIX `sh` instead of `bash` for Alpine and minimal image compat.
        cmd.args(["sh", "-c", command]);
        cade_core::agent_env::apply_agent_env(&mut cmd);
        cmd
    }
}

#[async_trait]
impl ExecutionBackend for DockerBackend {
    async fn exec_bash(
        &self,
        command: &str,
        cwd: &Path,
        timeout_secs: u64,
    ) -> crate::Result<BashOutput> {
        // Warn if Docker is not available
        let docker_check = Command::new("docker").arg("info").output().await;
        let is_running = docker_check
            .map(|out| out.status.success())
            .unwrap_or(false);
        if !is_running {
            return Err(crate::Error::custom(
                "Docker is not available. Install Docker or switch to local backend.",
            ));
        }

        let mut cmd = self.build_run_cmd(command, cwd);
        let result = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
            .await
            .map_err(|_| {
                crate::Error::custom(format!("docker run timed out after {timeout_secs}s"))
            })?
            .map_err(|e| crate::Error::custom(format!("docker run: {e}")))?;

        Ok(BashOutput {
            stdout: String::from_utf8_lossy(&result.stdout).to_string(),
            stderr: String::from_utf8_lossy(&result.stderr).to_string(),
            exit_code: result.status.code().unwrap_or(-1),
            timed_out: false,
        })
    }

    async fn read_file(&self, path: &Path) -> crate::Result<String> {
        // Files are mounted, so local read is fine
        tokio::fs::read_to_string(path)
            .await
            .map_err(|e| crate::Error::custom(format!("read {}: {e}", path.display())))
    }

    async fn write_file(&self, path: &Path, content: &str) -> crate::Result<()> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| crate::Error::custom(format!("create dirs: {e}")))?;
        }
        tokio::fs::write(path, content)
            .await
            .map_err(|e| crate::Error::custom(format!("write {}: {e}", path.display())))
    }

    async fn path_exists(&self, path: &Path) -> bool {
        tokio::fs::metadata(path).await.is_ok()
    }

    async fn list_dir(&self, path: &Path) -> crate::Result<Vec<DirEntry>> {
        let mut rd = tokio::fs::read_dir(path)
            .await
            .map_err(|e| crate::Error::custom(format!("read_dir {}: {e}", path.display())))?;
        let mut entries = Vec::new();
        while let Ok(Some(e)) = rd.next_entry().await {
            let meta = e.metadata().await.ok();
            entries.push(DirEntry {
                path: e.path(),
                is_dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                size: meta.map(|m| m.len()),
            });
        }
        Ok(entries)
    }

    fn name(&self) -> &'static str {
        "docker"
    }
}

// endregion: --- DockerBackend
