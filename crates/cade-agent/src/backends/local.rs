/// Local execution backend — runs commands on the current machine.
///
/// This is the default backend and mirrors the previous hardcoded behaviour
/// of `BashTool` and `ReadTool` / `WriteTool`.
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::{BashOutput, DirEntry, ExecutionBackend};

// region:    --- LocalBackend

pub struct LocalBackend;

#[async_trait]
impl ExecutionBackend for LocalBackend {
    async fn exec_bash(
        &self,
        command: &str,
        cwd: &Path,
        timeout_secs: u64,
    ) -> crate::Result<BashOutput> {
        use std::process::Stdio;
        let mut child = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
            let mut cmd = cade_core::shell::shell_command(command);
            cade_core::agent_env::apply_agent_env(&mut cmd);
            cade_core::askpass::apply_askpass_env(&mut cmd);
            cmd.current_dir(cwd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
        })
        .await
        .map_err(|_| crate::Error::custom(format!("timed out after {timeout_secs}s")))?
        .map_err(|e| crate::Error::custom(format!("spawn bash: {e}")))?;

        let stdout_h = child
            .stdout
            .take()
            .ok_or_else(|| crate::Error::custom("failed to open stdout pipe"))?;
        let stderr_h = child
            .stderr
            .take()
            .ok_or_else(|| crate::Error::custom("failed to open stderr pipe"))?;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(bool, String)>();
        let tx_out = tx.clone();
        let tx_err = tx.clone();
        drop(tx);

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout_h).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx_out.send((false, line));
            }
        });
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr_h).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx_err.send((true, line));
            }
        });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let mut out = BashOutput::default();

        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some((is_err, line))) => {
                    if is_err {
                        out.stderr.push_str(&line);
                        out.stderr.push('\n');
                    } else {
                        out.stdout.push_str(&line);
                        out.stdout.push('\n');
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    let _ = child.kill().await;
                    out.timed_out = true;
                    return Ok(out);
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| crate::Error::custom(format!("{e}")))?;
        out.exit_code = status.code().unwrap_or(-1);
        Ok(out)
    }

    async fn read_file(&self, path: &Path) -> crate::Result<String> {
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
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    fn name(&self) -> &'static str {
        "local"
    }
}

// endregion: --- LocalBackend

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_exec_bash_echo() {
        // -- Setup & Fixtures
        let b = LocalBackend;
        let cwd = std::env::current_dir().unwrap();

        // -- Exec
        let out = b.exec_bash("echo hello", &cwd, 10).await.unwrap();

        // -- Check
        assert!(out.combined().contains("hello"), "got: {}", out.combined());
        assert_eq!(out.exit_code, 0);
    }

    #[tokio::test]
    async fn test_local_exec_bash_exit_code() {
        // -- Setup & Fixtures
        let b = LocalBackend;
        let cwd = std::env::current_dir().unwrap();

        // -- Exec
        let out = b.exec_bash("exit 1", &cwd, 5).await.unwrap();

        // -- Check
        assert_eq!(out.exit_code, 1);
    }

    #[tokio::test]
    async fn test_local_write_read_roundtrip() -> crate::Result<()> {
        // -- Setup & Fixtures
        let b = LocalBackend;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("test.txt");

        // -- Exec
        b.write_file(&f, "hello roundtrip").await?;
        let content = b.read_file(&f).await?;

        // -- Check
        assert_eq!(content, "hello roundtrip");
        Ok(())
    }
}

// endregion: --- Tests
