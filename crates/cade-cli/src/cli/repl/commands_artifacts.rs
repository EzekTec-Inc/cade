//! /artifacts command handler.

use super::Repl;
use crate::Result;

impl Repl {
    pub(crate) async fn cmd_link(&mut self, _pending_input: &mut Option<String>) -> Result<bool> {
        self.tui_dim("  Linking tools…");
        self.spawn_tool_reregister();
        self.tui_ok("  ✓ Relink scheduled. Tools will be available shortly.".to_string());
        Ok(false)
    }

    pub(crate) async fn cmd_unlink(&mut self) -> Result<bool> {
        let agent_id = self.agent_id();
        match self.client.detach_agent_tools(&agent_id).await {
            Ok(n) => self.tui_ok(format!("  ✓ Detached {n} tool(s) from agent")),
            Err(e) => self.tui_err(e.to_string()),
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
