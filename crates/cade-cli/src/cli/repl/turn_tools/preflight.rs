use super::super::{Repl, ToolPreflightResult};
use crate::Result;
use crate::ui::RenderLine;
use std::io;
use super::super::turn_loop::{now_epoch_ms, blocked_result};

impl Repl {
    /// Phase 1: Sequential preflight — checks permissions, plan-mode blocking,
    /// hooks, and prompts the user for approval if needed.
    /// Returns `Approved` if the tool should proceed, or `Blocked(result)` if it
    /// was denied (with a pre-built error ToolResult).
    pub(crate) async fn preflight_tool(
        &self,
        stdout: &mut io::Stdout,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<ToolPreflightResult> {
        let canonical_name = cade_agent::tools::manager::canonical_name(tool_name);
        let is_mcp_write = cade_agent::tools::is_mcp_write_tool(tool_name, &self.mcp).await;

        // Unified permission resolution
        use cade_core::permissions::Verdict;
        match self.permissions.resolve(canonical_name, args, is_mcp_write) {
            Verdict::Deny(msg) => {
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::ToolResult {
                        is_error: true,
                        content: msg.clone(),
                    });
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                return Ok(blocked_result(call_id, tool_name, msg));
            }

            Verdict::Ask(_reason) => {
                // PermissionRequest hook — can block before showing prompt
                if let cade_core::hooks::HookOutcome::Block { reason } =
                    self.hooks.permission_request(tool_name, args).await
                {
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::ToolResult {
                        is_error: true,
                        content: format!("Hook denied: {reason}"),
                    });
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                return Ok(blocked_result(call_id, tool_name, format!("Hook denied: {reason}")));
            }

            // Prompt for approval
            if !self.prompt_approval(stdout, tool_name, args).await? {
                { let mut stats = self.session_stats.lock();
                    stats.reviewed += 1;
                }
                let msg = format!("Tool '{tool_name}' denied by user");
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::ToolResult {
                        is_error: true,
                        content: msg.clone(),
                    });
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                return Ok(blocked_result(call_id, tool_name, msg));
            }
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            { let mut stats = self.session_stats.lock();
                stats.reviewed += 1;
                stats.approved += 1;
            }
            }

            Verdict::Allow => {
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                self.last_modal_close_ms.store(
                    now_epoch_ms(),
                    std::sync::atomic::Ordering::SeqCst,
                );
            }
        }

        // PreToolUse hook — can block execution
        if let cade_core::hooks::HookOutcome::Block { reason } =
            self.hooks.pre_tool_use(tool_name, args).await
        {
            let _ = self
                .app
                .lock()
                .push(RenderLine::ToolResult {
                    is_error: true,
                    content: format!("Hook blocked: {reason}"),
                });
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            return Ok(blocked_result(call_id, tool_name, format!("Blocked by hook: {reason}")));
        }

        Ok(ToolPreflightResult::Approved)
    }

}
