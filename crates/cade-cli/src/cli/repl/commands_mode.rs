//! /mode command handler.

use super::{Repl, mode_display};
use crate::Result;
use crate::ui::ToastLevel;
use cade_core::permissions::PermissionMode;

impl Repl {
    pub(crate) async fn cmd_mode(&mut self, arg: Option<String>) -> Result<bool> {
        use crate::cli::repl::format::parse_mode_label;
        match arg.as_deref() {
            None | Some("") => {
                let (icon, label, hint) = mode_display(self.permissions.mode());
                self.tui_sys(format!("{icon} Current mode: {label}  {hint}"));
            }
            Some(name) => {
                let resolved = parse_mode_label(name);
                match resolved {
                    Some("default") => {
                        self.permissions.set_mode(PermissionMode::Default);
                        let (icon, label, _) = mode_display(PermissionMode::Default);
                        self.app
                            .lock()
                            .show_toast(format!("{icon} {label}"), ToastLevel::Success);
                        self.tui_ok(format!("{icon} Permission mode: {label}"));
                        self.sync_plan_tools(false).await;
                        let _ = self
                            .auto_switch_model_for_mode(PermissionMode::Default)
                            .await;
                    }
                    Some("plan") => {
                        self.permissions.set_mode(PermissionMode::Plan);
                        let (icon, label, hint) = mode_display(PermissionMode::Plan);
                        self.app
                            .lock()
                            .show_toast(format!("{icon} {label}"), ToastLevel::Info);
                        self.tui_hdr(format!("{icon} Permission mode: {label} {hint}"));
                        self.sync_plan_tools(true).await;
                        let _ = self.auto_switch_model_for_mode(PermissionMode::Plan).await;
                    }
                    Some("yolo") => {
                        self.permissions.set_mode(PermissionMode::BypassPermissions);
                        let (icon, label, _) = mode_display(PermissionMode::BypassPermissions);
                        self.app
                            .lock()
                            .show_toast(format!("{icon} {label}"), ToastLevel::Warning);
                        self.tui_sys(format!("{icon} Permission mode: {label}"));
                        self.sync_plan_tools(false).await;
                        let _ = self
                            .auto_switch_model_for_mode(PermissionMode::BypassPermissions)
                            .await;
                    }
                    Some("acceptEdits") => {
                        self.permissions.set_mode(PermissionMode::AcceptEdits);
                        let (icon, label, _) = mode_display(PermissionMode::AcceptEdits);
                        self.app
                            .lock()
                            .show_toast(format!("{icon} {label}"), ToastLevel::Success);
                        self.tui_ok(format!("{icon} Permission mode: {label}"));
                        self.sync_plan_tools(false).await;
                        let _ = self
                            .auto_switch_model_for_mode(PermissionMode::AcceptEdits)
                            .await;
                    }
                    _ => {
                        self.tui_err(format!(
                                "Unknown mode '{name}'. Valid: safe | edit-freely | plan | full-access (or: default | acceptEdits | yolo)"
                            ));
                    }
                }
            }
        }
        // SlashCmd::New is handled below (hot-swap)
        Ok(false)
    }

    pub(crate) async fn auto_switch_model_for_mode(&mut self, mode: PermissionMode) -> Result<()> {
        let preferred_model = self.settings.lock().model_for_mode(mode);

        if let Some(preferred) = preferred_model {
            let current = self.current_model.lock().clone();
            if preferred != current {
                self.tui_dim(format!(
                    "  🔄 Auto-switching model to {preferred} for {mode} mode…"
                ));
                let new_toolset = cade_core::toolsets::Toolset::for_model(&preferred);
                let old_toolset = *self.current_toolset.lock();

                match self
                    .client
                    .patch_agent_model(&self.agent_id(), &preferred)
                    .await
                {
                    Ok(new_model) => {
                        *self.current_model.lock() = new_model.clone();
                        if new_toolset != old_toolset {
                            *self.current_toolset.lock() = new_toolset;
                            self.spawn_tool_reregister();
                            self.tui_hdr(format!("  Toolset → {}", new_toolset.display_name()));
                        }
                        self.tui_ok(format!("  ✓ Model: {new_model}"));
                        {
                            let mut app = self.app.lock();
                            app.show_toast(
                                format!("Auto-switched model → {new_model}"),
                                ToastLevel::Success,
                            );
                            let _ = app.draw();
                        }
                    }
                    Err(e) => {
                        self.tui_err(format!("Failed to auto-switch model: {e}"));
                    }
                }
            }
        }
        Ok(())
    }
}
