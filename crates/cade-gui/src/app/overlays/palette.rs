//! Slash-command palette overlay.

use eframe::egui;

use super::super::AppAction;

pub fn render_palette_overlay(
    ctx: &egui::Context,
    palette_input: &str,
    palette_selection: usize,
) -> Option<AppAction> {
    use crate::palette::fuzzy_filter;
    let mut result: Option<AppAction> = None;

    // Compute screen-centered rect for a 520x360 panel.
    let screen = ctx.content_rect();
    let w = 520.0_f32.min(screen.width() - 40.0);
    let h = 360.0_f32.min(screen.height() - 80.0);
    let pos = egui::pos2(
        screen.center().x - w / 2.0,
        screen.top() + 80.0,
    );

    egui::Window::new("Command palette")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
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

            // Header + query input
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("⌘")
                        .color(crate::theme::PRIMARY)
                        .size(16.0),
                );
                let mut q = palette_input.to_string();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut q)
                        .hint_text("Type a command…")
                        .desired_width(ui.available_width()),
                );
                resp.request_focus();
                if resp.changed() {
                    result = Some(AppAction::SetPaletteInput(q));
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            // Filtered entries
            let filtered = fuzzy_filter(palette_input);
            if filtered.is_empty() {
                ui.label(
                    egui::RichText::new("No matching commands")
                        .color(crate::theme::TEXT_MUTED)
                        .italics(),
                );
            } else {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(h - 90.0)
                    .show(ui, |ui| {
                        for (idx, entry) in filtered.iter().enumerate() {
                            let is_sel = idx == palette_selection;
                            let bg = if is_sel {
                                crate::theme::BG_SURFACE2
                            } else {
                                crate::theme::BG_SURFACE0
                            };
                            let frame = egui::Frame::new()
                                .fill(bg)
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(8, 6));
                            let resp = frame
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "/{}",
                                                entry.def.trigger
                                            ))
                                            .color(crate::theme::PRIMARY)
                                            .monospace()
                                            .strong(),
                                        );
                                        if let Some(hint) = entry.def.arg_hint {
                                            ui.label(
                                                egui::RichText::new(hint)
                                                    .color(crate::theme::TEXT_MUTED)
                                                    .monospace()
                                                    .small(),
                                            );
                                        }
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(
                                                    egui::RichText::new(entry.def.description)
                                                        .color(crate::theme::TEXT_MUTED)
                                                        .small(),
                                                );
                                            },
                                        );
                                    });
                                })
                                .response
                                .interact(egui::Sense::click());
                            if resp.clicked() {
                                // Clicking an entry sets the selection
                                // to that index AND executes it.
                                let delta = idx as i32 - palette_selection as i32;
                                if delta != 0 {
                                    result = Some(AppAction::MovePaletteSelection(delta));
                                } else {
                                    result = Some(AppAction::ExecutePaletteCmd);
                                }
                            }
                            ui.add_space(2.0);
                        }
                    });
            }

            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("↑↓ select  ⏎ run  Esc close")
                        .color(crate::theme::TEXT_MUTED)
                        .small(),
                );
            });
        });

    result
}
