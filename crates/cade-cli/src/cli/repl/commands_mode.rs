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
                    }
                    Some("plan") => {
                        self.permissions.set_mode(PermissionMode::Plan);
                        let (icon, label, hint) = mode_display(PermissionMode::Plan);
                        self.app
                            .lock()
                            .show_toast(format!("{icon} {label}"), ToastLevel::Info);
                        self.tui_hdr(format!("{icon} Permission mode: {label} {hint}"));
                        self.sync_plan_tools(true).await;
                    }
                    Some("yolo") => {
                        self.permissions.set_mode(PermissionMode::BypassPermissions);
                        let (icon, label, _) = mode_display(PermissionMode::BypassPermissions);
                        self.app
                            .lock()
                            .show_toast(format!("{icon} {label}"), ToastLevel::Warning);
                        self.tui_sys(format!("{icon} Permission mode: {label}"));
                        self.sync_plan_tools(false).await;
                    }
                    Some("acceptEdits") => {
                        self.permissions.set_mode(PermissionMode::AcceptEdits);
                        let (icon, label, _) = mode_display(PermissionMode::AcceptEdits);
                        self.app
                            .lock()
                            .show_toast(format!("{icon} {label}"), ToastLevel::Success);
                        self.tui_ok(format!("{icon} Permission mode: {label}"));
                        self.sync_plan_tools(false).await;
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
}
