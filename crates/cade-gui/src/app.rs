//! Dashboard app — thin egui view over the login + session state machines.
//!
//! This module is wasm32-only.  All real logic lives in `login.rs` and
//! `session.rs` which are fully native-testable.  The render code here
//! contains no branching logic other than matching the state-machine
//! variants to UI strings; all state transitions are delegated to the
//! pure modules.
//!
//! After the user submits their token, a `spawn_local` async task calls
//! `http_wasm::{get_health, get_agents}` and feeds the results into the
//! shared `SessionState` via `on_health` / `on_agents` / `on_error`.
//! The `egui::Context` is cloned into the task so it can call
//! `request_repaint()` to wake the render loop.

use std::cell::RefCell;
use std::rc::Rc;

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use crate::config::Config;
use crate::login::LoginState;
use crate::session::SessionState;
use crate::shortcuts::{ShortcutAction, poll_shortcut};

/// Top-level eframe app for the cade-gui dashboard.
pub struct CadeApp {
    /// Login form state — driven by user input events.
    login: LoginState,
    /// Post-login session state — driven by async HTTP results.
    /// `None` means we haven't started connecting yet.
    session: Rc<RefCell<Option<SessionState>>>,
    /// Guard: true once we've spawned the connection task for the
    /// current `Submitted` token.  Reset on retry.
    connect_started: bool,
    /// Saved egui context for repaint requests from async tasks.
    ctx: egui::Context,
    /// Server URL resolved at boot (from page origin).
    server_url: String,
    /// Shared cache for egui_commonmark — avoids re-parsing markdown
    /// on every frame.
    md_cache: CommonMarkCache,
    /// Stable ID for the chat input field — used by Ctrl+L to request focus.
    input_id: egui::Id,
}

impl CadeApp {
    /// Construct from the `CreationContext` handed to us by eframe.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Apply the CADE dark theme once at startup.
        crate::theme::apply_theme(&cc.egui_ctx);

        // Resolve the server URL from the page origin.  In production
        // the dashboard is served by cade-server, so origin == API host.
        let origin = web_sys::window()
            .and_then(|w| w.location().origin().ok())
            .unwrap_or_else(|| "http://localhost:8284".to_string());
        let query = web_sys::window().and_then(|w| w.location().search().ok());
        let config = Config::resolve(&origin, query.as_deref(), None);

        let mut login = LoginState::new();

        // If a token was previously saved, pre-fill and auto-submit so the
        // first render frame triggers spawn_connect via the existing flow.
        if let Some(saved_token) = crate::storage::load(crate::storage::StorageKey::ApiToken) {
            if !saved_token.is_empty() {
                login.on_input(&saved_token);
                login.on_submit();
            }
        }

