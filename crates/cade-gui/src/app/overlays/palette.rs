//! Slash-command palette overlay.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;

pub fn render_palette_overlay(
    ctx: &egui::Context,
    palette_input: &str,
    palette_selection: usize,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    use crate::palette::fuzzy_filter;
    let mut result: Option<AppAction> = None;

    // TUI-fication: full-width bar anchored at top, not a floating centered modal.
    let screen = ctx.content_rect();
    let w = screen.width();
    let max_rows = 12;
    let filtered = fuzzy_filter(palette_input);
    let row_count = filtered.len().min(max_rows);
    // Height: input row (~28) + separator + rows (~24 each) + hint row
    let h = (36.0 + (row_count as f32 * 26.0) + 24.0).min(screen.height() - 20.0);
    let pos = egui::pos2(screen.left(), screen.top());

    egui::Window::new("Command palette")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .fixed_pos(pos)
        .fixed_size([w, h])
        .frame(
            egui::Frame::new()
                .fill(theme.bg_surface0())
                .stroke(egui::Stroke::new(1.0, theme.border_base()))
                .corner_radius(egui::CornerRadius::ZERO)
                .inner_margin(egui::Margin::symmetric(8, 4)),
        )
        .show(ctx, |ui| {
            ui.set_width(w - 16.0);

            // Header + query input — compact, single line
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(">")
                        .color(theme.primary())
                        .monospace()
                        .strong()
                        .size(14.0),
                );
                let mut q = palette_input.to_string();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut q)
                        .hint_text("Type a command…")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                );
                resp.request_focus();
                if resp.changed() {
                    result = Some(AppAction::SetPaletteInput(q));
                }
            });

            ui.add_space(2.0);
            // Thin separator line
            let sep_rect = ui.available_rect_before_wrap();
            let sep_rect = egui::Rect::from_min_size(
                sep_rect.min,
                egui::vec2(sep_rect.width(), 1.0),
            );
            ui.painter().rect_filled(sep_rect, 0.0, theme.border_base());
            ui.advance_cursor_after_rect(sep_rect);
            ui.add_space(2.0);

            // Filtered entries — dense, monospace, no card frames
            if filtered.is_empty() {
                ui.label(
                    egui::RichText::new("  No matching commands")
                        .color(theme.text_muted())
                        .monospace()
                        .italics()
                        .size(12.0),
                );
            } else {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(h - 64.0)
                    .show(ui, |ui| {
                        for (idx, entry) in filtered.iter().take(max_rows).enumerate() {
                            let is_sel = idx == palette_selection;
                            let bg = if is_sel {
                                theme.bg_surface2()
                            } else {
                                egui::Color32::TRANSPARENT
                            };

                            let resp = ui.horizontal(|ui| {
                                // Selection highlight — full-width background
                                let row_rect = ui.available_rect_before_wrap();
                                let row_rect = egui::Rect::from_min_size(
                                    row_rect.min,
                                    egui::vec2(row_rect.width(), 22.0),
                                );
                                if is_sel {
                                    ui.painter().rect_filled(row_rect, 0.0, bg);
                                }

                                // Trigger
                                ui.label(
                                    egui::RichText::new(format!("/{}", entry.def.trigger))
                                        .color(theme.primary())
                                        .monospace()
                                        .strong()
                                        .size(12.0),
                                );
                                // Arg hint
                                if let Some(hint) = entry.def.arg_hint {
                                    ui.label(
                                        egui::RichText::new(hint)
                                            .color(theme.text_dim())
                                            .monospace()
                                            .size(11.0),
                                    );
                                }
                                // Description — pushed right
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(entry.def.description)
                                                .color(theme.text_muted())
                                                .size(11.0),
                                        );
                                    },
                                );
                            }).response.interact(egui::Sense::click());

                            if resp.clicked() {
                                let delta = idx as i32 - palette_selection as i32;
                                if delta != 0 {
                                    result = Some(AppAction::MovePaletteSelection(delta));
                                } else {
                                    result = Some(AppAction::ExecutePaletteCmd);
                                }
                            }
                        }
                    });
            }

            ui.add_space(2.0);
            // Thin bottom separator
            let sep_rect = ui.available_rect_before_wrap();
            let sep_rect = egui::Rect::from_min_size(
                sep_rect.min,
                egui::vec2(sep_rect.width(), 1.0),
            );
            ui.painter().rect_filled(sep_rect, 0.0, theme.border_base());
            ui.advance_cursor_after_rect(sep_rect);

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("↑↓ select  ⏎ run  Esc close")
                        .color(theme.text_dim())
                        .monospace()
                        .size(10.0),
                );
            });
        });

    result
}
