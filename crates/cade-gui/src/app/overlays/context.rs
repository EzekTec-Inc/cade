//! Context-stats overlay.

use eframe::egui;

use super::super::AppAction;
pub fn render_context_overlay(
    ctx: &egui::Context,
    stats: Option<&crate::api::ContextStats>,
    loading: bool,
    error: Option<&str>,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;
    let screen = ctx.content_rect();
    let w = 520.0_f32.min(screen.width() - 40.0);
    let h = 420.0_f32.min(screen.height() - 80.0);
    let pos = egui::pos2(screen.center().x - w / 2.0, screen.center().y - h / 2.0);

    let mut open = true;
    egui::Window::new("Context")
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
                    egui::RichText::new("📐  Context Window")
                        .color(crate::theme::PRIMARY)
                        .strong()
                        .size(16.0),
                );
                if loading { ui.spinner(); }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        result = Some(AppAction::CloseContextOverlay);
                    }
                });
            });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(6.0);

            if let Some(err) = error {
                ui.label(egui::RichText::new(format!("⚠ {err}")).color(crate::theme::ERROR).small());
                return;
            }

            let Some(s) = stats else {
                ui.label(egui::RichText::new("Loading…").color(crate::theme::TEXT_MUTED).italics());
                return;
            };

            let rows: &[(&str, String)] = &[
                ("model",            s.model.as_deref().unwrap_or("—").to_string()),
                ("window_tokens",    s.window_tokens.to_string()),
                ("turns_included",   format!("{} / {}", s.turns_included, s.turns_total)),
                ("turns_omitted",    s.turns_omitted.to_string()),
                ("chars_used",       format!("{} / {} budget", s.chars_used, s.message_budget_chars)),
                ("memory_chars",     s.memory_chars.to_string()),
                ("system_prompt",    format!("{} chars", s.system_prompt_chars)),
                ("tool_count",       s.tool_count.to_string()),
                ("tool_schema_rsv",  format!("{} chars", s.tool_schema_reserve_chars)),
                ("needs_consolidation", s.needs_consolidation.to_string()),
            ];

            egui::Grid::new("ctx_grid")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    for (k, v) in rows {
                        ui.label(egui::RichText::new(*k).color(crate::theme::TEXT_MUTED).monospace().size(11.0));
                        ui.label(egui::RichText::new(v).color(crate::theme::TEXT_PRIMARY).monospace().size(11.0));
                        ui.end_row();
                    }
                });

            // Simple fill bar for chars_used / message_budget_chars
            if s.message_budget_chars > 0 {
                ui.add_space(8.0);
                let pct = (s.chars_used as f32 / s.message_budget_chars as f32).min(1.0);
                let bar_w = ui.available_width();
                let (resp, painter) = ui.allocate_painter(egui::vec2(bar_w, 10.0), egui::Sense::hover());
                let r = resp.rect;
                painter.rect_filled(r, 3.0, crate::theme::BG_SURFACE0);
                painter.rect_filled(
                    egui::Rect::from_min_size(r.min, egui::vec2(r.width() * pct, r.height())),
                    3.0,
                    if pct > 0.85 { crate::theme::WARNING } else { crate::theme::PRIMARY },
                );
                ui.label(egui::RichText::new(format!("{:.0}% context used", pct * 100.0)).color(crate::theme::TEXT_DIM).size(10.0));
            }
        });

    if !open && result.is_none() {
        result = Some(AppAction::CloseContextOverlay);
    }
    result
}

// ── Stats overlay (M19) ───────────────────────────────────────────────

