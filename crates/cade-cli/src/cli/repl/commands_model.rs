//! /model command handler.

use crate::Result;
use super::Repl;
use std::sync::Arc;
use cade_core::toolsets::Toolset;
use crate::ui::ToastLevel;

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
                    *self.current_model.lock() =
                        new_model.clone();
                    if new_toolset != old_toolset {
                        *self.current_toolset.lock() =
                            new_toolset;
                        self.spawn_tool_reregister();
                        self.tui_hdr(format!(
                            "  Toolset → {}",
                            new_toolset.display_name()
                        ));
                    }
                    self.tui_ok(format!("  ✓ Model: {new_model}"));
                    {
                        let mut app = self.app.lock();
                        app.show_toast(
                            format!("Model → {new_model}"),
                            ToastLevel::Success,
                        );
                        let _ = app.draw();
                    }
                }
                Err(e) => self.tui_err(e.to_string()),
            }
        Ok(false)
    }

}
