use anyhow::Result;
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

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
            .ok_or_else(|| anyhow::anyhow!("bash: missing 'command' arg"))?;

        let timeout_secs = args["timeout"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        // C-02: Defence-in-depth — log suspicious patterns even if the caller
        // already approved the command.
        if cade_core::permissions::bash_command_is_suspicious(command) {
            tracing::warn!(
                "bash: executing suspicious command (approved by caller): {:?}",
                command.chars().take(120).collect::<String>()
            );
        }

        let output = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            Command::new("bash")
                .arg("-c")
                .arg(command)
                .output(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Command timed out after {timeout_secs}s"))?
        .map_err(|e| anyhow::anyhow!("Failed to spawn bash: {e}"))?;

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

    pub fn schema() -> serde_json::Value {
        serde_json::json!({
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
