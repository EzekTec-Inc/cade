use anyhow::Result;
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

const DEFAULT_TIMEOUT_SECS: u64 = 120;

pub struct BashTool;

impl BashTool {
    /// Execute `command` in a shell. Returns combined stdout+stderr.
    pub async fn run(args: &Value) -> Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("bash: missing 'command' arg"))?;

        let timeout_secs = args["timeout"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

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
