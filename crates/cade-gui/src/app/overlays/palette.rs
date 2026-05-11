//! Slash-command palette overlay — modernised, TUI-aligned.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;

/// Category display label + order.
fn category_label(cat: cade_core::resources::palette::CmdCategory) -> &'static str {
    use cade_core::resources::palette::CmdCategory;
    match cat {
        CmdCategory::Navigation => "Navigation",
        CmdCategory::Session => "Session & Config",
        CmdCategory::Memory => "Memory",
        CmdCategory::Tools => "Tools",
        CmdCategory::Display => "Display",
    }
}

/// Category ordering for grouped display.
fn category_order(cat: cade_core::resources::palette::CmdCategory) -> u8 {
    use cade_core::resources::palette::CmdCategory;
    match cat {
        CmdCategory::Navigation => 0,
        CmdCategory::Session => 1,
        CmdCategory::Memory => 2,
        CmdCategory::Tools => 3,
        CmdCategory::Display => 4,
    }
}

pub fn render_palette_overlay(
    ctx: &egui::Context,
    palette_input: &str,
    palette_selection: usize,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    use crate::palette::fuzzy_filter;
    let mut result: Option<AppAction> = None;

    let screen = ctx.content_rect();
    let w = (screen.width() * 0.6).max(420.0).min(screen.width());
    let filtered = fuzzy_filter(palette_input);
    let max_rows = 16;
    let row_count = filtered.len().min(max_rows);
    let h = (44.0 + (row_count as f32 * 24.0) + 32.0).min(screen.height() - 20.0);

    let rect = crate::responsive::overlay_rect(ctx, w, h, Some(screen.top() + 40.0));
    let w = rect.width();
    let h = rect.height();
    let pos = rect.min;

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
                .inner_margin(egui::Margin::symmetric(8, 6)),
        )
        .show(ctx, |ui| {
            ui.set_width(w - 16.0);

            // ── Search input ────────────────────────────────────
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
            separator(ui, theme);
            ui.add_space(2.0);

            // ── Results (grouped by category) ───────────────────
            if filtered.is_empty() {
                ui.label(
                    egui::RichText::new("  No matching commands")
                        .color(theme.text_muted())
                        .monospace()
                        .italics()
                        .size(12.0),
                );
            } else {
                // Group entries by category for display
                let grouped = group_by_category(&filtered);
                let _ = max_rows; // height calc only; scroll handles overflow

                // Track flat index for selection highlight
                let mut flat_idx = 0usize;

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(h - 80.0)
                    .show(ui, |ui| {
                        for (cat, entries) in &grouped {
                            // Category header
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new(format!("── {} ──", category_label(*cat)))
                                    .color(theme.text_muted())
                                    .monospace()
                                    .size(10.0),
                            );
                            ui.add_space(1.0);

                            for entry in entries {
                                let is_sel = flat_idx == palette_selection;
                                let bg = if is_sel {
                                    theme.bg_surface2()
                                } else {
                                    egui::Color32::TRANSPARENT
                                };

                                let resp = ui.horizontal(|ui| {
                                    let row_rect = ui.available_rect_before_wrap();
                                    let row_rect = egui::Rect::from_min_size(
                                        row_rect.min,
                                        egui::vec2(row_rect.width(), 22.0),
                                    );
                                    ui.painter().rect_filled(row_rect, 0.0, bg);

                                    // Trigger label
                                    ui.label(
                                        egui::RichText::new(format!("/{}", entry.def.trigger))
                                            .color(if is_sel {
                                                theme.primary()
                                            } else {
                                                theme.text_primary()
                                            })
                                            .monospace()
                                            .strong()
                                            .size(12.0),
                                    );

                                    // Arg hint (if any)
                                    if let Some(hint) = entry.def.arg_hint {
                                        ui.label(
                                            egui::RichText::new(hint)
                                                .color(theme.text_dim())
                                                .monospace()
                                                .italics()
                                                .size(10.0),
                                        );
                                    }

                                    // Description (right-aligned)
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(entry.def.description)
                                                    .color(theme.text_muted())
                                                    .monospace()
                                                    .size(10.0),
                                            );
                                        },
                                    );
                                });

                                if resp.response.interact(egui::Sense::click()).clicked() {
                                    result = Some(AppAction::ExecutePaletteCommand(
                                        entry.def.trigger.to_string(),
                                    ));
                                }

                                flat_idx += 1;
                            }
                        }
                    });
            }

            ui.add_space(2.0);
            separator(ui, theme);
            ui.add_space(2.0);

            // ── Footer: selected item description + key hints ───
            ui.horizontal(|ui| {
                // Show description of selected item
                if let Some(entry) = get_flat_entry(&filtered, palette_selection) {
                    ui.label(
                        egui::RichText::new(entry.def.description)
                            .color(theme.text_primary())
                            .monospace()
                            .size(10.0),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("↑↓ navigate  ⏎ run  Esc close")
                            .color(theme.text_dim())
                            .monospace()
                            .size(9.0),
                    );
                });
            });
        });

    result
}

/// Draw a thin 1px separator.
fn separator(ui: &mut egui::Ui, theme: &crate::theme::ThemeColors) {
    let r = ui.available_rect_before_wrap();
    let r = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), 1.0));
    ui.painter().rect_filled(r, 0.0, theme.border_base());
    ui.advance_cursor_after_rect(r);
}

/// Group filtered entries by category, preserving category order.
fn group_by_category<'a, 'b>(
    entries: &'b [crate::palette::FilteredCmd<'a>],
) -> Vec<(
    cade_core::resources::palette::CmdCategory,
    Vec<&'b crate::palette::FilteredCmd<'a>>,
)> {
    use std::collections::BTreeMap;

    let mut map: BTreeMap<
        u8,
        (
            cade_core::resources::palette::CmdCategory,
            Vec<&'b crate::palette::FilteredCmd<'a>>,
        ),
    > = BTreeMap::new();

    for entry in entries {
        let order = category_order(entry.def.category);
        map.entry(order)
            .or_insert_with(|| (entry.def.category, Vec::new()))
            .1
            .push(entry);
    }

    map.into_values().collect()
}

/// Get the flat-indexed entry (matching the selection index in the grouped display).
fn get_flat_entry<'a, 'b>(
    entries: &'b [crate::palette::FilteredCmd<'a>],
    idx: usize,
) -> Option<&'b crate::palette::FilteredCmd<'a>> {
    // When grouped, we iterate categories in order and flatten
    let grouped = group_by_category(entries);
    let mut flat = 0;
    for (_cat, group) in &grouped {
        for entry in group {
            if flat == idx {
                return Some(entry);
            }
            flat += 1;
        }
    }
    None
}
