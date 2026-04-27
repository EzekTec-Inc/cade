//! Full-screen command menu overlay.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;

pub fn render_menu_overlay(
    ctx: &egui::Context,
    menu_input: &str,
    menu_selection: usize,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    use crate::palette::fuzzy_filter;
    let mut result: Option<AppAction> = None;

    let screen = ctx.content_rect();
    let w = 800.0_f32.min(screen.width() - 40.0);
    let h = 600.0_f32.min(screen.height() - 80.0);
    let pos = egui::pos2(screen.center().x - w / 2.0, screen.center().y - h / 2.0);

    // Dim backdrop
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("menu_overlay_backdrop"),
    ));
    painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(140));

    egui::Window::new("Command Menu")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
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

            // Header + query input
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("🔍").color(theme.primary()).size(16.0));
                let mut q = menu_input.to_string();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut q)
                        .hint_text("Type to filter commands...")
                        .desired_width(ui.available_width()),
                );
                resp.request_focus();
                if resp.changed() {
                    result = Some(AppAction::SetMenuInput(q));
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            // Filtered entries
            let filtered = fuzzy_filter(menu_input);
            if filtered.is_empty() {
                ui.label(
                    egui::RichText::new("No matching commands")
                        .color(theme.text_muted())
                        .italics(),
                );
            } else {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(h - 90.0)
                    .show(ui, |ui| {
                        // Group by category to replicate TUI layout
                        use cade_core::resources::palette::CmdCategory;
                        use std::collections::BTreeMap;

                        let mut grouped: BTreeMap<u8, Vec<(usize, &crate::palette::FilteredCmd)>> =
                            BTreeMap::new();

                        // We iterate the filtered flat list, but preserve original sorting per category.
                        for (idx, cmd) in filtered.iter().enumerate() {
                            let order = match cmd.def.category {
                                CmdCategory::Session => 0,
                                CmdCategory::Display => 1,
                                CmdCategory::Memory => 2,
                                CmdCategory::Tools => 3,
                                CmdCategory::Navigation => 4,
                            };
                            grouped.entry(order).or_default().push((idx, cmd));
                        }

                        for (order, group_items) in grouped {
                            let category_name = match order {
                                0 => "Session",
                                1 => "Model & Mode",
                                2 => "Memory",
                                3 => "Tools & Providers",
                                4 => "Navigation & Misc",
                                _ => "Misc",
                            };

                            // Only show section header if there are matching items inside
                            if !group_items.is_empty() {
                                ui.add_space(8.0);
                                ui.heading(
                                    egui::RichText::new(category_name)
                                        .color(theme.primary())
                                        .strong(),
                                );
                                ui.add_space(4.0);

                                for (idx, entry) in group_items {
                                    let is_sel = idx == menu_selection;
                                    let bg = if is_sel {
                                        theme.bg_surface2()
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    let text_col = if is_sel {
                                        theme.text_primary()
                                    } else {
                                        theme.text_muted()
                                    };

                                    let resp = ui.allocate_response(
                                        egui::vec2(ui.available_width(), 22.0),
                                        egui::Sense::click(),
                                    );

                                    if resp.hovered() {
                                        ui.painter().rect_filled(
                                            resp.rect,
                                            4.0,
                                            theme.bg_surface2(),
                                        );
                                    } else if is_sel {
                                        ui.painter().rect_filled(resp.rect, 4.0, bg);
                                    }

                                    let trigger_text = format!("/{}", entry.def.trigger);

                                    ui.allocate_ui_with_layout(
                                        resp.rect.size(),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.horizontal(|ui| {
                                                ui.add_space(8.0);
                                                let label_resp = ui.label(
                                                    egui::RichText::new(&trigger_text)
                                                        .color(theme.primary())
                                                        .strong(),
                                                );
                                                // Make trigger label fixed width so descriptions align nicely
                                                ui.allocate_space(egui::vec2(
                                                    (180.0 - label_resp.rect.width()).max(0.0),
                                                    0.0,
                                                ));
                                                ui.label(
                                                    egui::RichText::new(entry.def.description)
                                                        .color(text_col),
                                                );
                                            });
                                        },
                                    );

                                    if resp.clicked() {
                                        result = Some(AppAction::ExecuteMenuCmd);
                                    }
                                }
                            }
                        }
                    });
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("↑↓ Navigate  Enter Select  Esc Cancel")
                        .color(theme.text_muted())
                        .size(10.0),
                );
            });
        });

    result
}
