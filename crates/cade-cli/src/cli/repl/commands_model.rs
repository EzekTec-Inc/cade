//! /model command handler.

use super::Repl;
use crate::Result;
use crate::ui::ToastLevel;
use cade_core::toolsets::Toolset;
use std::sync::Arc;

impl Repl {
    pub(crate) async fn cmd_model(
        &mut self,
        m: String,
        _stdout: &mut std::io::Stdout,
    ) -> Result<bool> {
        // Empty arg → open interactive picker
        let m = if m.is_empty() {
            match self.interactive_model_picker(Arc::clone(&self.app)).await? {
                Some(picked) => picked,
                None => {
                    let _ = self.app.lock().draw();
                    return Ok(false);
                }
            }
        } else {
            m
        };
        let new_toolset = Toolset::for_model(&m);
        let old_toolset = *self.current_toolset.lock();
        self.tui_dim(format!("  Switching model → {m}…"));
        match self.client.patch_agent_model(&self.agent_id(), &m).await {
            Ok(new_model) => {
                *self.current_model.lock() = new_model.clone();

                // Save model association for current mode
                let active_mode = self.permissions.mode();
                if let Err(e) = self
                    .settings
                    .lock()
                    .set_model_for_mode(active_mode, &new_model)
                {
                    tracing::error!("Failed to save preferred model for mode {active_mode}: {e}");
                }

                if new_toolset != old_toolset {
                    *self.current_toolset.lock() = new_toolset;
                    self.spawn_tool_reregister();
                    self.tui_hdr(format!("  Toolset → {}", new_toolset.display_name()));
                }
                self.tui_ok(format!("  ✓ Model: {new_model}"));
                if let Some(adaptive_theme) = crate::cli::repl::resolve_adaptive_theme(&new_model) {
                    let mut app = self.app.lock();
                    app.apply_theme(adaptive_theme);
                    self.tui_ok("  ✓ Applied adaptive provider-aware theme colors".to_string());
                }
                {
                    let mut app = self.app.lock();
                    app.show_toast(format!("Model → {new_model}"), ToastLevel::Success);
                    let _ = app.draw();
                }
            }
            Err(e) => self.tui_err(e.to_string()),
        }
        Ok(false)
    }
}