        Self {
            login,
            session: Rc::new(RefCell::new(None)),
            connect_started: false,
            ctx: cc.egui_ctx.clone(),
            server_url: config.server_url,
            md_cache: CommonMarkCache::default(),
            input_id: egui::Id::new("chat_input"),
        }
    }

    /// Spawn the async connection task that calls get_health → get_agents
    /// and feeds results into the shared SessionState.
    fn spawn_connect(&mut self, token: &str) {
        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();
        let server_url = self.server_url.clone();
        let token = token.to_string();

        // Transition to Connecting immediately.
        *session.borrow_mut() = Some(SessionState::start(&server_url, &token));
        self.connect_started = true;
        ctx.request_repaint();

        wasm_bindgen_futures::spawn_local(async move {
            // Step 1: health check
            match crate::http_wasm::get_health(&server_url, &token).await {
                Ok(health) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_health(health);
                    }
                    ctx.request_repaint();
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_error(e.to_string());
                    }
                    ctx.request_repaint();
                    return;
                }
            }

            // Step 2: agent list
            match crate::http_wasm::get_agents(&server_url, &token).await {
                Ok(agents) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_agents(agents);
                    }
                    // Persist credentials so the next page load skips login.
                    crate::storage::save(crate::storage::StorageKey::ApiToken, &token);
                    crate::storage::save(crate::storage::StorageKey::ServerUrl, &server_url);
                    ctx.request_repaint();
                }
                Err(e) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_error(e.to_string());
                    }
                    ctx.request_repaint();
                }
            }
        });
    }

    /// Reset to login screen (called when the "Retry" button is clicked).
    fn retry(&mut self) {
        self.login = LoginState::new();
        *self.session.borrow_mut() = None;
        self.connect_started = false;
    }

    /// Clear saved credentials and return to the login screen.
    fn logout(&mut self) {
        crate::storage::clear_all();
        self.retry();
    }

    /// Select an agent and spawn an async task to fetch its messages.
    fn spawn_fetch_messages(&mut self, idx: usize) {
        // Extract what we need while holding the borrow briefly.
        let (changed, server_url, token, agent_id) = {
            let mut session = self.session.borrow_mut();
            let s = match session.as_mut() {
                Some(s) => s,
                None => return,
            };
            let changed = s.on_select_agent(idx);
            if !changed {
                return;
            }
            let server_url = s.server_url().to_string();
            let token = s.token().to_string();
            let agent_id = s.selected_agent_id().unwrap().to_string();
            (changed, server_url, token, agent_id)
        };

        if !changed {
            return;
        }

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            match crate::http_wasm::get_messages(&server_url, &token, &agent_id).await {
                Ok(msgs) => {
                    if let Some(s) = session.borrow_mut().as_mut() {
                        s.on_messages(msgs);
                    }
                }
                Err(_e) => {
                    // Silently ignore message-fetch errors for now —
                    // the timeline just stays empty.  A future milestone
                    // can surface a toast or inline error.
                }
            }
            ctx.request_repaint();
        });
    }

    /// Call `on_send` on the session state, then spawn an async SSE stream
    /// that feeds chunks back into the session.
    fn spawn_stream_message(&mut self) {
        // on_send returns the trimmed input if the send is valid.
        let (input, server_url, token, agent_id, conv_id) = {
            let mut session = self.session.borrow_mut();
            let s = match session.as_mut() {
                Some(s) => s,
                None => return,
            };
            let input = match s.on_send() {
                Some(i) => i,
                None => return,
            };
            let server_url = s.server_url().to_string();
            let token = s.token().to_string();
            let agent_id = match s.selected_agent_id() {
                Some(id) => id.to_string(),
                None => return,
            };
            let conv_id = s.conversation_id().map(String::from);
            (input, server_url, token, agent_id, conv_id)
        };

        let session = Rc::clone(&self.session);
        let ctx = self.ctx.clone();

        wasm_bindgen_futures::spawn_local(async move {
            let session_clone = Rc::clone(&session);
            let ctx_clone = ctx.clone();

            let result = crate::http_wasm::send_message_stream(
                &server_url,
                &token,
                &agent_id,
                &input,
                conv_id.as_deref(),
                move |evt| {
                    use crate::api::StreamEvent;
                    if let Some(s) = session_clone.borrow_mut().as_mut() {
                        match evt {
                            StreamEvent::ConversationId(cid) => s.on_conversation_id(&cid),
                            StreamEvent::Text(text) => s.on_stream_chunk(&text),
                            StreamEvent::Reasoning(text) => s.on_stream_reasoning(&text),
                            StreamEvent::ToolCall { id, name, arguments } => {
                                s.on_stream_tool_call(&id, &name, &arguments);
                            }
                            StreamEvent::Usage { input_tokens, output_tokens, model } => {
                                s.on_usage(input_tokens, output_tokens, model.as_deref());
                            }
                            StreamEvent::FinishReason(reason) => {
                                s.on_finish_reason(&reason);
                            }
                        }
                    }
                    ctx_clone.request_repaint();
                },
            )
            .await;

            // Mark stream as done and surface any error.
            if let Some(s) = session.borrow_mut().as_mut() {
                match result {
                    Ok(()) => s.on_stream_done(),
                    Err(e) => s.push_error(&format!("{e}")),
                }
            }

            ctx.request_repaint();
        });
    }
}

