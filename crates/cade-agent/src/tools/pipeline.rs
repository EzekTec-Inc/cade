//! Deep Tool Execution Pipeline (`ToolPipeline`).
//!
//! Encapsulates:
//! - Permission evaluation & authorization (`PermissionManager` + `ApprovalDelegate`)
//! - PreToolUse hook execution (with cancellation and parameter overrides)
//! - Tool routing and dispatching (`ToolRuntime`)
//! - PostToolUse / PostToolUseFailure hook dispatching
//! - Comprehensive error normalization and audit telemetry

// region:    --- Imports

use std::sync::Arc;
use serde_json::Value;
use tracing::{debug, info, warn};

use cade_core::hooks::{HookEngine, HookOutcome};
use cade_core::permissions::{PermissionManager, PermissionMode, Verdict, is_write_schema};

use crate::tools::runtime::{RuntimeToolResult, ToolRuntime};
use crate::Result;

// endregion: --- Imports

// region:    --- Approval Delegate Seam

/// Seam for delegating write/mutation approvals to caller environments
/// (e.g. interactive TUI timeline in `cade-cli` or async queue in `cade-server`).
#[async_trait::async_trait]
pub trait ApprovalDelegate: Send + Sync {
    /// Request approval from the user or remote queue for a tool execution.
    async fn request_approval(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &Value,
        reason: &str,
    ) -> Result<bool>;
}

/// Auto-approving delegate (for headless tests, batch evaluation, and bypass modes).
#[derive(Debug, Default, Clone, Copy)]
pub struct AutoApprovalDelegate;

#[async_trait::async_trait]
impl ApprovalDelegate for AutoApprovalDelegate {
    async fn request_approval(
        &self,
        _tool_call_id: &str,
        _tool_name: &str,
        _arguments: &Value,
        _reason: &str,
    ) -> Result<bool> {
        Ok(true)
    }
}

/// Deny-all delegate (for strictly sandboxed or read-only execution modes).
#[derive(Debug, Default, Clone, Copy)]
pub struct DenyAllApprovalDelegate;

#[async_trait::async_trait]
impl ApprovalDelegate for DenyAllApprovalDelegate {
    async fn request_approval(
        &self,
        _tool_call_id: &str,
        _tool_name: &str,
        _arguments: &Value,
        _reason: &str,
    ) -> Result<bool> {
        Ok(false)
    }
}

// endregion: --- Approval Delegate Seam

// region:    --- Pipeline Outcome

/// Normalized result of a pipeline tool execution.
#[derive(Debug, Clone)]
pub struct PipelineOutcome {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub is_error: bool,
    pub ui_resource_uri: Option<String>,
    pub was_blocked: bool,
    pub permission_denied: bool,
}

impl PipelineOutcome {
    pub fn success(tool_call_id: String, tool_name: String, output: String) -> Self {
        Self {
            tool_call_id,
            tool_name,
            output,
            is_error: false,
            ui_resource_uri: None,
            was_blocked: false,
            permission_denied: false,
        }
    }

    pub fn error(tool_call_id: String, tool_name: String, output: String) -> Self {
        Self {
            tool_call_id,
            tool_name,
            output,
            is_error: true,
            ui_resource_uri: None,
            was_blocked: false,
            permission_denied: false,
        }
    }

    pub fn blocked(tool_call_id: String, tool_name: String, reason: String) -> Self {
        Self {
            tool_call_id,
            tool_name,
            output: format!("[Blocked by hook: {reason}]"),
            is_error: true,
            ui_resource_uri: None,
            was_blocked: true,
            permission_denied: false,
        }
    }

    pub fn denied(tool_call_id: String, tool_name: String, reason: String) -> Self {
        Self {
            tool_call_id,
            tool_name,
            output: format!("[Permission Denied] {reason}"),
            is_error: true,
            ui_resource_uri: None,
            was_blocked: false,
            permission_denied: true,
        }
    }
}

// endregion: --- Pipeline Outcome

// region:    --- Tool Pipeline

