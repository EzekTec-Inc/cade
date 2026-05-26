import sys

# 1. Fix mod.rs to include sidebar
with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'r') as f:
    mod_rs = f.read()

sidebar_call = """
        // ── Sidebar (Left) ─────────────────────────────────────────────
        if let Some(new_action) = components::sidebar::render(ui, &mut self.active_page, &self.theme) {
            action = new_action;
        }

        // ── Top toolbar (M1) ─────────────────────────────────────────────"""

mod_rs = mod_rs.replace('        // ── Top toolbar (M1) ─────────────────────────────────────────────', sidebar_call)

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/mod.rs', 'w') as f:
    f.write(mod_rs)

# 2. Fix sidebar.rs (CADE Betta, Red Bar)
with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/components/sidebar.rs', 'r') as f:
    sidebar = f.read()

sidebar = sidebar.replace('"CADE Beta"', '"CADE Betta"')

old_horizontal = """                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(*icon).color(text_color).size(14.0));
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(*label).color(text_color).size(13.0));
                            });"""

new_horizontal = """                            ui.horizontal(|ui| {
                                if is_active {
                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(2.0, 14.0), egui::Sense::hover());
                                    ui.painter().rect_filled(rect, egui::CornerRadius::same(1), text_color);
                                    ui.add_space(6.0);
                                } else {
                                    ui.add_space(8.0);
                                }
                                ui.label(egui::RichText::new(*icon).color(text_color).size(14.0));
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(*label).color(text_color).size(13.0));
                            });"""

sidebar = sidebar.replace(old_horizontal, new_horizontal)

with open('/home/engr-uba/Downloads/02 Rust-project/CADE/crates/cade-gui/src/app/components/sidebar.rs', 'w') as f:
    f.write(sidebar)

