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
                    ..
                }) => {
                    // ── Connected: 3-panel layout ───────────────────
                    // Skip vertical_centered for connected state — we
                    // need the full area for panels.  The panels use
                    // `show_inside` to nest within the parent `Ui`.
                    let version = health.version.as_deref().unwrap_or("unknown");

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
                                for agent in agents {
                                    let label = format!("🤖 {}", agent.name);
                                    let _ = ui.selectable_label(false, label);
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
                                ui.add_enabled(
                                    false,
                                    egui::TextEdit::singleline(
                                        &mut String::new(),
                                    )
                                    .hint_text("Send a message… (coming soon)")
                                    .desired_width(ui.available_width() - 60.0),
                                );
                            });
                        });

                    // ── Central area: timeline with markdown ─────
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            // Placeholder: render a sample markdown to prove
                            // the pipeline works.  Real content comes from
                            // SSE stream frames in a future milestone.
                            let sample_md = "\
## Welcome to CADE Dashboard

Connected and ready.  Select an agent from the sidebar to begin.

### What you can do

- **Chat** with any configured agent
- View the *streaming* response in real time
- Inspect tool calls and their results

```rust
fn main() {
    println!(\"Hello from CADE!\");
}
```

> This timeline will show the conversation once you pick an agent.
";
                            CommonMarkViewer::new()
                                .show(ui, &mut self.md_cache, sample_md);
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
        }
    }
}

/// Deferred actions collected during the render pass and applied
/// after all borrows are released.
enum AppAction {
    None,
    Connect(String),
    Retry,
}
