//! Login screen — thin egui view over `crate::login::LoginState`.
//!
//! This module is wasm32-only.  All real logic lives in `login.rs` which is
//! fully native-testable.  The render code here contains no branching logic
//! other than matching the state-machine variants to UI strings; all state
//! transitions are delegated to `LoginState::on_input` / `on_submit`.

use eframe::egui;

use crate::login::LoginState;

/// Single-screen eframe app for the dashboard's login view.
pub struct CadeApp {
    login: LoginState,
}

impl CadeApp {
    /// Construct from the `CreationContext` handed to us by eframe.
    /// Kept simple: no persistence, no theme customization at this milestone.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            login: LoginState::new(),
        }
    }
}

impl eframe::App for CadeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.heading("CADE Dashboard");
            ui.add_space(12.0);
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
                    let submit = ui.button("Connect");
                    let enter = resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if submit.clicked() || enter {
                        self.login.on_submit();
                    }
                }
                LoginState::Submitted { key: _ } => {
                    ui.label("Connected — session UI coming soon.");
                }
            }
        });
    }
}
