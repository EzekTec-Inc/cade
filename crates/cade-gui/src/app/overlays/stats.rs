//! Usage statistics overlay.

use eframe::egui;

use super::super::AppAction;
pub fn render_stats_overlay(
    ctx: &egui::Context,
    total_in: u64,
    total_out: u64,
    last_usage: Option<&(u64, u64, Option<String>)>,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;
    let screen = ctx.content_rect();
    let w = 380.0_f32.min(screen.width() - 40.0);
    let h = 240.0_f32.min(screen.height() - 80.0);
    let pos = egui::pos2(screen.center().x - w / 2.0, screen.center().y - h / 2.0);

    let mut open = true;
    egui::Window::new("Stats")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .open(&mut open)
        .fixed_pos(pos)
        .fixed_size([w, h])
        .frame(
            egui::Frame::new()
                .fill(crate::theme::BG_SURFACE1)
                .stroke(egui::Stroke::new(1.0, crate::theme::BORDER_FOCUS))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(12.0),
        )
        .show(ctx, |ui| {
            ui.set_width(w - 24.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("📊  Token Usage")
                        .color(crate::theme::PRIMARY)
                        .strong()
                        .size(16.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        result = Some(AppAction::CloseStatsOverlay);
                    }
                });
            });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(6.0);

            // Session totals
            ui.label(egui::RichText::new("Session totals").color(crate::theme::TEXT_MUTED).strong().size(12.0));
            egui::Grid::new("stats_totals").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                ui.label(egui::RichText::new("input tokens").color(crate::theme::TEXT_MUTED).monospace().size(11.0));
                ui.label(egui::RichText::new(total_in.to_string()).color(crate::theme::TEXT_PRIMARY).monospace().size(11.0));
                ui.end_row();
                ui.label(egui::RichText::new("output tokens").color(crate::theme::TEXT_MUTED).monospace().size(11.0));
                ui.label(egui::RichText::new(total_out.to_string()).color(crate::theme::TEXT_PRIMARY).monospace().size(11.0));
                ui.end_row();
                ui.label(egui::RichText::new("total tokens").color(crate::theme::TEXT_MUTED).monospace().size(11.0));
                ui.label(egui::RichText::new((total_in + total_out).to_string()).color(crate::theme::SUCCESS).monospace().strong().size(11.0));
                ui.end_row();
            });

            if let Some((lin, lout, model)) = last_usage {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Last turn").color(crate::theme::TEXT_MUTED).strong().size(12.0));
                egui::Grid::new("stats_last").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                    ui.label(egui::RichText::new("input").color(crate::theme::TEXT_MUTED).monospace().size(11.0));
                    ui.label(egui::RichText::new(lin.to_string()).color(crate::theme::TEXT_PRIMARY).monospace().size(11.0));
                    ui.end_row();
                    ui.label(egui::RichText::new("output").color(crate::theme::TEXT_MUTED).monospace().size(11.0));
                    ui.label(egui::RichText::new(lout.to_string()).color(crate::theme::TEXT_PRIMARY).monospace().size(11.0));
                    ui.end_row();
                    if let Some(m) = model {
                        ui.label(egui::RichText::new("model").color(crate::theme::TEXT_MUTED).monospace().size(11.0));
                        ui.label(egui::RichText::new(m).color(crate::theme::TEXT_MUTED).monospace().size(11.0));
                        ui.end_row();
                    }
                });
            }
        });

    if !open && result.is_none() {
        result = Some(AppAction::CloseStatsOverlay);
    }
    result
}
