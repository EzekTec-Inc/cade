/// Docker execution backend — runs commands inside a persistent container.
///
/// All bash execution goes through `docker exec <container> sh -c <cmd>`.
/// File operations write and read strictly inside the isolated container using `docker cp`.
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use super::{BashOutput, DirEntry, ExecutionBackend};

pub struct DockerBackend {
    /// Docker image to use for `docker run` invocations.
    pub image: String,
    /// Extra flags passed to `docker run` (e.g. `["--network=none"]`).
    pub extra_flags: Vec<String>,
    /// Active persistent container ID (initialized lazily)
    container_id: tokio::sync::Mutex<Option<String>>,
}

impl DockerBackend {
    pub fn new(image: String, extra_flags: Vec<String>) -> Self {
        Self {
            image,
            extra_flags,
            container_id: tokio::sync::Mutex::new(None),
        }
    }

    /// Retrieve or spawn a persistent detached background container session (ADR 6).
    async fn get_or_start_container(&self) -> crate::Result<String> {
        let mut guard = self.container_id.lock().await;
        if let Some(id) = &*guard {
            return Ok(id.clone());
        }

        // Check if Docker is available before starting
        let docker_check = Command::new("docker").arg("info").output().await;
        let is_running = docker_check
            .map(|out| out.status.success())
            .unwrap_or(false);
        if !is_running {
            return Err(crate::Error::custom(
                "Docker is not available. Install Docker or switch to local backend."
            ));
        }

        tracing::info!("Spawning persistent background Docker container from image '{}'...", self.image);
        let mut cmd = Command::new("docker");
        cmd.args(["run", "-d", "--rm"]);
        for flag in &self.extra_flags {
            cmd.arg(flag);
        }
        cmd.args([&self.image, "tail", "-f", "/dev/null"]);
        
        let out = cmd.output().await.map_err(|e| crate::Error::custom(format!("failed to spawn docker container: {e}")))?;
        if !out.status.success() {
            let err_msg = String::from_utf8_lossy(&out.stderr).to_string();
            return Err(crate::Error::custom(format!("docker run -d failed: {err_msg}")));
        }

        let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        
        // Ensure /workspace directory exists inside the container
        let _ = Command::new("docker")
            .args(["exec", &id, "mkdir", "-p", "/workspace"])
            .output()
            .await;

        tracing::info!("Docker container persistent session active: {}", id);
        *guard = Some(id.clone());
        Ok(id)
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
        let container_id = self.get_or_start_container().await?;
        
        let mut cmd = Command::new("docker");
        cmd.args([
            "exec",
            "-w",
            &format!("/workspace/{}", cwd.to_string_lossy()),
            &container_id,
            "sh",
            "-c",
            command,
        ]);
        cmd.kill_on_drop(true);

        let result = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
            .await
            .map_err(|_| {
                crate::Error::custom(format!("docker exec timed out after {timeout_secs}s"))
            })?
            .map_err(|e| crate::Error::custom(format!("docker exec: {e}")))?;

        Ok(BashOutput {
            stdout: String::from_utf8_lossy(&result.stdout).to_string(),
            stderr: String::from_utf8_lossy(&result.stderr).to_string(),
            exit_code: result.status.code().unwrap_or(-1),
            timed_out: false,
        })
    }

    async fn read_file(&self, path: &Path) -> crate::Result<String> {
        let container_id = self.get_or_start_container().await?;
        let temp_file = tempfile::NamedTempFile::new().map_err(|e| crate::Error::custom(e.to_string()))?;

        let src = format!("{}:/workspace/{}", container_id, path.to_string_lossy());

        // Run docker cp
        let out = Command::new("docker")
            .args(["cp", &src, temp_file.path().to_str().unwrap()])
            .output()
            .await
            .map_err(|e| crate::Error::custom(format!("docker cp failed: {e}")))?;

        if !out.status.success() {
            let err_msg = String::from_utf8_lossy(&out.stderr).to_string();
            return Err(crate::Error::custom(format!("docker cp: {err_msg}")));
        }

        tokio::fs::read_to_string(temp_file.path())
            .await
            .map_err(|e| crate::Error::custom(format!("failed to read temp file: {e}")))
    }

