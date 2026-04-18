//! Agents overview overlay.

use eframe::egui;
use cade_api_types::AgentInfo;

use super::super::AppAction;
pub fn render_agents_overlay(
    ctx: &egui::Context,
    agents: &[AgentInfo],
    selected: Option<usize>,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;
    let screen = ctx.content_rect();
    let w = 480.0_f32.min(screen.width() - 40.0);
    let h = 400.0_f32.min(screen.height() - 80.0);
    let pos = egui::pos2(screen.center().x - w / 2.0, screen.center().y - h / 2.0);

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
                .fill(crate::theme::BG_SURFACE1)
                .stroke(egui::Stroke::new(1.0, crate::theme::BORDER_FOCUS))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(12.0),
        )
        .show(ctx, |ui| {
            ui.set_width(w - 24.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("🤖  Agents")
                        .color(crate::theme::PRIMARY)
                        .strong()
                        .size(16.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        result = Some(AppAction::CloseAgentsOverlay);
                    }
                    ui.label(
                        egui::RichText::new("Esc to close")
                            .color(crate::theme::TEXT_DIM)
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
                        let bg = if is_sel { crate::theme::BG_SURFACE2 } else { crate::theme::BG_SURFACE0 };
                        let resp = egui::Frame::new()
                            .fill(bg)
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(&agent.name).color(crate::theme::TEXT_PRIMARY).strong());
                                    if is_sel {
                                        ui.label(egui::RichText::new("← current").color(crate::theme::PRIMARY).small());
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if let Some(model) = &agent.model {
                                            ui.label(egui::RichText::new(model).color(crate::theme::TEXT_MUTED).monospace().size(10.0));
                                        }
                                    });
                                });
                                let short_id = if agent.id.len() > 16 { format!("{}…", &agent.id[..16]) } else { agent.id.clone() };
                                ui.label(egui::RichText::new(short_id).color(crate::theme::TEXT_DIM).monospace().size(10.0));
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

