use crate::app::{ActivePage, AppAction};
use eframe::egui;

pub fn render(
    ui: &mut egui::Ui,
    active_page: &mut ActivePage,
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
                let (rect, _resp) =
                    ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                ui.painter().rect_stroke(
                    rect,
                    egui::CornerRadius::same(4),
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
                ui.painter()
                    .circle_filled(rect.center(), 3.0, egui::Color32::WHITE);

                ui.add_space(8.0);
                ui.heading(
                    egui::RichText::new("CADE Beta")
                        .color(egui::Color32::WHITE)
                        .strong()
                        .size(18.0),
                );
            });

            ui.add_space(32.0);

            // Menu sections
            let mut render_section = |title: &str, items: &[(&str, ActivePage, &str)]| {
                ui.label(
                    egui::RichText::new(title)
                        .color(egui::Color32::from_gray(120))
                        .size(10.0)
                        .strong(),
                );
                ui.add_space(8.0);

                for (icon, target_page, label) in items {
                    let is_active = *active_page == *target_page;
                    let text_color = if is_active {
                        egui::Color32::from_rgb(230, 80, 80) // #E55050
                    } else {
                        egui::Color32::from_gray(180)
                    };

                    let bg_color = if is_active {
                        egui::Color32::from_rgb(40, 28, 28) // Subtle red tint
                    } else {
                        egui::Color32::TRANSPARENT
                    };

                    let resp = egui::Frame::NONE
                        .fill(bg_color)
                        .corner_radius(egui::CornerRadius::same(4))
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                if is_active {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(2.0, 14.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(
                                        rect,
                                        egui::CornerRadius::same(1),
                                        text_color,
                                    );
                                    ui.add_space(6.0);
                                } else {
                                    ui.add_space(8.0);
                                }
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
                        *active_page = *target_page;
                    }
                    ui.add_space(2.0);
                }
                ui.add_space(24.0);
            };

            render_section(
                "ACCOUNT",
                &[
                    ("👤", ActivePage::Overview, "Profile"),
                    ("💬", ActivePage::Chat, "Chat / Agents"),
                    ("🗂", ActivePage::Memory, "Memory Palace"),
                    ("🛠", ActivePage::Skills, "Skills Library"),
                ],
            );

            render_section(
                "ORGANIZATION",
                &[
                    ("👥", ActivePage::Logs, "Members / Logs"),
                    ("📊", ActivePage::Overview, "Usage"),
                ],
            );

            render_section(
                "REFERENCE",
                &[
                    ("📖", ActivePage::Documentation, "Documentation"),
                    ("🔌", ActivePage::ApiReference, "API Reference"),
                ],
            );
        });

    action
}
