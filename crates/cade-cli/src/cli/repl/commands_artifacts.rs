//! /artifacts command handler.

use crate::Result;
use super::Repl;

impl Repl {
    pub(crate) async fn cmd_link(
        &mut self,
        _pending_input: &mut Option<String>,
    ) -> Result<bool> {
            self.tui_dim("  Linking tools…");
            let client2 = self.client.clone();
            let mcp2 = std::sync::Arc::clone(&self.mcp);
            let toolset2 = *self.current_toolset.lock();
            let agent_id = self.agent_id();
            use cade_agent::agent::tools::{register_cade_tools, register_mcp_tools};
            let allow_agent_mode = self
                .settings
                .lock()
                .permission_settings()
                .allow_agent_mode_changes;
            let native_ids: Vec<String> =
                register_cade_tools(&client2, toolset2, allow_agent_mode)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|t| t.id)
                    .collect();
            let n_native = native_ids.len();
            if !native_ids.is_empty() {
                let _ = client2.attach_agent_tools(&agent_id, &native_ids).await;
            }
            let mcp_ids: Vec<String> =
                register_mcp_tools(&client2, mcp2.all_tool_schemas().await)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|t| t.id)
                    .collect();
            let n_mcp = mcp_ids.len();
            if !mcp_ids.is_empty() {
                let _ = client2.attach_agent_tools(&agent_id, &mcp_ids).await;
            }
            self.tui_ok(format!(
                "  ✓ Linked {n_native} native + {n_mcp} MCP tool(s)"
            ));
        Ok(false)
    }

    pub(crate) async fn cmd_unlink(
        &mut self,
    ) -> Result<bool> {
            let agent_id = self.agent_id();
            match self.client.detach_agent_tools(&agent_id).await {
                Ok(n) => self.tui_ok(format!("  ✓ Detached {n} tool(s) from agent")),
                Err(e) => self.tui_err(e.to_string()),
            }
        Ok(false)
    }

    pub(crate) async fn cmd_artifacts(
        &mut self,
    ) -> Result<bool> {
            if self.require_capability(
                cade_core::capabilities::Capability::Agentic,
                "/artifacts",
            ) {
                return Ok(false);
            }
            let agent_id = self.agent_id();
            match self.client.list_artifacts(&agent_id).await {
                Err(e) => self.tui_err(format!("  ✗ list_artifacts: {e}")),
                Ok(arts) if arts.is_empty() => {
                    self.tui_dim("  No artifacts stored yet.".to_string());
                }
                Ok(arts) => {
                    self.tui_hdr(format!("  Artifacts ({}):", arts.len()));
                    for a in arts.iter().take(20) {
                        let id = a["id"].as_str().unwrap_or("?");
                        let kind = a["kind"].as_str().unwrap_or("?");
                        let size = a["size_bytes"].as_i64().unwrap_or(0);
                        let ts = a["created_at"].as_i64().unwrap_or(0);
                        let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
                            .map(|d| d.format("%m-%d %H:%M").to_string())
                            .unwrap_or_default();
                        self.tui_dim(format!(
                            "    {kind:<12}  {size:>6}B  {dt}  {}",
                            &id[..12.min(id.len())]
                        ));
                    }
                }
            }
        Ok(false)
    }

}
