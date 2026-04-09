//! /help command handler.
use crate::ui::RenderLine;
use crate::Result;
use super::Repl;

impl Repl {
    pub(crate) async fn cmd_help(
        &mut self,
        pending_input: &mut Option<String>,
    ) -> Result<bool> {
            // Open full-screen command browser (filtered by capabilities)
            let chosen = {
                let mut app = self.app.lock();
                let colors = app.colors.clone();
                crate::ui::menu::show_command_menu_with_caps(
                    &mut app.terminal,
                    &colors,
                    Some(&self.capabilities),
                )?
            };
            let _ = self.app.lock().draw();
            if let Some(cmd) = chosen {
                // If it's a tool hint (no slash) or a command that needs arguments,
                // insert it into the editor instead of executing immediately.
                let needs_args = !cmd.starts_with('/')
                    || (cmd.contains(' ')
                        && !["/stats model", "/skills reload"].contains(&cmd.as_str()))
                    || [
                        "/delete",
                        "/checkpoint",
                        "/fork",
                        "/approve-always",
                        "/deny-always",
                        "/remember",
                        "/disconnect",
                        "/search",
                        "/export",
                        "/rename",
                        "/connect",
                    ]
                    .contains(&cmd.as_str());
                if needs_args {
                    let mut app = self.app.lock();
                    app.editor.insert_str(&format!("{cmd} "));
                } else {
                    *pending_input = Some(cmd);
                }
            }
            return Ok(false);
    }

    pub(crate) async fn cmd_info(
        &mut self,
    ) -> Result<bool> {
            let msg = format!(
                "  Agent   : {} ({})\n  Conv    : {}\n  Model   : {}\n  Mode    : {}\n  CWD     : {}\n  Version : {}",
                self.agent_name(),
                self.agent_id(),
                self.conversation_id().as_deref().unwrap_or("default"),
                self.model(),
                self.permissions.mode(),
                self.cwd.display(),
                env!("CARGO_PKG_VERSION")
            );
            let _ = self
                .app
                .lock()
                .push(RenderLine::SystemMsg(msg));
        Ok(false)
    }

    pub(crate) async fn cmd_usage(
        &mut self,
        pending_input: &mut Option<String>,
    ) -> Result<bool> {
            use std::sync::atomic::Ordering;
            let in_tok = self.session_input_tokens.load(Ordering::SeqCst);
            let out_tok = self.session_output_tokens.load(Ordering::SeqCst);
            let total = in_tok + out_tok;
            self.tui_blank();
            self.tui_hdr("  Token usage this session:");
            self.tui_dim(format!("    Input  : {:>8}", in_tok));
            self.tui_dim(format!("    Output : {:>8}", out_tok));
            self.tui_dim(format!("    Total  : {:>8}", total));
            if total == 0 {
                self.tui_dim("    (no usage recorded yet — requires Anthropic/OpenAI)");
            }
        Ok(false)
    }

    pub(crate) async fn cmd_stats(
        &mut self,
        arg: Option<String>,
    ) -> Result<bool> {
            let sub = arg.as_deref().unwrap_or("").trim();
            let lines = match sub {
                "model" | "models" => self
                    .session_stats
                    .lock()
                    .render_model_detail(),
                _ => {
                    // full session card (default)
                    let auth_method = if self.settings.lock().api_key().is_some() {
                        "API Key".to_string()
                    } else {
                        "OAuth / Browser".to_string()
                    };
                    let session_id = self.conversation_id().unwrap_or_default();
                    self.session_stats
                        .lock()
                        .render_card(&auth_method, &session_id)
                }
            };
            self.tui_blank();
            for line in lines {
                let _ = self.app.lock().push(line);
            }
            self.tui_blank();
        Ok(false)
    }

    pub(crate) async fn cmd_debug_last(
        &mut self,
        pending_input: &mut Option<String>,
    ) -> Result<bool> {
            let conv = self.conversation_id();
            match self
                .client
                .last_assistant_message(&self.agent_id(), conv.as_deref())
                .await
            {
                Ok(Some(msg)) => {
                    self.tui_hdr("  Raw last assistant message");
                    if let Ok(raw) = serde_json::to_string_pretty(&msg) {
                        for line in raw.lines() {
                            self.tui_dim(format!("    {line}"));
                        }
                    } else {
                        self.tui_dim(format!("    {msg}"));
                    }
                    self.tui_blank();
                }
                Ok(None) => self.tui_dim("  ⎿  No assistant replies stored yet."),
                Err(e) => {
                    self.tui_err(format!("Failed to load last assistant message: {e}"))
                }
            }
        Ok(false)
    }

}
