//! /artifacts command handler.

use super::Repl;
use crate::Result;

impl Repl {
    pub(crate) async fn cmd_link(
        &mut self,
        arg: Option<String>,
        _pending_input: &mut Option<String>,
    ) -> Result<bool> {
        let mut active = self.active_mcp_servers.lock();
        if let Some(ref server_name) = arg {
            let server_name = server_name.trim();
            if server_name == "all" {
                active.insert("all".to_string());
                self.tui_ok("  ✓ Linking ALL MCP servers...");
            } else {
                active.insert(server_name.to_string());
                self.tui_ok(format!("  ✓ Linking MCP server: {server_name}..."));
            }
        } else {
            active.insert("all".to_string());
            self.tui_ok("  ✓ Linking ALL MCP servers (default)...");
        }
        drop(active);

        self.spawn_tool_reregister();
        self.tui_ok("  ✓ Relink scheduled. Tools will be available shortly.".to_string());
        Ok(false)
    }

    pub(crate) async fn cmd_unlink(&mut self, arg: Option<String>) -> Result<bool> {
        let agent_id = self.agent_id();
        let mut active = self.active_mcp_servers.lock();
        if let Some(ref server_name) = arg {
            let server_name = server_name.trim();
            if server_name == "all" {
                active.clear();
                drop(active);
                self.tui_ok("  ✓ Unlinking ALL MCP servers...");
                self.spawn_tool_reregister();
            } else {
                active.remove(server_name);
                drop(active);
                self.tui_ok(format!("  ✓ Unlinked MCP server: {server_name}"));
                self.spawn_tool_reregister();
            }
        } else {
            active.clear();
            drop(active);
            match self.client.detach_agent_tools(&agent_id).await {
                Ok(n) => self.tui_ok(format!("  ✓ Detached {n} tool(s) from agent (Zero-Tool mode)")),
                Err(e) => self.tui_err(e.to_string()),
            }
        }
        Ok(false)
    }

    pub(crate) async fn cmd_artifacts(&mut self) -> Result<bool> {
        if self.require_capability(cade_core::capabilities::Capability::Agentic, "/artifacts") {
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
