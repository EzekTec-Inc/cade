/// SSH execution backend — runs commands on a remote host.
///
/// Uses the system `ssh` binary (respects `~/.ssh/config`).
/// File operations are done via `sftp` / `scp`.
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use super::{BashOutput, DirEntry, ExecutionBackend};

// region:    --- SshBackend

pub struct SshBackend {
    /// Remote host name or IP.
    pub host: String,
    /// Remote username.
    pub user: String,
    /// Path to the SSH private key file (uses SSH agent / default if None).
    pub key_path: Option<PathBuf>,
    /// SSH port (default 22).
    pub port: u16,
}

impl SshBackend {
    fn base_ssh_args(&self) -> Vec<String> {
        let mut args = vec![
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "ConnectTimeout=10".to_string(),
            "-o".to_string(),
            "StrictHostKeyChecking=accept-new".to_string(),
            "-p".to_string(),
            self.port.to_string(),
        ];
        if let Some(key) = &self.key_path {
            args.push("-i".to_string());
            args.push(key.to_string_lossy().to_string());
        }
        args.push(format!("{}@{}", self.user, self.host));
        args
    }

    /// Run a single command on the remote host, optionally in a specific directory.
    async fn run_remote(
        &self,
        command: &str,
        cwd: &Path,
        timeout_secs: u64,
    ) -> crate::Result<BashOutput> {
        let cwd_str = cwd.to_string_lossy();
        // Wrap command with `cd <cwd> &&` so it runs in the right directory
        let wrapped = format!("cd {cwd_str:?} && {command}");

        let mut cmd = Command::new("ssh");
        for arg in self.base_ssh_args() {
            cmd.arg(arg);
        }
        cmd.arg("--").arg("bash").arg("-c").arg(&wrapped);

        let result = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
            .await
            .map_err(|_| crate::Error::custom(format!("ssh timed out after {timeout_secs}s")))?
            .map_err(|e| crate::Error::custom(format!("ssh: {e}")))?;

        Ok(BashOutput {
            stdout: String::from_utf8_lossy(&result.stdout).to_string(),
            stderr: String::from_utf8_lossy(&result.stderr).to_string(),
            exit_code: result.status.code().unwrap_or(-1),
            timed_out: false,
        })
    }
}

#[async_trait]
impl ExecutionBackend for SshBackend {
    async fn exec_bash(
        &self,
        command: &str,
        cwd: &Path,
        timeout_secs: u64,
    ) -> crate::Result<BashOutput> {
        if self.host.is_empty() {
            return Err(crate::Error::custom("SSH backend: host is not configured"));
        }
        self.run_remote(command, cwd, timeout_secs).await
    }

    async fn read_file(&self, path: &Path) -> crate::Result<String> {
        // Use `ssh <host> cat <path>` to read a remote file
        let mut cmd = Command::new("ssh");
        for arg in self.base_ssh_args() {
            cmd.arg(arg);
        }
        cmd.arg("cat").arg(path.to_string_lossy().as_ref());
        let out = cmd
            .output()
            .await
            .map_err(|e| crate::Error::custom(format!("ssh cat: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(crate::Error::custom(format!(
                "remote read failed: {stderr}"
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    async fn write_file(&self, path: &Path, content: &str) -> crate::Result<()> {
        // Use `ssh <host> tee <path>` with stdin
        use tokio::io::AsyncWriteExt;
        let mut cmd = Command::new("ssh");
        for arg in self.base_ssh_args() {
            cmd.arg(arg);
        }
        cmd.arg("tee").arg(path.to_string_lossy().as_ref());
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::null());

        let mut child = cmd
            .spawn()
            .map_err(|e| crate::Error::custom(format!("ssh tee: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(content.as_bytes())
                .await
                .map_err(|e| crate::Error::custom(format!("ssh write stdin: {e}")))?;
        }
        let status = child
            .wait()
            .await
            .map_err(|e| crate::Error::custom(format!("ssh tee wait: {e}")))?;
        if !status.success() {
            return Err(crate::Error::custom("remote write failed"));
        }
        Ok(())
    }

    async fn path_exists(&self, path: &Path) -> bool {
        let mut cmd = Command::new("ssh");
        for arg in self.base_ssh_args() {
            cmd.arg(arg);
        }
        cmd.arg("test")
            .arg("-e")
            .arg(path.to_string_lossy().as_ref());
        cmd.output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn list_dir(&self, path: &Path) -> crate::Result<Vec<DirEntry>> {
        // Use `ls -la --time-style=long-iso` on the remote
        let cmd_str = format!("ls -1pF {:?}", path.to_string_lossy().to_string());
        let out = self.run_remote(&cmd_str, path, 10).await?;
        let entries: Vec<DirEntry> = out
            .stdout
            .lines()
            .filter(|l| !l.is_empty() && *l != "./" && *l != "../")
            .map(|l| {
                let is_dir = l.ends_with('/');
                let name = l.trim_end_matches('/').trim_end_matches('*');
                DirEntry {
                    path: path.join(name),
                    is_dir,
                    size: None,
                }
            })
            .collect();
        Ok(entries)
    }

    fn name(&self) -> &'static str {
        "ssh"
    }
}

// endregion: --- SshBackend