/// Deep module orchestrating end-to-end tool execution, security, hooks, and metrics.
#[derive(Clone)]
pub struct ToolPipeline {
    runtime: Arc<ToolRuntime>,
    permissions: PermissionManager,
    hooks: Arc<HookEngine>,
    approval_delegate: Arc<dyn ApprovalDelegate>,
}

impl ToolPipeline {
    /// Construct a new deep ToolPipeline.
    pub fn new(
        runtime: Arc<ToolRuntime>,
        permissions: PermissionManager,
        hooks: Arc<HookEngine>,
        approval_delegate: Arc<dyn ApprovalDelegate>,
    ) -> Self {
        Self {
            runtime,
            permissions,
            hooks,
            approval_delegate,
        }
    }

    /// Access the underlying permissions manager.
    pub fn permissions(&self) -> &PermissionManager {
        &self.permissions
    }

    /// Access the underlying hook engine.
    pub fn hooks(&self) -> &Arc<HookEngine> {
        &self.hooks
    }

    /// Access the underlying runtime.
    pub fn runtime(&self) -> &Arc<ToolRuntime> {
        &self.runtime
    }

    /// Execute a tool call end-to-end through the complete security, hook, and execution pipeline.
    pub async fn execute(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<PipelineOutcome> {
        debug!(
            target: "cade_agent::pipeline",
            tool_call_id = %tool_call_id,
            tool_name = %tool_name,
            "ToolPipeline: evaluating tool execution"
        );

        // 1. Resolve canonical tool name
        let canonical_owned: String = {
            use cade_core::toolsets::Toolset;
            use cade_core::toolsets::adapter::ToolSurfaceAdapter;
            let ga = ToolSurfaceAdapter::for_toolset(Toolset::Gemini);
            ga.to_canonical(tool_name).to_string()
        };
        let canonical = canonical_owned.as_str();

        // 2. Evaluate permission rules & plan-mode write blocking
        let is_mcp_write = crate::tools::is_mcp_write_tool(canonical, self.runtime.mcp()).await;
        let is_write = is_write_schema(canonical) || is_mcp_write;

        if self.permissions.mode() == PermissionMode::Plan && is_write {
            warn!(
                target: "cade_agent::pipeline",
                tool_name = %canonical,
                "Tool execution blocked by Plan mode (read-only)"
            );
            return Ok(PipelineOutcome::denied(
                tool_call_id.to_string(),
                tool_name.to_string(),
                format!(
                    "Tool '{tool_name}' is a mutating operation and is forbidden while in Plan Mode. Switch to Default mode or exit plan mode to execute."
                ),
            ));
        }

        let verdict = self.permissions.resolve(canonical, arguments, is_mcp_write);
        match verdict {
            Verdict::Deny(reason) => {
                warn!(
                    target: "cade_agent::pipeline",
                    tool_name = %canonical,
                    reason = %reason,
                    "Tool execution denied by permission rule"
                );
                return Ok(PipelineOutcome::denied(
                    tool_call_id.to_string(),
                    tool_name.to_string(),
                    reason,
                ));
            }
            Verdict::Ask(reason) => {
                debug!(
                    target: "cade_agent::pipeline",
                    tool_name = %canonical,
                    reason = %reason,
                    "Requesting permission from ApprovalDelegate"
                );
                let approved = self
                    .approval_delegate
                    .request_approval(tool_call_id, tool_name, arguments, &reason)
                    .await?;

                if !approved {
                    info!(
                        target: "cade_agent::pipeline",
                        tool_name = %canonical,
                        "Tool execution rejected by user/queue"
                    );
                    return Ok(PipelineOutcome::denied(
                        tool_call_id.to_string(),
                        tool_name.to_string(),
                        "User or supervisor denied permission to execute tool.".to_string(),
                    ));
                }
            }
            Verdict::Allow => {
                debug!(
                    target: "cade_agent::pipeline",
                    tool_name = %canonical,
                    "Tool auto-approved by permission policy"
                );
            }
        }

        // 3. Execute PreToolUse hooks
        let effective_args = arguments.clone();
        let pre_hook_outcome = self.hooks.pre_tool_use(tool_name, &effective_args).await;

        if let HookOutcome::Block { reason } = pre_hook_outcome {
            warn!(
                target: "cade_agent::pipeline",
                tool_name = %tool_name,
                reason = %reason,
                "Tool execution blocked by PreToolUse hook"
            );
            return Ok(PipelineOutcome::blocked(
                tool_call_id.to_string(),
                tool_name.to_string(),
                reason,
            ));
        }

        // 4. Dispatch tool execution via ToolRuntime
        let run_result = self
            .runtime
            .execute(tool_call_id.to_string(), canonical, &effective_args)
            .await;

        let (mut output, is_error, ui_resource_uri) = match run_result {
            Some(RuntimeToolResult {
                output,
                is_error,
                ui_resource_uri,
                ..
            }) => (output, is_error, ui_resource_uri),
            None => (
                format!("Tool '{tool_name}' not supported in current execution runtime."),
                true,
                None,
            ),
        };

        // 5. Execute PostToolUse or PostToolUseFailure hooks
        if is_error {
            self.hooks
                .post_tool_use_failure(tool_name, &effective_args, &output, None, None)
                .await;
        } else if let Some(extra_context) = self
            .hooks
            .post_tool_use(tool_name, &effective_args, &output, None, None)
            .await
            && !extra_context.is_empty()
        {
            output.push_str("\n\n[Hook context: ");
            output.push_str(&extra_context);
            output.push(']');
        }

        Ok(PipelineOutcome {
            tool_call_id: tool_call_id.to_string(),
            tool_name: tool_name.to_string(),
            output,
            is_error,
            ui_resource_uri,
            was_blocked: false,
            permission_denied: false,
        })
    }
}

// endregion: --- Tool Pipeline

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_pipeline(
        mode: PermissionMode,
        delegate: Arc<dyn ApprovalDelegate>,
    ) -> Result<ToolPipeline> {
        let storage = Arc::new(crate::agent::HttpTransport::new(
            "http://localhost:0".to_string(),
            "fake-key".to_string(),
        )?);
        let mcp = Arc::new(crate::mcp::McpManager::empty());
        let runtime = Arc::new(ToolRuntime::new(
            storage,
            mcp,
            "test-agent".to_string(),
            std::env::temp_dir(),
        ));
        let permissions = PermissionManager::new(mode);
        let hooks = Arc::new(HookEngine::new(
            cade_core::settings::HooksConfig::default(),
            std::env::temp_dir(),
            "test-session".to_string(),
        ));
        Ok(ToolPipeline::new(runtime, permissions, hooks, delegate))
    }

