//! Core ConstitutionGovernor Trait Seam & Dynamic Anti-Escape-Hatch Sniffer (Issue #60 / PRD #59).
//!
//! Enforces non-negotiable project constitutions, blocks unmanaged subprocess/script
//! workarounds for MCP servers, and provides a clean seam for constraint adherence.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// region:    --- Types

/// Verdict returned after constitutional evaluation of an intended capability call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GovernorVerdict {
    /// Action conforms strictly to project constitutional rules.
    Pass,
    /// A constitutional boundary or missing asset was encountered; halt immediately.
    YieldToUser {
        reason: String,
        suggested_action: String,
    },
}

/// Specific constitutional violation reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConstitutionViolation {
    /// Detected an attempt to spawn external subprocesses/scripts to emulate MCP tools.
    BypassAttemptBlocked {
        tool: String,
        script_pattern: String,
        reason: String,
    },
    /// Attempted to mutate source code outside the designated AST engine.
    InvalidCodeMutationTool { tool: String, path: String },
    /// General constitutional rule breach.
    RuleViolation(String),
}

impl std::fmt::Display for ConstitutionViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BypassAttemptBlocked {
                tool,
                script_pattern,
                reason,
            } => {
                write!(
                    f,
                    "Constitutional Violation: Tool '{tool}' attempted to bypass managed MCP connections via unmanaged script/subprocess pattern '{script_pattern}'. Reason: {reason}"
                )
            }
            Self::InvalidCodeMutationTool { tool, path } => {
                write!(
                    f,
                    "Constitutional Violation: Tool '{tool}' is not authorized to mutate source code at '{path}'. Use designated AST tools."
                )
            }
            Self::RuleViolation(r) => write!(f, "Constitutional Violation: {r}"),
        }
    }
}

impl std::error::Error for ConstitutionViolation {}

/// Unified, deep interface for enforcing project constitutions and anti-workaround boundaries.
#[async_trait]
pub trait ConstitutionGovernor: Send + Sync {
    /// Evaluates an intended capability execution against dynamic project constitutions.
    async fn evaluate_intent(
        &self,
        capability_name: &str,
        arguments: &Value,
    ) -> Result<GovernorVerdict, ConstitutionViolation>;
}

// endregion: --- Types

// region:    --- Dynamic Constitution Governor

/// Dynamic production implementation of ConstitutionGovernor.
pub struct DynamicConstitutionGovernor {
    configured_mcp_commands: Vec<String>,
}

impl DynamicConstitutionGovernor {
    pub fn new(configured_mcp_commands: Vec<String>) -> Self {
        Self {
            configured_mcp_commands,
        }
    }

    pub fn from_mcp_configs(
        configs: &std::collections::HashMap<String, crate::settings::models::McpServerConfig>,
    ) -> Self {
        let mut cmds = Vec::new();
        for cfg in configs.values() {
            if !cfg.command.trim().is_empty() {
                cmds.push(cfg.command.clone());
                if let Some(binary_name) = std::path::Path::new(&cfg.command)
                    .file_name()
                    .and_then(|n| n.to_str())
                {
                    cmds.push(binary_name.to_string());
                }
            }
        }
        Self::new(cmds)
    }

    /// Sniff shell command payload for attempts to emulate MCP communication or invoke MCP binaries.
    pub fn sniff_shell_command(&self, command: &str) -> Option<ConstitutionViolation> {
        let trimmed = command.trim();

        // 1. Check for inline script execution attempting to spawn subprocesses or JSON-RPC
        let suspicious_script_patterns = [
            ("python3 -c", "inline Python script"),
            ("python -c", "inline Python script"),
            ("node -e", "inline Node evaluation"),
            ("ruby -e", "inline Ruby execution"),
            ("perl -e", "inline Perl execution"),
        ];

        for (pattern, desc) in suspicious_script_patterns {
            if trimmed.contains(pattern)
                && (trimmed.contains("jsonrpc")
                    || trimmed.contains("subprocess")
                    || trimmed.contains("start-mcp-server")
                    || trimmed.contains("tools/call"))
            {
                return Some(ConstitutionViolation::BypassAttemptBlocked {
                    tool: "bash".to_string(),
                    script_pattern: pattern.to_string(),
                    reason: format!(
                        "Detected attempt to emulate MCP client connection via {desc}. All MCP operations must strictly use CADE's native MCP connection."
                    ),
                });
            }
        }

        // 2. Check if command directly invokes any of the configured MCP server binaries via shell
        for mcp_cmd in &self.configured_mcp_commands {
            if !mcp_cmd.trim().is_empty()
                && trimmed.contains(mcp_cmd)
                && (trimmed.contains("start-mcp-server")
                    || trimmed.contains("tools/call")
                    || trimmed.contains("jsonrpc")
                    || trimmed.contains("tools/list"))
            {
                return Some(ConstitutionViolation::BypassAttemptBlocked {
                    tool: "bash".to_string(),
                    script_pattern: mcp_cmd.clone(),
                    reason: format!(
                        "Detected direct shell execution of configured MCP binary '{mcp_cmd}'. All MCP tools must be executed through CADE's native tool calling interface."
                    ),
                });
            }
        }

        None
    }

    /// Check if an unauthorized generic tool attempts to mutate source code files.
    pub fn sniff_code_mutation(
        &self,
        capability_name: &str,
        arguments: &Value,
    ) -> Option<ConstitutionViolation> {
        let is_generic_write = matches!(
            capability_name,
            "write_file"
                | "edit_file"
                | "replace_in_file"
                | "developer__write_file"
                | "developer__replace_in_file"
        );

        if !is_generic_write {
            return None;
        }

        let target_path = arguments
            .get("path")
            .or_else(|| arguments.get("relative_path"))
            .or_else(|| arguments.get("file_path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if target_path.ends_with(".rs")
            || target_path.ends_with(".ts")
            || target_path.ends_with(".js")
            || target_path.ends_with(".py")
            || target_path.ends_with(".lua")
        {
            return Some(ConstitutionViolation::InvalidCodeMutationTool {
                tool: capability_name.to_string(),
                path: target_path.to_string(),
            });
        }

        None
    }
}

#[async_trait]
impl ConstitutionGovernor for DynamicConstitutionGovernor {
    async fn evaluate_intent(
        &self,
        capability_name: &str,
        arguments: &Value,
    ) -> Result<GovernorVerdict, ConstitutionViolation> {
        if matches!(capability_name, "bash" | "shell" | "RunShellCommand") {
            let cmd = arguments
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(violation) = self.sniff_shell_command(cmd) {
                return Err(violation);
            }
        }

        if let Some(violation) = self.sniff_code_mutation(capability_name, arguments) {
            return Err(violation);
        }

        Ok(GovernorVerdict::Pass)
    }
}

// endregion: --- Dynamic Constitution Governor

// region:    --- Mock Constitution Governor

/// Mock implementation for testing.
pub struct MockConstitutionGovernor {
    pub canned_verdict: Result<GovernorVerdict, ConstitutionViolation>,
}

impl Default for MockConstitutionGovernor {
    fn default() -> Self {
        Self {
            canned_verdict: Ok(GovernorVerdict::Pass),
        }
    }
}

#[async_trait]
impl ConstitutionGovernor for MockConstitutionGovernor {
    async fn evaluate_intent(
        &self,
        _capability_name: &str,
        _arguments: &Value,
    ) -> Result<GovernorVerdict, ConstitutionViolation> {
        self.canned_verdict.clone()
    }
}

// endregion: --- Mock Constitution Governor
