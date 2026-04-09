use crate::Result;
use super::super::Repl;

impl Repl {
    /// `/mcp` interactive picker
    pub(crate) async fn interactive_mcp_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
    ) -> Result<Option<cade_tui::mcp_picker::McpAction>> {
        use cade_tui::mcp_picker::{McpEntry, show_mcp_manager};

        let mcp_configs = self
            .settings
            .lock()
            .global_settings_mut()
            .mcp_servers
            .clone();
        let statuses = self.mcp.status().await;

        let mut entries = Vec::new();
        for (key, config) in mcp_configs {
            let status = statuses.iter().find(|s| s.key == key);
            let tool_count = if config.disabled {
                None
            } else if let Some(s) = status {
                if !s.disabled {
                    Some(s.tools.len())
                } else {
                    None
                }
            } else {
                None
            };
            entries.push(McpEntry {
                key,
                config,
                tool_count,
            });
        }
        entries.sort_by(|a, b| a.key.cmp(&b.key));

        let mut app = app_arc.lock();
        let colors = app.colors.clone();

        let result = show_mcp_manager(&mut app.terminal, entries, &colors);
        // Clear screen when done to force a re-render of underlying timeline
        let _ = app.terminal.clear();
        Ok(result?)
    }
}
