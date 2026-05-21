use eframe::egui;
use crate::app::ActivePage;

pub fn render(
    ui: &mut egui::Ui,
    active_page: &mut ActivePage,
    theme: &crate::theme::ThemeColors,
) {
    egui::TopBottomPanel::top("dashboard_header").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("Serena Dashboard").color(theme.text_primary()));
            ui.add_space(20.0);
            
            ui.selectable_value(active_page, ActivePage::Overview, "Overview");
            ui.selectable_value(active_page, ActivePage::Chat, "Chat");
            ui.selectable_value(active_page, ActivePage::Logs, "Logs");
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Menu").clicked() {
                    // Menu logic here
                }
            });
        });
    });
}
