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

        Self {
            login: LoginState::new(),
            session: Rc::new(RefCell::new(None)),
            connect_started: false,
            ctx: cc.egui_ctx.clone(),
            server_url: config.server_url,
            md_cache: CommonMarkCache::default(),
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
        let (input, server_url, token, agent_id) = {
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
            (input, server_url, token, agent_id)
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
                move |chunk| {
                    if let Some(s) = session_clone.borrow_mut().as_mut() {
                        s.on_stream_chunk(chunk);
                    }
                    ctx_clone.request_repaint();
                },
            )
            .await;

            // Mark stream as done regardless of success/error.
            if let Some(s) = session.borrow_mut().as_mut() {
                s.on_stream_done();
            }

            if let Err(_e) = result {
                // TODO: surface streaming errors in the UI.
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
                    ..
                }) => {
                    // ── Connected: 3-panel layout ───────────────────
                    let version = health.version.as_deref().unwrap_or("unknown");
                    let has_agent = selected_agent.is_some();
                    let is_streaming = streaming;

                    // Clone input buffer for the editable text field.
                    let mut input_edit = input_buffer.clone();

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
                                        .hint_text(if !has_agent {
                                            "Select an agent first…"
                                        } else if is_streaming {
                                            "Waiting for response…"
                                        } else {
                                            "Send a message…"
                                        })
                                        .desired_width(ui.available_width() - 80.0),
                                );

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
                                        // Role header
                                        let role_text = match msg.role.as_str() {
                                            "user" => "👤 User",
                                            "assistant" => "🤖 Assistant",
                                            "system" => "⚙️ System",
                                            "tool" => "🔧 Tool",
                                            other => other,
                                        };
                                        ui.add_space(8.0);
                                        ui.label(
                                            egui::RichText::new(role_text)
                                                .strong()
                                                .size(13.0),
                                        );
                                        ui.separator();

                                        // Render content as markdown (string) or
                                        // raw JSON (structured).
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
}