    #[tokio::test]
    async fn test_auto_approved_read_tool() -> Result<()> {
        let pipeline = create_test_pipeline(
            PermissionMode::Default,
            Arc::new(AutoApprovalDelegate),
        )?;
        let outcome = pipeline
            .execute("call_1", "list_checkpoints", &json!({}))
            .await?;

        assert_eq!(outcome.tool_call_id, "call_1");
        assert!(!outcome.permission_denied);
        assert!(!outcome.was_blocked);
        Ok(())
    }

    #[tokio::test]
    async fn test_plan_mode_blocks_write_tool() -> Result<()> {
        let pipeline = create_test_pipeline(
            PermissionMode::Plan,
            Arc::new(AutoApprovalDelegate),
        )?;
        let outcome = pipeline
            .execute("call_2", "write_file", &json!({"path": "test.txt", "content": "abc"}))
            .await?;

        assert!(outcome.permission_denied);
        assert!(outcome.is_error);
        assert!(outcome.output.contains("Plan Mode"));
        Ok(())
    }

    #[tokio::test]
    async fn test_delegate_denial_blocks_execution() -> Result<()> {
        let pipeline = create_test_pipeline(
            PermissionMode::Default,
            Arc::new(DenyAllApprovalDelegate),
        )?;
        let outcome = pipeline
            .execute("call_3", "write_file", &json!({"path": "test.txt", "content": "abc"}))
            .await?;

        assert!(outcome.permission_denied);
        assert!(outcome.is_error);
        assert!(outcome.output.contains("denied permission"));
        Ok(())
    }
}

// endregion: --- Tests