    async fn write_file(&self, path: &Path, content: &str) -> crate::Result<()> {
        let container_id = self.get_or_start_container().await?;
        let temp_file = tempfile::NamedTempFile::new().map_err(|e| crate::Error::custom(e.to_string()))?;
        std::fs::write(temp_file.path(), content).map_err(|e| crate::Error::custom(e.to_string()))?;

        let dest = format!("{}:/workspace/{}", container_id, path.to_string_lossy());

        // Run docker cp
        let out = Command::new("docker")
            .args(["cp", temp_file.path().to_str().unwrap(), &dest])
            .output()
            .await
            .map_err(|e| crate::Error::custom(format!("docker cp failed: {e}")))?;

        if !out.status.success() {
            let err_msg = String::from_utf8_lossy(&out.stderr).to_string();
            return Err(crate::Error::custom(format!("docker cp: {err_msg}")));
        }
        Ok(())
    }

    async fn path_exists(&self, path: &Path) -> bool {
        let container_id = match self.get_or_start_container().await {
            Ok(id) => id,
            Err(_) => return false,
        };
        
        let path_str = path.to_string_lossy();
        let out_check = Command::new("docker")
            .args(["exec", &container_id, "sh", "-c", &format!("test -e /workspace/{}", path_str)])
            .output()
            .await;
            
        out_check.map(|out| out.status.success()).unwrap_or(false)
    }

    async fn list_dir(&self, path: &Path) -> crate::Result<Vec<DirEntry>> {
        let container_id = self.get_or_start_container().await?;
        let path_str = path.to_string_lossy();
        
        let py_cmd = format!(
            "python3 -c \"import os, json; print(json.dumps([{{'name': e.name, 'is_dir': e.is_dir(), 'size': e.stat().st_size}} for e in os.scandir('/workspace/{}')]))\"",
            path_str
        );

        let mut cmd = Command::new("docker");
        cmd.args(["exec", &container_id, "sh", "-c", &py_cmd]);
        
        let out = cmd.output().await.map_err(|e| crate::Error::custom(format!("docker list_dir failed: {e}")))?;
        if !out.status.success() {
            // Fallback to simple ls -1 if python is missing
            let mut fallback_cmd = Command::new("docker");
            fallback_cmd.args(["exec", &container_id, "ls", "-1", &format!("/workspace/{}", path_str)]);
            let fallback_out = fallback_cmd.output().await.map_err(|e| crate::Error::custom(e.to_string()))?;
            let stdout_str = String::from_utf8_lossy(&fallback_out.stdout);
            let mut entries = Vec::new();
            for line in stdout_str.lines() {
                let name = line.trim().to_string();
                if !name.is_empty() {
                    entries.push(DirEntry {
                        path: path.join(name),
                        is_dir: false, // Fallback default
                        size: None,
                    });
                }
            }
            return Ok(entries);
        }

        let stdout_str = String::from_utf8_lossy(&out.stdout).to_string();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&stdout_str)
            .map_err(|e| crate::Error::custom(format!("failed to parse directory JSON: {e}")))?;

        let mut entries = Vec::new();
        for val in parsed {
            let name = val["name"].as_str().unwrap_or("").to_string();
            let is_dir = val["is_dir"].as_bool().unwrap_or(false);
            let size = val["size"].as_u64();
            entries.push(DirEntry {
                path: path.join(name),
                is_dir,
                size,
            });
        }
        Ok(entries)
    }

    fn name(&self) -> &'static str {
        "docker"
    }
}

impl Drop for DockerBackend {
    fn drop(&mut self) {
        if let Some(id) = self.container_id.get_mut().take() {
            tracing::info!("Stopping and removing persistent Docker container: {id}...");
            let _ = std::process::Command::new("docker")
                .args(["stop", &id])
                .output();
        }
    }
}
