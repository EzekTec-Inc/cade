use crate::Result;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// M-03: Cap bash output returned to the LLM to avoid blowing the context window.
/// ~16k chars ≈ 4k tokens — enough for build output, test results, etc.
const MAX_OUTPUT_CHARS: usize = 16_384;

pub struct BashTool;

impl BashTool {
    /// Execute `command` in a shell. Returns combined stdout+stderr.
    ///
    /// # C-02 / defence-in-depth
    /// This function is a last-resort safety net: it logs a warning when a
    /// suspicious command pattern is detected.  The primary permission gate is
    /// `PermissionManager::is_blocked()` in `cli/repl.rs` and `cli/headless.rs`,
    /// which is called BEFORE this function.  The check here ensures that any
    /// code path that calls `BashTool::run()` directly (e.g. tests, future
    /// integrations) still gets an audit trail.
    pub async fn run(args: &Value) -> Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("bash: missing 'command' arg"))?;

        let timeout_secs = args["timeout"].as_u64().unwrap_or(DEFAULT_TIMEOUT_SECS);

        // C-02: Defence-in-depth — log suspicious patterns even if the caller
        // already approved the command.
        if cade_core::permissions::bash_command_is_suspicious(command) {
            tracing::warn!(
                "bash: executing suspicious command (approved by caller): {:?}",
                command.chars().take(120).collect::<String>()
            );
        }

        let output = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
            let mut cmd = cade_core::shell::shell_command(command);
            cade_core::agent_env::apply_agent_env(&mut cmd);
            cmd.output().await
        })
        .await
        .map_err(|_| crate::Error::custom(format!("Command timed out after {timeout_secs}s")))?
        .map_err(|e| crate::Error::custom(format!("Failed to spawn bash: {e}")))?;

        let mut result = String::from_utf8_lossy(&output.stdout).to_string();

        if !output.stderr.is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("STDERR:\n");
            result.push_str(&stderr);
        }

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            if result.is_empty() {
                result = format!("(exit code {code})");
            } else {
                result.push_str(&format!("\n(exit code {code})"));
            }
        }

        // M-03: Truncate large outputs so the LLM context window is not blown.
        if result.len() > MAX_OUTPUT_CHARS {
            let truncated = &result[..MAX_OUTPUT_CHARS];
            result = format!(
                "{truncated}\n\n[...output truncated — {} chars omitted. Use head/tail/grep to narrow output.]",
                result.len() - MAX_OUTPUT_CHARS
            );
        }

        Ok(result)
    }

    /// Stream a bash command line-by-line, calling `on_line` for every output
    /// line as it arrives (stdout and stderr interleaved in arrival order).
    ///
    /// Returns the full accumulated output string (for the LLM — identical to
    /// what `run()` would return) and propagates any spawn or timeout error.
    ///
    /// The caller is responsible for showing a `LiveOutput` RenderLine and
    /// passing a closure that appends each line to it.
    pub async fn run_streaming<F>(args: &Value, mut on_line: F) -> Result<String>
    where
        F: FnMut(String) + Send,
    {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("bash: missing 'command' arg"))?;

        let timeout_secs = args["timeout"].as_u64().unwrap_or(DEFAULT_TIMEOUT_SECS);

        if cade_core::permissions::bash_command_is_suspicious(command) {
            tracing::warn!(
                "bash: executing suspicious command (approved by caller): {:?}",
                command.chars().take(120).collect::<String>()
            );
        }

        use std::process::Stdio;
        let mut child = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
            let mut cmd = cade_core::shell::shell_command(command);
            cade_core::agent_env::apply_agent_env(&mut cmd);
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
        })
        .await
        .map_err(|_| crate::Error::custom(format!("Command timed out after {timeout_secs}s")))?
        .map_err(|e| crate::Error::custom(format!("Failed to spawn bash: {e}")))?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        // Merge stdout and stderr into one channel so lines arrive in
        // roughly the order the process writes them.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        let tx_out = tx.clone();
        let out_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx_out.send(line);
            }
        });

        let tx_err = tx.clone();
        let err_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = tx_err.send(line);
            }
        });
        // Drop the original sender so rx closes when both tasks finish.
        drop(tx);

        let mut accumulated = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(line)) => {
                    on_line(line.clone());
                    accumulated.push_str(&line);
                    accumulated.push('\n');
                }
                Ok(None) => break, // channel closed — both tasks done
                Err(_) => {
                    // Timeout: kill the child and stop reading.
                    let _ = child.kill().await;
                    return Err(crate::Error::custom(format!(
                        "Command timed out after {timeout_secs}s"
                    )));
                }
            }
        }

        // Wait for the reader tasks and the child process.
        let _ = out_task.await;
        let _ = err_task.await;
        let status = child
            .wait()
            .await
            .map_err(|e| crate::Error::custom(format!("{e}")))?;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            if accumulated.is_empty() {
                accumulated = format!("(exit code {code})");
            } else {
                accumulated.push_str(&format!("\n(exit code {code})"));
            }
        }

        // M-03: same truncation as run().
        if accumulated.len() > MAX_OUTPUT_CHARS {
            let truncated = &accumulated[..MAX_OUTPUT_CHARS];
            accumulated = format!(
                "{truncated}\n\n[...output truncated — {} chars omitted. Use head/tail/grep to narrow output.]",
                accumulated.len() - MAX_OUTPUT_CHARS
            );
        }

        Ok(accumulated)
    }

    pub fn schema() -> serde_json::Value {
        json!({
            "name": "bash",
            "description": "Execute a shell command. Returns stdout/stderr. Use for builds, tests, git, file inspection, and anything requiring a shell.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The bash command to execute"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Timeout in seconds (default 120)",
                        "default": 120
                    }
                },
                "required": ["command"]
            }
        })
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn run_simple_command() -> Result<()> {
        // -- Exec
        let args = json!({"command": "echo hello world"});
        let output = BashTool::run(&args).await?;

        // -- Check
        assert!(output.contains("hello world"), "got: {output}");
        Ok(())
    }

    #[tokio::test]
    async fn run_command_with_exit_code() -> Result<()> {
        // -- Exec
        let args = json!({"command": "exit 42"});
        let output = BashTool::run(&args).await?;

        // -- Check
        assert!(output.contains("exit code 42"), "got: {output}");
        Ok(())
    }

    #[tokio::test]
    async fn run_command_with_stderr() -> Result<()> {
        // -- Exec
        let args = json!({"command": "echo error >&2"});
        let output = BashTool::run(&args).await?;

        // -- Check
        assert!(output.contains("STDERR:"), "got: {output}");
        assert!(output.contains("error"), "got: {output}");
        Ok(())
    }

    #[tokio::test]
    async fn run_missing_command_arg() {
        // -- Exec & Check
        let args = json!({"timeout": 5});
        let result = BashTool::run(&args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing"));
    }

    #[tokio::test]
    async fn run_command_timeout() {
        // -- Exec & Check
        let args = json!({"command": "sleep 60", "timeout": 1});
        let result = BashTool::run(&args).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("timed out"), "got: {msg}");
    }

    #[tokio::test]
    async fn run_truncates_large_output() -> Result<()> {
        // -- Exec
        let args = json!({"command": "yes 'aaaaaaaaaa' | head -5000"});
        let output = BashTool::run(&args).await?;

        // -- Check
        if output.len() > MAX_OUTPUT_CHARS + 200 {
            panic!("output should be truncated, got {} chars", output.len());
        }
        Ok(())
    }

    #[tokio::test]
    async fn streaming_simple_command() -> Result<()> {
        // -- Setup & Fixtures
        let args = json!({"command": "echo line1; echo line2"});
        let mut lines_seen = Vec::new();

        // -- Exec
        let output = BashTool::run_streaming(&args, |line| {
            lines_seen.push(line);
        })
        .await?;

        // -- Check
        assert!(output.contains("line1"), "got: {output}");
        assert!(output.contains("line2"), "got: {output}");
        assert!(!lines_seen.is_empty(), "should have seen lines streamed");
        Ok(())
    }

    #[tokio::test]
    async fn streaming_timeout() {
        // -- Exec & Check
        let args = json!({"command": "sleep 60", "timeout": 1});
        let result = BashTool::run_streaming(&args, |_| {}).await;
        assert!(result.is_err());
    }

    #[test]
    fn schema_is_valid() -> Result<()> {
        // -- Exec
        let schema = BashTool::schema();

        // -- Check
        assert_eq!(schema["name"], "bash");
        let desc = schema["description"]
            .as_str()
            .ok_or("Should have description")?;
        assert!(desc.len() > 10);
        assert!(schema["parameters"]["properties"]["command"].is_object());
        Ok(())
    }
}

// endregion: --- Tests
