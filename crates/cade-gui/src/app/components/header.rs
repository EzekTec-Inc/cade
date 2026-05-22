use eframe::egui;
use crate::app::ActivePage;
use crate::theme::EguiThemeExt;

pub fn render(
    ui: &mut egui::Ui,
    active_page: &mut ActivePage,
    session: &Option<crate::session::SessionState>,
    theme: &crate::theme::ThemeColors,
) -> Option<crate::app::AppAction> {
    let mut action = None;
    egui::Panel::top("dashboard_header").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("Serena Dashboard").color(theme.text_primary()));
            ui.add_space(20.0);
            
            ui.selectable_value(active_page, ActivePage::Overview, "Overview");
            ui.selectable_value(active_page, ActivePage::Chat, "Chat");
            ui.selectable_value(active_page, ActivePage::Memory, "Memory Palace");
            ui.selectable_value(active_page, ActivePage::Skills, "Skills Library");
            ui.selectable_value(active_page, ActivePage::Logs, "Logs");
            
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Menu").clicked() {
                    action = Some(crate::app::AppAction::OpenMenu(String::new()));
                }
                
                ui.add_space(10.0);

                if let Some(crate::session::SessionState::Connected(sess)) = session {
                    let model_label = sess.context_stats.as_ref()
                        .and_then(|c| c.model.clone())
                        .unwrap_or_else(|| "Auto-detect".into());
                    
                    let btn = ui.button(format!("Model: {}", model_label));
                    if btn.clicked() {
                        action = Some(crate::app::AppAction::OpenPalette(String::from("/model ")));
                    }
                    
                    // Show a quick environment/provider indicator if desired
                    let agent_name = sess.agents.get(sess.selected_agent.unwrap_or(0))
                        .map(|a| a.name.as_str())
                        .unwrap_or("No Agent");
                    
                    if ui.button(format!("Env: {}", agent_name)).clicked() {
                        action = Some(crate::app::AppAction::ToggleProfilesOverlay);
                    }
                }
            });
        });
    });
    action
}
