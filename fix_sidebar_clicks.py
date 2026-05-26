import sys

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/components/sidebar.rs', 'r') as f:
    sidebar = f.read()

sidebar = sidebar.replace("""                    egui::Frame::NONE
                        .fill(bg_color)
                        .corner_radius(egui::CornerRadius::same(4))
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(*icon).color(text_color).size(14.0));
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(*label).color(text_color).size(13.0));
                            });
                        });""", """                    let resp = egui::Frame::NONE
                        .fill(bg_color)
                        .corner_radius(egui::CornerRadius::same(4))
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(*icon).color(text_color).size(14.0));
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(*label).color(text_color).size(13.0));
                            });
                        })
                        .response
                        .interact(egui::Sense::click());

                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked() {
                        // Dummy action or real action
                    }""")

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/components/sidebar.rs', 'w') as f:
    f.write(sidebar)

