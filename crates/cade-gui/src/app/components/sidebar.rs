use crate::app::AppAction;
use eframe::egui;

pub fn render(
    ui: &mut egui::Ui,
    _theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let action = None;

    let frame = egui::Frame::NONE
        .fill(egui::Color32::from_rgb(23, 23, 23)) // #171717
        .inner_margin(egui::Margin::same(16))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(40)));

    egui::Panel::left("dashboard_sidebar")
        .frame(frame)
        .exact_size(240.0)
        .resizable(false)
        .show_inside(ui, |ui| {
            // Logo area
            ui.horizontal(|ui| {
                // Mocking the Letta Beta logo icon
                let (rect, _resp) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                ui.painter().rect_stroke(rect, egui::CornerRadius::same(4), egui::Stroke::new(2.0, egui::Color32::WHITE), egui::StrokeKind::Inside);
                ui.painter().circle_filled(rect.center(), 3.0, egui::Color32::WHITE);
                
                ui.add_space(8.0);
                ui.heading(
                    egui::RichText::new("CADE Beta")
                        .color(egui::Color32::WHITE)
                        .strong()
                        .size(18.0)
                );
            });
            
            ui.add_space(32.0);

            // Menu sections
            let mut render_section = |title: &str, items: &[(&str, bool, &str)]| {
                ui.label(
                    egui::RichText::new(title)
                        .color(egui::Color32::from_gray(120))
                        .size(10.0)
                        .strong()
                );
                ui.add_space(8.0);
                
                for (icon, is_active, label) in items {
                    let text_color = if *is_active {
                        egui::Color32::from_rgb(230, 80, 80) // #E55050
                    } else {
                        egui::Color32::from_gray(180)
                    };

                    let bg_color = if *is_active {
                        egui::Color32::from_rgb(40, 28, 28) // Subtle red tint
                    } else {
                        egui::Color32::TRANSPARENT
                    };

                    egui::Frame::NONE
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
                        });
                    ui.add_space(2.0);
                }
                ui.add_space(24.0);
            };

            render_section("ACCOUNT", &[
                ("👤", true, "Profile"),
                ("🗂", false, "Projects"),
            ]);

            render_section("ORGANIZATION", &[
                ("👥", false, "Members"),
                ("📊", false, "Usage"),
                ("📝", false, "API logs"),
                ("🔗", false, "Integrations"),
                ("💳", false, "Billing"),
                ("⚙", false, "Settings"),
            ]);

            render_section("REFERENCE", &[
                ("🔒", false, "Rate limits"),
            ]);
        });

    action
}