impl eframe::App for CadeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Collect actions during rendering so we can apply them after
        // all borrows are released.  This avoids borrow-conflict issues
        // with Rc<RefCell<..>>.
        let mut action = AppAction::None;

        // ── Global keyboard shortcuts ────────────────────────────
        let shortcut = ui.input(poll_shortcut);
        let mut request_focus_input = false;

        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading("CADE Dashboard");
            ui.add_space(12.0);

            // Snapshot session state for this frame's render pass.
            let session_snapshot = self.session.borrow().clone();

            match session_snapshot {
                Some(SessionState::Connecting { .. }) => {
                    ui.label("Connecting to server...");
                    ui.spinner();
                }
                Some(SessionState::HealthOk { .. }) => {
                    ui.label("Server reached — loading agents...");
                    ui.spinner();
                }
                Some(SessionState::Connected {
                    ref agents,
                    ref health,
                    ref selected_agent,
                    ref messages,
                    ref input_buffer,
                    streaming,
                    ref error_toast,
                    ref last_usage,
                    ref last_finish_reason,
                    ..
                }) => {
                    // ── Connected: 3-panel layout ───────────────────
                    let version = health.version.as_deref().unwrap_or("unknown");
                    let has_agent = selected_agent.is_some();
                    let is_streaming = streaming;

                    // Clone input buffer for the editable text field.
                    let mut input_edit = input_buffer.clone();

                    // ── Map keyboard shortcuts to actions ─────────
                    if let Some(sc) = shortcut {
                        match sc {
                            ShortcutAction::Send => {
                                if has_agent && !is_streaming && !input_edit.trim().is_empty() {
                                    action = AppAction::SendMessage;
                                }
                            }
                            ShortcutAction::InsertNewline => {
                                // Handled inside TextEdit (multiline future); no-op for singleline.
                            }
                            ShortcutAction::DismissError => {
                                if error_toast.is_some() {
                                    action = AppAction::DismissError;
                                }
                            }
                            ShortcutAction::FocusInput => {
                                request_focus_input = true;
                            }
                        }
                    }

                    // ── Left sidebar: agent list ────────────────────
                    egui::Panel::left("agent_sidebar")
                        .default_size(180.0)
                        .resizable(true)
                        .show_inside(ui, |ui| {
                            ui.heading("Agents");
                            ui.separator();
                            if agents.is_empty() {
                                ui.label("No agents configured.");
                            } else {
                                for (i, agent) in agents.iter().enumerate() {
                                    let is_selected = *selected_agent == Some(i);
                                    let label = format!("🤖 {}", agent.name);
                                    if ui.selectable_label(is_selected, label).clicked()
                                        && !is_selected
                                    {
                                        action = AppAction::SelectAgent(i);
                                    }
                                }
                            }
                            ui.separator();
                            ui.add_space(4.0);
                            if ui.button("🚪 Logout").clicked() {
                                action = AppAction::Logout;
                            }
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(format!("v{version}"))
                                    .small()
                                    .weak(),
                            );
                        });

                    // ── Bottom panel: input bar ─────────────────────
                    egui::Panel::bottom("input_bar")
                        .min_size(40.0)
                        .show_inside(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("▸");
                                let can_edit = has_agent && !is_streaming;
                                let resp = ui.add_enabled(
                                    can_edit,
                                    egui::TextEdit::singleline(&mut input_edit)
                                        .id(self.input_id)
                                        .hint_text(if !has_agent {
                                            "Select an agent first…"
                                        } else if is_streaming {
                                            "Waiting for response…"
                                        } else {
                                            "Send a message…  (Ctrl+L to focus)"
                                        })
                                        .desired_width(ui.available_width() - 80.0),
                                );

                                // Focus the input when Ctrl+L was pressed.
                                if request_focus_input {
                                    resp.request_focus();
                                }

                                // Sync edits back into session state.
                                if resp.changed() {
                                    if let Some(SessionState::Connected {
                                        input_buffer: buf, ..
                                    }) = self.session.borrow_mut().as_mut()
                                    {
                                        *buf = input_edit.clone();
                                    }
                                }

                                let enter_pressed = resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));

                                let send_enabled = can_edit && !input_edit.trim().is_empty();
                                let send_clicked =
                                    ui.add_enabled(send_enabled, egui::Button::new("Send"))
                                        .clicked();

                                if (send_clicked || enter_pressed) && send_enabled {
                                    action = AppAction::SendMessage;
                                }

                                if is_streaming {
                                    ui.spinner();
                                }
                            });
                        });

                    // ── Central area: timeline ──────────────────────
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                if selected_agent.is_none() {
                                    let welcome = "\
## Welcome to CADE Dashboard

Connected and ready.  Select an agent from the sidebar to begin.

- **Chat** with any configured agent
- View the *streaming* response in real time
- Inspect tool calls and their results

> This timeline will show the conversation once you pick an agent.
";
                                    CommonMarkViewer::new()
                                        .show(ui, &mut self.md_cache, welcome);
                                } else if messages.is_empty() && !is_streaming {
                                    ui.label("No messages yet. Send one to start a conversation.");
                                } else {
                                    for msg in messages {
                                        ui.add_space(8.0);
                                        match msg.role.as_str() {
                                            "reasoning" => {
                                                // Collapsible "Thinking" block with
                                                // dimmed purple styling.
                                                let text = match &msg.content {
                                                    serde_json::Value::String(s) => s.as_str(),
                                                    _ => "",
                                                };
                                                egui::CollapsingHeader::new(
                                                    egui::RichText::new("💭 Thinking…")
                                                        .color(crate::theme::PURPLE)
                                                        .italics()
                                                        .size(12.0),
                                                )
                                                .default_open(false)
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        egui::RichText::new(text)
                                                            .color(crate::theme::TEXT_MUTED)
                                                            .size(12.0),
                                                    );
                                                });
                                            }
                                            "tool_call" => {
                                                // Styled tool-call card with name +
                                                // collapsible arguments.
                                                let name = msg.content.get("name")
                                                    .and_then(|n| n.as_str())
                                                    .unwrap_or("unknown");
                                                let args = msg.content.get("arguments")
                                                    .and_then(|a| a.as_str())
                                                    .unwrap_or("{}");

                                                // Pretty-print JSON args if possible.
                                                let args_pretty = serde_json::from_str::<serde_json::Value>(args)
                                                    .ok()
                                                    .and_then(|v| serde_json::to_string_pretty(&v).ok())
                                                    .unwrap_or_else(|| args.to_string());

                                                egui::Frame::new()
                                                    .fill(crate::theme::BG_SURFACE1)
                                                    .stroke(egui::Stroke::new(1.0, crate::theme::TEAL.gamma_multiply(0.4)))
                                                    .corner_radius(egui::CornerRadius::same(4))
                                                    .inner_margin(8.0)
                                                    .show(ui, |ui| {
                                                        ui.label(
                                                            egui::RichText::new(format!("🔧 {name}"))
                                                                .color(crate::theme::TEAL)
                                                                .strong()
                                                                .size(12.0),
                                                        );
                                                        egui::CollapsingHeader::new(
                                                            egui::RichText::new("Arguments")
                                                                .color(crate::theme::TEXT_DIM)
                                                                .size(11.0),
                                                        )
                                                        .default_open(false)
                                                        .show(ui, |ui| {
                                                            ui.label(
                                                                egui::RichText::new(&args_pretty)
                                                                    .color(crate::theme::TEXT_MUTED)
                                                                    .monospace()
                                                                    .size(11.0),
                                                            );
                                                        });
                                                    });
                                            }
                                            role => {
                                                // Standard message: user, assistant,
                                                // system, tool, etc.
                                                let (icon, color) = match role {
                                                    "user" => ("👤 User", crate::theme::ROLE_USER),
                                                    "assistant" => ("🤖 Assistant", crate::theme::ROLE_ASSISTANT),
                                                    "system" => ("⚙️ System", crate::theme::ROLE_SYSTEM),
                                                    "tool" => ("🔧 Tool", crate::theme::ROLE_TOOL),
                                                    _ => (role, crate::theme::TEXT_MUTED),
                                                };
                                                ui.label(
                                                    egui::RichText::new(icon)
                                                        .color(color)
                                                        .strong()
                                                        .size(13.0),
                                                );
                                                ui.separator();

                                                let content_str = match &msg.content {
                                                    serde_json::Value::String(s) => s.clone(),
                                                    other => other.to_string(),
                                                };
                                                CommonMarkViewer::new().show(
                                                    ui,
                                                    &mut self.md_cache,
                                                    &content_str,
                                                );
                                            }
                                        }
                                    }

                                    if is_streaming {
                                        ui.add_space(4.0);
                                        ui.horizontal(|ui| {
                                            ui.spinner();
                                            ui.label(
                                                egui::RichText::new("Streaming…")
                                                    .weak()
                                                    .italics(),
                                            );
                                        });
                                    }
                                }
                            });

                        // ── Usage stats footer ─────────────────────
                        if let Some((inp, out, model)) = last_usage {
                            ui.add_space(2.0);
                            let model_str = model
                                .as_ref()
                                .map(|m| format!(" · {m}"))
                                .unwrap_or_default();
                            let finish = last_finish_reason
                                .as_ref()
                                .map(|r| format!(" · {r}"))
                                .unwrap_or_default();
                            ui.label(
                                egui::RichText::new(format!(
                                    "↑{inp} ↓{out} tokens{model_str}{finish}"
                                ))
                                .color(crate::theme::TEXT_DIM)
                                .size(11.0),
                            );
                        }

                        // ── Error toast overlay ────────────────────
                        if let Some(err) = error_toast {
                            ui.add_space(4.0);
                            egui::Frame::new()
                                .fill(crate::theme::ERROR.gamma_multiply(0.15))
                                .stroke(egui::Stroke::new(1.0, crate::theme::ERROR))
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("⚠ Error:")
                                                .color(crate::theme::ERROR)
                                                .strong(),
                                        );
                                        ui.label(
                                            egui::RichText::new(err.as_str())
                                                .color(crate::theme::TEXT_PRIMARY),
                                        );
                                        if ui.small_button("✕").clicked() {
                                            action = AppAction::DismissError;
                                        }
                                    });
                                });
                        }
                    });
                }
                Some(SessionState::ConnectionFailed { ref error, .. }) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 50, 50),
                        "Connection failed",
                    );
                    ui.add_space(4.0);
                    ui.label(error.as_str());
                    ui.add_space(8.0);
                    if ui.button("Retry").clicked() {
                        action = AppAction::Retry;
                    }
                }
                None => {
                    // Still in login flow.
                    match &self.login {
                        LoginState::Entering { buffer } => {
                            ui.label("Paste your CADE API key:");
                            let mut editable = buffer.clone();
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut editable)
                                    .password(true)
                                    .desired_width(320.0)
                                    .hint_text("CADE_API_KEY"),
                            );
                            if resp.changed() {
                                self.login.on_input(&editable);
                            }
                            ui.add_space(8.0);
                            let submit_btn = ui.button("Connect");
                            let enter = resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if submit_btn.clicked() || enter {
                                self.login.on_submit();
                            }
                        }
                        LoginState::Submitted { key } => {
                            if !self.connect_started {
                                action = AppAction::Connect(key.clone());
                            }
                            ui.label("Initiating connection...");
                            ui.spinner();
                        }
                    }
                }
            }
        });

        // Apply deferred actions outside the ui closure.
        match action {
            AppAction::None => {}
            AppAction::Connect(token) => self.spawn_connect(&token),
            AppAction::Retry => self.retry(),
            AppAction::SelectAgent(idx) => self.spawn_fetch_messages(idx),
            AppAction::SendMessage => self.spawn_stream_message(),
            AppAction::DismissError => {
                if let Some(s) = self.session.borrow_mut().as_mut() {
                    s.dismiss_error();
                }
            }
            AppAction::Logout => self.logout(),
        }
    }
}

/// Deferred actions collected during the render pass and applied
/// after all borrows are released.
enum AppAction {
    None,
    Connect(String),
    Retry,
    /// User clicked an agent in the sidebar — spawn a message fetch.
    SelectAgent(usize),
    /// User submitted a message — spawn the SSE stream.
    SendMessage,
    /// User dismissed the error toast.
    DismissError,
    /// User clicked Logout — clear credentials and return to login.
    Logout,
}
