//! Usage statistics overlay.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;
pub fn render_stats_overlay(
    ctx: &egui::Context,
    total_in: u64,
    total_out: u64,
    last_usage: Option<&(u64, u64, Option<String>)>,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;
    let screen = ctx.content_rect();
    let rect = crate::responsive::overlay_rect(ctx, 380.0, 240.0, None);
    let w = rect.width();
    let h = rect.height();
    let pos = rect.min;

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
                .fill(theme.bg_surface1())
                .stroke(egui::Stroke::new(1.0, theme.border_focus()))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(12.0),
        )
        .show(ctx, |ui| {
            ui.set_width(w - 24.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("📊  Token Usage")
                        .color(theme.primary())
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
            ui.label(
                egui::RichText::new("Session totals")
                    .color(theme.text_muted())
                    .strong()
                    .size(12.0),
            );
            egui::Grid::new("stats_totals")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("input tokens")
                            .color(theme.text_muted())
                            .monospace()
                            .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(total_in.to_string())
                            .color(theme.text_primary())
                            .monospace()
                            .size(11.0),
                    );
                    ui.end_row();
                    ui.label(
                        egui::RichText::new("output tokens")
                            .color(theme.text_muted())
                            .monospace()
                            .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(total_out.to_string())
                            .color(theme.text_primary())
                            .monospace()
                            .size(11.0),
                    );
                    ui.end_row();
                    ui.label(
                        egui::RichText::new("total tokens")
                            .color(theme.text_muted())
                            .monospace()
                            .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new((total_in + total_out).to_string())
                            .color(theme.success())
                            .monospace()
                            .strong()
                            .size(11.0),
                    );
                    ui.end_row();
                });

            if let Some((lin, lout, model)) = last_usage {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Last turn")
                        .color(theme.text_muted())
                        .strong()
                        .size(12.0),
                );
                egui::Grid::new("stats_last")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("input")
                                .color(theme.text_muted())
                                .monospace()
                                .size(11.0),
                        );
                        ui.label(
                            egui::RichText::new(lin.to_string())
                                .color(theme.text_primary())
                                .monospace()
                                .size(11.0),
                        );
                        ui.end_row();
                        ui.label(
                            egui::RichText::new("output")
                                .color(theme.text_muted())
                                .monospace()
                                .size(11.0),
                        );
                        ui.label(
                            egui::RichText::new(lout.to_string())
                                .color(theme.text_primary())
                                .monospace()
                                .size(11.0),
                        );
                        ui.end_row();
                        if let Some(m) = model {
                            ui.label(
                                egui::RichText::new("model")
                                    .color(theme.text_muted())
                                    .monospace()
                                    .size(11.0),
                            );
                            ui.label(
                                egui::RichText::new(m)
                                    .color(theme.text_muted())
                                    .monospace()
                                    .size(11.0),
                            );
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
