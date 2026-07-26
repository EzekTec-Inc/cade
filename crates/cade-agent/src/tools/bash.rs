use crate::Result;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use cade_core::shell::{ShellExecutionEngine, ShellRequest, DEFAULT_TIMEOUT_SECS, MAX_OUTPUT_CHARS, truncate_head_tail};

pub struct BashTool;

impl BashTool {
    /// Execute `command` in a shell. Returns combined stdout+stderr.
    ///
    /// # C-02 / defence-in-depth
    /// This function is a last-resort safety net: it logs a warning when a
    /// suspicious command pattern is detected. The primary permission gate is
    /// `PermissionManager::is_blocked()` in `cli/repl.rs` and `cli/headless.rs`,
    /// which is called BEFORE this function.
    pub async fn run(args: &Value) -> Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| crate::Error::custom("bash: missing 'command' arg"))?;

        let timeout_secs = args["timeout"].as_u64().unwrap_or(DEFAULT_TIMEOUT_SECS);

        let engine = ShellExecutionEngine::new();
        let req = ShellRequest::new(command)
            .with_timeout(Duration::from_secs(timeout_secs));

        let res = engine.execute(req).await.map_err(|e| crate::Error::custom(e.to_string()))?;
        Ok(res.format_for_llm())
    }

    /// Stream a bash command line-by-line, calling `on_line` for every output
    /// line as it arrives (stdout and stderr interleaved in arrival order).
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
            cade_core::askpass::apply_askpass_env(&mut cmd);
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
        })
        .await
        .map_err(|_| crate::Error::custom(format!("Command timed out after {timeout_secs}s")))?
        .map_err(|e| crate::Error::custom(format!("Failed to spawn bash: {e}")))?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

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
                Ok(None) => break,
                Err(_) => {
                    let _ = child.kill().await;
                    return Err(crate::Error::custom(format!(
                        "Command timed out after {timeout_secs}s"
                    )));
                }
            }
        }

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

        let (truncated_res, _) = truncate_head_tail(&accumulated, MAX_OUTPUT_CHARS);
        Ok(truncated_res)
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
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn run_simple_command() -> Result<()> {
        let args = json!({"command": "echo hello world"});
        let output = BashTool::run(&args).await?;

        assert!(output.contains("hello world"), "got: {output}");
        Ok(())
    }

    #[tokio::test]
    async fn run_command_with_exit_code() -> Result<()> {
        let args = json!({"command": "exit 42"});
        let output = BashTool::run(&args).await?;

        assert!(output.contains("exit code 42"), "got: {output}");
        Ok(())
    }

    #[tokio::test]
    async fn run_command_with_stderr() -> Result<()> {
        let args = json!({"command": "echo error >&2"});
        let output = BashTool::run(&args).await?;

        assert!(output.contains("STDERR:"), "got: {output}");
        assert!(output.contains("error"), "got: {output}");
        Ok(())
    }

    #[tokio::test]
    async fn run_missing_command_arg() {
        let args = json!({"timeout": 5});
        let result = BashTool::run(&args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing"));
    }

    #[tokio::test]
    async fn run_command_timeout() {
        let args = json!({"command": "sleep 60", "timeout": 1});
        let result = BashTool::run(&args).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("timed out"), "got: {msg}");
    }

    #[tokio::test]
    async fn run_truncates_large_output() -> Result<()> {
        let args = json!({"command": "yes 'aaaaaaaaaa' | head -5000"});
        let output = BashTool::run(&args).await?;

        if output.len() > MAX_OUTPUT_CHARS + 500 {
            panic!("output should be truncated, got {} chars", output.len());
        }
        Ok(())
    }

    #[tokio::test]
    async fn streaming_simple_command() -> Result<()> {
        let args = json!({"command": "echo line1; echo line2"});
        let mut lines_seen = Vec::new();

        let output = BashTool::run_streaming(&args, |line| {
            lines_seen.push(line);
        })
        .await?;

        assert!(output.contains("line1"), "got: {output}");
        assert!(output.contains("line2"), "got: {output}");
        assert!(!lines_seen.is_empty(), "should have seen lines streamed");
        Ok(())
    }

    #[tokio::test]
    async fn streaming_timeout() {
        let args = json!({"command": "sleep 60", "timeout": 1});
        let result = BashTool::run_streaming(&args, |_| {}).await;
        assert!(result.is_err());
    }

    #[test]
    fn schema_is_valid() -> Result<()> {
        let schema = BashTool::schema();

        assert_eq!(schema["name"], "bash");
        let desc = schema["description"]
            .as_str()
            .ok_or("Should have description")?;
        assert!(desc.len() > 10);
        assert!(schema["parameters"]["properties"]["command"].is_object());
        Ok(())
    }
}
