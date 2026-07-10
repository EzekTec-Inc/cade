//! /session command handler.
use super::Repl;
use crate::Result;
use crate::ui::{RenderLine, ToastLevel};
use cade_core::permissions::PermissionMode;

impl Repl {
    pub(crate) async fn cmd_undo(&mut self) -> Result<bool> {
        let agent_id = self.agent_id();
        match self.client.list_checkpoints(&agent_id).await {
            Err(e) => self.tui_err(format!("  ✗ list_checkpoints: {e}")),
            Ok(checkpoints) if checkpoints.is_empty() => {
                self.tui_dim("  No checkpoints available to undo.".to_string());
            }
            Ok(checkpoints) => {
                if let Some(last_cp) = checkpoints.last() {
                    let checkpoint_id = last_cp["id"].as_str().unwrap_or("").to_string();
                    let commit_hash = last_cp["git_commit_hash"].as_str().map(String::from);
                    self.tui_dim(format!("  Restoring checkpoint {checkpoint_id}…"));
                    if let Some(c) = commit_hash {
                        use cade_agent::tools::git_checkpoint;
                        match git_checkpoint::restore_git_checkpoint(&c, &self.cwd).await {
                            Ok(()) => self.tui_ok(format!("  ✓ Git reset applied: {c}")),
                            Err(e) => self.tui_err(format!("  ✗ Git restore: {e}")),
                        }
                    }
                    let _ = self
                        .client
                        .restore_checkpoint(&agent_id, &checkpoint_id)
                        .await;
                    self.tui_ok(format!("  ✓ Restored to checkpoint {checkpoint_id}"));
                }
            }
        }
        Ok(false)
    }

    pub(crate) async fn cmd_rename(&mut self, new_name: String) -> Result<bool> {
        let id = self.agent_id();
        let new_name = new_name.trim().to_string();
        let name = if new_name.is_empty() {
            // Prompt for name via QuestionWidget
            use crate::ui::question::{Question, QuestionOption};
            let opts = vec![QuestionOption {
                label: "Cancel".to_string(),
                description: String::new(),
            }];
            let q = Question {
                header: "Rename agent".to_string(),
                text: "Enter new agent name:".to_string(),
                options: opts.clone(),
                multi_select: false,
                allow_other: true,
                progress: None,
            };
            let ans = {
                let mut app = self.app.lock();
                app.ask_question(&q)?
            };
            match &ans {
                Some(a) if a.as_str() != "Cancel" && !a.as_str().is_empty() => {
                    a.as_str().to_string()
                }
                _ => String::new(),
            }
        } else {
            new_name
        };
        if name.is_empty() {
            self.tui_dim("  (cancelled)");
        } else {
            match self.client.rename_agent(&id, &name).await {
                Ok(_) => {
                    *self.agent_name.lock() = name.clone();
                    self.tui_ok(format!("  ✓ Renamed to: {name}"));
                }
                Err(e) => self.tui_err(e.to_string()),
            }
        }
        Ok(false)
    }

    pub(crate) async fn cmd_export(&mut self, out_arg: Option<String>) -> Result<bool> {
        let agent_id = self.agent_id();
        let agent_name = self.agent_name();
        let out_path =
            out_arg.unwrap_or_else(|| crate::cli::export_import::default_export_path(&agent_name));
        self.tui_dim(format!("  Exporting agent '{agent_name}' → {out_path} …"));
        match crate::cli::export_import::export_agent_to_file(&self.client, &agent_id, &out_path)
            .await
        {
            Ok(_) => {
                self.app
                    .lock()
                    .show_toast(format!("Exported → {out_path}"), ToastLevel::Success);
                self.tui_ok(format!("  ✓ Exported → {out_path}"))
            }
            Err(e) => self.tui_err(format!("  ✗ Export failed: {e}")),
        }
        // -- Checkpoints
        Ok(false)
    }

    pub(crate) async fn cmd_clear(&mut self) -> Result<bool> {
        let _ = self.app.lock().clear_content();
        match self.client.clear_messages(&self.agent_id()).await {
            Ok(n) => self.tui_ok(format!("✓ Context window cleared ({n} messages deleted)")),
            Err(e) => self.tui_sys(format!("⚠ Screen cleared (context clear failed: {e})")),
        }
        Ok(false)
    }

    pub(crate) async fn cmd_stream(&mut self) -> Result<bool> {
        use std::sync::atomic::Ordering;
        let current = self.streaming_enabled.load(Ordering::SeqCst);
        self.streaming_enabled.store(!current, Ordering::SeqCst);
        let label = if !current { "on" } else { "off" };
        self.tui_hdr(format!("  Streaming: {label}"));
        self.app
            .lock()
            .show_toast(format!("Streaming {label}"), ToastLevel::Info);
        Ok(false)
    }

    pub(crate) async fn cmd_reload(&mut self) -> Result<bool> {
        self.tui_dim("  Reloading UI plugins...");
        let mut app = self.app.lock();
        if let Some(lua) = app.lua_engine.take() {
            drop(lua);
        }
        let new_engine = cade_tui::lua_engine::LuaEngine::new().ok();
        if let Some(engine) = &new_engine {
            if let Some(home) = dirs::home_dir() {
                engine.load_plugins(&home.join(".cade").join("plugins"));
            }
            if let Ok(cwd) = std::env::current_dir() {
                engine.load_plugins(&cwd.join(".cade").join("plugins"));
            }
        }
        app.lua_engine = new_engine;
        app.show_toast("UI Plugins reloaded", ToastLevel::Success);
        Ok(false)
    }

    pub(crate) async fn cmd_yolo(&mut self) -> Result<bool> {
        self.permissions.set_mode(PermissionMode::BypassPermissions);
        self.app
            .lock()
            .update_mode(PermissionMode::BypassPermissions);
        let _ = self.app.lock().push(RenderLine::SystemMsg(
            "⚡ Permission mode: bypassPermissions — all tools auto-approved".to_string(),
        ));
        self.sync_plan_tools(false).await;
        let _ = self.auto_switch_model_for_mode(PermissionMode::BypassPermissions).await;
        Ok(false)
    }
}
