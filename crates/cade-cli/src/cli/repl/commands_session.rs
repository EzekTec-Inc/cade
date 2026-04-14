//! /session command handler.
use crate::ui::{RenderLine, ToastLevel};
use cade_core::permissions::PermissionMode;
use crate::Result;
use super::Repl;

impl Repl {
    pub(crate) async fn cmd_undo(
        &mut self,
    ) -> Result<bool> {
            let agent_id = self.agent_id();
            match self.client.list_checkpoints(&agent_id).await {
                Err(e) => self.tui_err(format!("  ✗ list_checkpoints: {e}")),
                Ok(checkpoints) if checkpoints.is_empty() => {
                    self.tui_dim("  No checkpoints available to undo.".to_string());
                }
                Ok(checkpoints) => {
                    if let Some(last_cp) = checkpoints.last() {
                        let checkpoint_id =
                            last_cp["id"].as_str().unwrap_or("").to_string();
                        let stash_ref =
                            last_cp["git_stash_ref"].as_str().map(String::from);
                        self.tui_dim(format!(
                            "  Restoring checkpoint {checkpoint_id}…"
                        ));
                        if let Some(s) = stash_ref {
                            use cade_agent::tools::git_checkpoint;
                            match git_checkpoint::restore_git_checkpoint(&s, &self.cwd)
                                .await
                            {
                                Ok(()) => {
                                    self.tui_ok(format!("  ✓ Git stash applied: {s}"))
                                }
                                Err(e) => self.tui_err(format!("  ✗ Git restore: {e}")),
                            }
                        }
                        let _ = self
                            .client
                            .restore_checkpoint(&agent_id, &checkpoint_id)
                            .await;
                        self.tui_ok(format!(
                            "  ✓ Restored to checkpoint {checkpoint_id}"
                        ));
                    }
                }
            }
        Ok(false)
    }

    pub(crate) async fn cmd_rename(
        &mut self,
        new_name: String,
    ) -> Result<bool> {
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

    pub(crate) async fn cmd_export(
        &mut self,
        out_arg: Option<String>,
    ) -> Result<bool> {
            let agent_id = self.agent_id();
            let agent_name = self.agent_name();
            let out_path = out_arg.unwrap_or_else(|| {
                crate::cli::export_import::default_export_path(&agent_name)
            });
            self.tui_dim(format!("  Exporting agent '{agent_name}' → {out_path} …"));
            match crate::cli::export_import::export_agent_to_file(
                &self.client,
                &agent_id,
                &out_path,
            )
            .await
            {
                Ok(_) => {
                    self.app.lock().show_toast(
                        format!("Exported → {out_path}"),
                        ToastLevel::Success,
                    );
                    self.tui_ok(format!("  ✓ Exported → {out_path}"))
                }
                Err(e) => self.tui_err(format!("  ✗ Export failed: {e}")),
            }
        // -- Checkpoints
        Ok(false)
    }

    pub(crate) async fn cmd_mouse(
        &mut self,
    ) -> Result<bool> {
            let mut app = self.app.lock();
            app.toggle_mouse_capture();
            if app.mouse_capture_disabled {
                let _ = app.push(RenderLine::SystemMsg(
                    "Mouse capture disabled — scroll disabled. Click and drag to select text. /mouse to restore.".into()
                ));
            } else {
                let _ = app.push(RenderLine::SuccessMsg(
                    "Mouse capture restored — scroll enabled.".into(),
                ));
            }
        Ok(false)
    }

    pub(crate) async fn cmd_copy(
        &mut self,
    ) -> Result<bool> {
        match self.client.last_assistant_message(&self.agent_id(), self.conversation_id.lock().as_deref()).await {
            Ok(Some(msg)) => {
                // Extract text from the message content array.
                let mut text = String::new();
                if let Some(content_arr) = msg["content"].as_array() {
                    for part in content_arr {
                        if part["type"].as_str() == Some("text") {
                            if let Some(t) = part["text"].as_str() {
                                text.push_str(t);
                                text.push('\n');
                            }
                        }
                    }
                }
                
                let text = text.trim();
                if text.is_empty() {
                    self.app.lock().show_toast("No text found in last assistant message", ToastLevel::Warning);
                    return Ok(false);
                }

                // 1. OSC 52 Universal Fallback
                use base64::Engine;
                let b64 = base64::prelude::BASE64_STANDARD.encode(text);
                print!("\x1b]52;c;{}\x07", b64);
                use std::io::Write;
                let _ = std::io::stdout().flush();

                // 2. Native OS clipboard (if clipboard-images feature is enabled)
                #[cfg(feature = "clipboard-images")]
                {
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        let _ = cb.set_text(text);
                    }
                }

                self.app.lock().show_toast("Copied last message to clipboard", ToastLevel::Success);
            }
            Ok(None) => {
                self.app.lock().show_toast("No assistant message found to copy", ToastLevel::Warning);
            }
            Err(e) => {
                self.app.lock().show_toast(format!("Failed to fetch last message: {e}"), ToastLevel::Error);
            }
        }
        let _ = self.app.lock().draw();
        Ok(false)
    }

    pub(crate) async fn cmd_clear(
        &mut self,
    ) -> Result<bool> {
            let _ = self.app.lock().clear_content();
            match self.client.clear_messages(&self.agent_id()).await {
                Ok(n) => self
                    .tui_ok(format!("✓ Context window cleared ({n} messages deleted)")),
                Err(e) => self
                    .tui_sys(format!("⚠ Screen cleared (context clear failed: {e})")),
            }
        Ok(false)
    }

    pub(crate) async fn cmd_stream(
        &mut self,
    ) -> Result<bool> {
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

    pub(crate) async fn cmd_yolo(
        &mut self,
    ) -> Result<bool> {
            self.permissions.set_mode(PermissionMode::BypassPermissions);
            self.app
                .lock()
                .update_mode(PermissionMode::BypassPermissions);
            let _ =
                self.app
                    .lock()
                    .push(RenderLine::SystemMsg(
                    "⚡ Permission mode: bypassPermissions — all tools auto-approved"
                        .to_string(),
                ));
            self.sync_plan_tools(false).await;
        Ok(false)
    }

}
