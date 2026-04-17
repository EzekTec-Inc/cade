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
        // P2-3: cwd is POSIX single-quoted so hostile directory names
        // (e.g. `$(rm -rf ~)`, backticks, embedded quotes) cannot break
        // out of the `cd <cwd>` wrapper.  See `build_remote_cwd_command`
        // and its tests.
        let wrapped = build_remote_cwd_command(command, cwd);

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

// region:    --- Shell quoting (P2-3)

/// POSIX-safe single-quote wrap.
///
/// Wraps `s` in single quotes and escapes any embedded single quote as
/// `'\''` (close-quote, escaped quote, reopen-quote).  The result is
/// safe to splice into a `bash -c` string — no expansion, no command
/// substitution, no globbing applies inside single quotes.
fn posix_shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            // End current quoted run, emit an escaped literal quote,
            // then reopen the quoted run.
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Build the `bash -c` payload that runs `command` after `cd`-ing into
/// `cwd`.  `cwd` is shell-quoted; `command` is not (its contents are
/// the caller's contract and match the previous behaviour of this
/// backend).
fn build_remote_cwd_command(command: &str, cwd: &Path) -> String {
    let quoted_cwd = posix_shell_quote(&cwd.to_string_lossy());
    format!("cd {quoted_cwd} && {command}")
}

// endregion: --- Shell quoting (P2-3)

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
        // P2-3: quote the path for both the `ls` argument and the cwd
        // wrapper so hostile directory names can't inject commands.
        let path_str = path.to_string_lossy();
        let quoted_path = posix_shell_quote(&path_str);
        let cmd_str = format!("ls -1pF {quoted_path}");
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

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // -- P2-3: SSH cwd shell quoting
    //
    // Every test asserts the exact wire-format string that will be passed
    // to `bash -c` on the remote host.  The helper must produce a
    // POSIX-safe single-quoted cwd so hostile directory names cannot
    // break out of the `cd <cwd> && <cmd>` wrapper.

    #[test]
    fn build_cmd_quotes_plain_cwd_with_single_quotes() {
        let cwd = PathBuf::from("/tmp/project");
        let got = build_remote_cwd_command("ls", &cwd);
        assert_eq!(got, "cd '/tmp/project' && ls");
    }

    #[test]
    fn build_cmd_rejects_command_substitution_in_cwd() {
        // Hostile cwd: `$(rm -rf ~)` must be literal, not evaluated.
        let cwd = PathBuf::from("/tmp/$(rm -rf ~)");
        let got = build_remote_cwd_command("ls", &cwd);
        // With debug format (the OLD code) this would render as
        // "cd \"/tmp/$(rm -rf ~)\" && ls" which bash still expands
        // because `$(...)` is active inside double quotes.  The safe
        // rendering is single-quoted.
        assert_eq!(got, "cd '/tmp/$(rm -rf ~)' && ls");
        // Explicit negative check — no double-quote wrapping of cwd.
        assert!(
            !got.contains("\"/tmp/"),
            "cwd must not be double-quoted: {got}"
        );
    }

    #[test]
    fn build_cmd_rejects_backtick_in_cwd() {
        let cwd = PathBuf::from("/tmp/`id`");
        let got = build_remote_cwd_command("ls", &cwd);
        assert_eq!(got, "cd '/tmp/`id`' && ls");
    }

    #[test]
    fn build_cmd_rejects_quote_breakout_in_cwd() {
        // Classic breakout attempt: embedded single quote.
        // Must be escaped as '\'' inside a single-quoted string.
        let cwd = PathBuf::from("/tmp/x'; rm -rf ~; echo '");
        let got = build_remote_cwd_command("ls", &cwd);
        assert_eq!(got, "cd '/tmp/x'\\''; rm -rf ~; echo '\\''' && ls");
    }

    #[test]
    fn build_cmd_preserves_command_verbatim() {
        // The command itself is not re-quoted — it's the caller's
        // contract (same as before this fix).  Only the cwd is hardened.
        let cwd = PathBuf::from("/tmp");
        let got = build_remote_cwd_command("cargo test --workspace", &cwd);
        assert_eq!(got, "cd '/tmp' && cargo test --workspace");
    }

    // -- posix_shell_quote

    #[test]
    fn posix_shell_quote_wraps_plain_string() {
        assert_eq!(posix_shell_quote("/tmp/a"), "'/tmp/a'");
    }

    #[test]
    fn posix_shell_quote_escapes_embedded_single_quote() {
        // ' becomes '\''  — close quote, escaped quote, reopen quote.
        assert_eq!(posix_shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn posix_shell_quote_accepts_empty_string() {
        assert_eq!(posix_shell_quote(""), "''");
    }
}

// endregion: --- Tests
