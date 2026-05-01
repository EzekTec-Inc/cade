//! Agents overview overlay.

use crate::theme::EguiThemeExt;
use cade_api_types::AgentInfo;
use eframe::egui;

use super::super::AppAction;
pub fn render_agents_overlay(
    ctx: &egui::Context,
    agents: &[AgentInfo],
    selected: Option<usize>,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;
    let screen = ctx.content_rect();
    let rect = crate::responsive::overlay_rect(ctx, 480.0, 400.0, None);
    let w = rect.width();
    let h = rect.height();
    let pos = rect.min;

    let mut open = true;
    egui::Window::new("Agents")
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
                    egui::RichText::new("🤖  Agents")
                        .color(theme.primary())
                        .strong()
                        .size(16.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        result = Some(AppAction::CloseAgentsOverlay);
                    }
                    ui.label(
                        egui::RichText::new("Esc to close")
                            .color(theme.text_dim())
                            .small(),
                    );
                });
            });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(6.0);

            egui::ScrollArea::vertical()
                .id_salt("agents_list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (idx, agent) in agents.iter().enumerate() {
                        let is_sel = selected == Some(idx);
                        let bg = if is_sel {
                            theme.bg_surface2()
                        } else {
                            theme.bg_surface0()
                        };
                        let resp = egui::Frame::new()
                            .fill(bg)
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(&agent.name)
                                            .color(theme.text_primary())
                                            .strong(),
                                    );
                                    if is_sel {
                                        ui.label(
                                            egui::RichText::new("← current")
                                                .color(theme.primary())
                                                .small(),
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if let Some(model) = &agent.model {
                                                ui.label(
                                                    egui::RichText::new(model)
                                                        .color(theme.text_muted())
                                                        .monospace()
                                                        .size(10.0),
                                                );
                                            }
                                        },
                                    );
                                });
                                let short_id = if agent.id.len() > 16 {
                                    format!("{}…", &agent.id[..16])
                                } else {
                                    agent.id.clone()
                                };
                                ui.label(
                                    egui::RichText::new(short_id)
                                        .color(theme.text_dim())
                                        .monospace()
                                        .size(10.0),
                                );
                            })
                            .response
                            .interact(egui::Sense::click());
                        if resp.clicked() && !is_sel {
                            result = Some(AppAction::SelectAgent(idx));
                        }
                        ui.add_space(3.0);
                    }
                });
        });

    if !open && result.is_none() {
        result = Some(AppAction::CloseAgentsOverlay);
    }
    result
}

// ── Context-stats overlay (M19) ───────────────────────────────────────
