//! /mcp command handler.

use super::Repl;
use crate::Result;

impl Repl {
    pub(crate) async fn cmd_mcp(
        &mut self,
        input: &str,
        _pending_input: &mut Option<String>,
    ) -> Result<bool> {
        if self.require_capability(cade_core::capabilities::Capability::Mcp, "/mcp") {
            return Ok(false);
        }
        // Support "/mcp reload" subcommand
        let sub = input.trim().strip_prefix("/mcp").unwrap_or("").trim();
        if sub == "reload" {
            self.do_settings_reload().await;
            return Ok(false);
        }
        let mut selected_idx = 0;
        loop {
            let (action, final_idx) = self
                .interactive_mcp_picker(std::sync::Arc::clone(&self.app), selected_idx)
                .await?;
            selected_idx = final_idx;

            match action {
                Some(cade_tui::mcp_picker::McpAction::Toggle(key)) => {
                    let mut s = self.settings.lock();
                    if let Some(server) = s.local_settings_mut().mcp_servers.get_mut(&key) {
                        server.disabled = !server.disabled;
                        let _ = s.save_local();
                    } else if let Some(server) = s.project_settings_mut().mcp_servers.get_mut(&key)
                    {
                        server.disabled = !server.disabled;
                        let _ = s.save_project();
                    } else if let Some(server) = s.global_settings_mut().mcp_servers.get_mut(&key) {
                        server.disabled = !server.disabled;
                        let _ = s.save_global();
                    }
                    drop(s);
                    self.do_settings_reload().await;
                }
                Some(cade_tui::mcp_picker::McpAction::Delete(key)) => {
                    let mut s = self.settings.lock();
                    if s.local_settings_mut().mcp_servers.remove(&key).is_some() {
                        let _ = s.save_local();
                    } else if s.project_settings_mut().mcp_servers.remove(&key).is_some() {
                        let _ = s.save_project();
                    } else if s.global_settings_mut().mcp_servers.remove(&key).is_some() {
                        let _ = s.save_global();
                    }
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
                        "  Edit the JSON below and hit Enter to create/save.".to_string(),
                    ));
                    break;
                }
                Some(cade_tui::mcp_picker::McpAction::Edit(key)) => {
                    let s = self.settings.lock();
                    let config = s
                        .all_mcp_servers()
                        .get(&key)
                        .cloned()
                        .unwrap_or_default();
                    drop(s);
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
                    break;
                }
                None => {
                    self.tui_dim("  /mcp closed");
                    break;
                }
            }
        }
        Ok(false)
    }
}
