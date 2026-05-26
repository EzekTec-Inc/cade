import sys

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'r') as f:
    mod_rs = f.read()

# Update sidebar call to pass active_page
mod_rs = mod_rs.replace('components::sidebar::render(ui, &self.theme)', 'components::sidebar::render(ui, &mut self.active_page, &self.theme)')

# Update CentralPanel to route based on active_page
old_central = """        // ── Main Content (Central Panel) ─────────────────────────────────────────────
        if let Some(SessionState::Connected(session)) = &session_snapshot_for_toolbar {
            components::overview::render(ui, session, &self.theme);
        } else {
            // Fallback for unconnected state
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label("Connecting...");
                    ui.spinner();
                });
            });
        }"""

new_central = """        // ── Main Content (Central Panel) ─────────────────────────────────────────────
        if let Some(SessionState::Connected(session)) = &session_snapshot_for_toolbar {
            match self.active_page {
                ActivePage::Overview => {
                    components::overview::render(ui, session, &self.theme);
                }
                ActivePage::Chat => {
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label("Chat functionality is temporarily hidden to show the Account Overview layout. Select 'Profile' to return.");
                        });
                    });
                }
                ActivePage::Memory => {
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label("Memory Palace placeholder");
                        });
                    });
                }
                ActivePage::Skills => {
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label("Skills Library placeholder");
                        });
                    });
                }
                ActivePage::Logs => {
                    egui::CentralPanel::default().show_inside(ui, |ui| {
                        ui.centered_and_justified(|ui| {
                            ui.label("Logs placeholder");
                        });
                    });
                }
            }
        } else {
            // Fallback for unconnected state
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label("Connecting...");
                    ui.spinner();
                });
            });
        }"""

mod_rs = mod_rs.replace(old_central, new_central)

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'w') as f:
    f.write(mod_rs)

