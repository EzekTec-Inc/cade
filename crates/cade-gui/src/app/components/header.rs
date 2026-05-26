use crate::app::ActivePage;
use eframe::egui;

pub fn render(
    ui: &mut egui::Ui,
    _active_page: &mut ActivePage,
    _session: &Option<crate::session::SessionState>,
    _theme: &crate::theme::ThemeColors,
) -> Option<crate::app::AppAction> {
    let action = None;
    
    // Custom frame for top bar to give it a specific background and bottom border
    let frame = egui::Frame::NONE
        .fill(egui::Color32::from_rgb(23, 23, 23))
        .inner_margin(egui::Margin::symmetric(16, 12))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(40)));

    egui::Panel::top("dashboard_header").frame(frame).show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            // Left side
            ui.label(
                egui::RichText::new("Stephen Ezekwem's organization")
                    .color(egui::Color32::from_gray(200))
                    .size(14.0)
            );
            
            // "Free" badge
            egui::Frame::NONE
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(100)))
                .corner_radius(egui::CornerRadius::same(2))
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("Free")
                            .color(egui::Color32::from_gray(180))
                            .size(11.0)
                    );
                });

            // Right side
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Avatar placeholder
                let (rect, _resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, egui::CornerRadius::same(12), egui::Color32::from_rgb(40, 120, 80));

                ui.add_space(16.0);

                // Free Plan button
                let btn = egui::Button::new(
                    egui::RichText::new("Free Plan").color(egui::Color32::WHITE)
                )
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(100)));
                
                if ui.add(btn).clicked() { }

                ui.add_space(16.0);

                // Links
                let link_color = egui::Color32::from_gray(180);
                if ui.add(egui::Label::new(egui::RichText::new("Manage LLM keys").color(link_color).size(13.0)).sense(egui::Sense::click())).clicked() { }
                ui.add_space(12.0);
                if ui.add(egui::Label::new(egui::RichText::new("API reference").color(link_color).size(13.0)).sense(egui::Sense::click())).clicked() { }
                ui.add_space(12.0);
                if ui.add(egui::Label::new(egui::RichText::new("Docs").color(link_color).size(13.0)).sense(egui::Sense::click())).clicked() { }
                ui.add_space(12.0);
                if ui.add(egui::Label::new(egui::RichText::new("Support").color(link_color).size(13.0)).sense(egui::Sense::click())).clicked() { }
            });
        });
    });
    
    action
}
