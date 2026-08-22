use super::super::Repl;
use crate::Result;

impl Repl {
    /// `/mcp` interactive picker
    pub(crate) async fn interactive_mcp_picker(
        &self,
        app_arc: std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
        initial_selected: usize,
    ) -> Result<(Option<cade_tui::mcp_picker::McpAction>, usize)> {
        use cade_tui::mcp_picker::{McpEntry, show_mcp_manager};

        let mcp_configs = self.settings.lock().all_mcp_servers();
        let statuses = match self.client.get_mcp_statuses().await {
            Ok(s) => s,
            Err(_) => self.mcp.status().await,
        };

        let mut entries = Vec::new();
        for (key, config) in mcp_configs {
            let status = statuses.iter().find(|s| s.key == key);
            let (tool_count, tools) = if config.disabled {
                (None, Vec::new())
            } else if let Some(s) = status {
                if !s.disabled {
                    (Some(s.tools.len()), s.tools.clone())
                } else {
                    (None, Vec::new())
                }
            } else {
                (None, Vec::new())
            };
            entries.push(McpEntry {
                key,
                config,
                tool_count,
                tools,
            });
        }
        entries.sort_by(|a, b| a.key.cmp(&b.key));

        let mut app = app_arc.lock();
        let colors = app.colors.clone();

        let result = show_mcp_manager(&mut app.terminal, entries, &colors, initial_selected);
        // Clear screen when done to force a re-render of underlying timeline
        let _ = app.terminal.clear();
        let (action, final_idx) = result?;
        Ok((action, final_idx))
    }
}
