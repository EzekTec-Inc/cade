//! /mcp command handler.

use crate::Result;
use super::Repl;

impl Repl {
    pub(crate) async fn cmd_mcp(
        &mut self,
        input: &str,
        pending_input: &mut Option<String>,
    ) -> Result<bool> {
            if self.require_capability(cade_core::capabilities::Capability::Mcp, "/mcp")
            {
                return Ok(false);
            }
            // Support "/mcp reload" subcommand
            let sub = input.trim().strip_prefix("/mcp").unwrap_or("").trim();
            if sub == "reload" {
                self.do_settings_reload().await;
                return Ok(false);
            }
            match self
                .interactive_mcp_picker(std::sync::Arc::clone(&self.app))
                .await?
            {
                Some(cade_tui::mcp_picker::McpAction::Toggle(key)) => {
                    let mut s = self.settings.lock();
                    if let Some(server) =
                        s.global_settings_mut().mcp_servers.get_mut(&key)
                    {
                        server.disabled = !server.disabled;
                    }
                    let _ = s.save_global();
                    drop(s);
                    self.do_settings_reload().await;
                }
                Some(cade_tui::mcp_picker::McpAction::Delete(key)) => {
                    let mut s = self.settings.lock();
                    s.global_settings_mut().mcp_servers.remove(&key);
                    let _ = s.save_global();
                    drop(s);
                    self.do_settings_reload().await;
                }
                Some(cade_tui::mcp_picker::McpAction::New) => {
                    let tmpl = serde_json::json!({
                        "new_server": {
                            "command": "npx",
                            "args": ["-y", "@modelcontextprotocol/server-everything"],
                            "env": {},
                            "disabled": false
                        }
                    });
                    let mut app = self.app.lock();
                    let text = format!(
                        "/mcp-save\n{}",
                        serde_json::to_string_pretty(&tmpl).unwrap()
                    );
                    app.editor.set_text(text.clone());
                    app.editor.set_cursor_pos(text.len());
                    app.push_silent(crate::ui::RenderLine::SystemMsg(
                        "  Edit the JSON below and hit Enter to create/save."
                            .to_string(),
                    ));
                }
                Some(cade_tui::mcp_picker::McpAction::Edit(key)) => {
                    let config = self
                        .settings
                        .lock()
                        .global_settings_mut()
                        .mcp_servers
                        .get(&key)
                        .cloned()
                        .unwrap_or_default();
                    let tmpl = serde_json::json!({ key: config });
                    let mut app = self.app.lock();
                    let text = format!(
                        "/mcp-save\n{}",
                        serde_json::to_string_pretty(&tmpl).unwrap()
                    );
                    app.editor.set_text(text.clone());
                    app.editor.set_cursor_pos(text.len());
                    app.push_silent(crate::ui::RenderLine::SystemMsg(
                        "  Edit the JSON below and hit Enter to save.".to_string(),
                    ));
                }
                None => {
                    self.tui_dim("  /mcp closed");
                }
            }
        Ok(false)
    }
}
