//! Tools / MCP overlay and ask_user_question widget.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;
pub fn render_tools_overlay(
    ctx: &egui::Context,
    tools: &[crate::api::AgentTool],
    loading: bool,
    error: Option<&str>,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let rect = crate::responsive::overlay_rect(ctx, 560.0, 480.0, None);
    let w = rect.width();
    let h = rect.height();
    let pos = rect.min;

    let mut open = true;
    egui::Window::new("Tools / MCP")
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

            // ── Header ─────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("🔧  Tools / MCP")
                        .color(theme.primary())
                        .strong()
                        .size(16.0),
                );
                if loading {
                    ui.spinner();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        result = Some(AppAction::CloseToolsOverlay);
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

            if let Some(err) = error {
                egui::Frame::new()
                    .fill(theme.error().gamma_multiply(0.15))
                    .stroke(egui::Stroke::new(1.0, theme.error()))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!("⚠ {err}"))
                                .color(theme.error())
                                .small(),
                        );
                    });
                ui.add_space(6.0);
            }

            if tools.is_empty() && !loading {
                ui.label(
                    egui::RichText::new(
                        "No tools attached — attach MCP servers via the CLI or settings.",
                    )
                    .color(theme.text_muted())
                    .italics(),
                );
                return;
            }

            // Tool count hint
            if !tools.is_empty() {
                ui.label(
                    egui::RichText::new(format!("{} tool(s) registered", tools.len()))
                        .color(theme.text_muted())
                        .small(),
                );
                ui.add_space(4.0);
            }

            egui::ScrollArea::vertical()
                .id_salt("tools_list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for tool in tools.iter() {
                        egui::Frame::new()
                            .fill(theme.bg_surface0())
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("⚙").color(theme.teal()));
                                    ui.label(
                                        egui::RichText::new(&tool.name)
                                            .color(theme.text_primary())
                                            .monospace()
                                            .strong(),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                egui::RichText::new(&tool.id)
                                                    .color(theme.text_dim())
                                                    .monospace()
                                                    .size(10.0),
                                            );
                                        },
                                    );
                                });
                            });
                        ui.add_space(3.0);
                    }
                });
        });

    if !open && result.is_none() {
        result = Some(AppAction::CloseToolsOverlay);
    }

    result
}

// ── Inline question widget (M18) ──────────────────────────────────────

/// Render the `ask_user_question` inline widget.
///
/// Displayed as a centred floating panel (similar to the palette) with
/// the question text, numbered options, and Submit / Dismiss buttons.
/// Keyboard: ↑↓ navigate, Enter submit, Esc dismiss.
pub fn render_question_widget(
    ctx: &egui::Context,
    question: &crate::api::Question,
    cursor: usize,
    checked: &[bool],
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let screen = ctx.content_rect();
    let w = 520.0_f32.min(screen.width() - 40.0);
    // Height scales with option count.
    let option_rows = question.options.len().max(1) as f32;
    let h = (160.0 + option_rows * 52.0).min(screen.height() - 80.0);

    let rect = crate::responsive::overlay_rect(ctx, w, h, None);
    let w = rect.width();
    let h = rect.height();
    let pos = rect.min;

    let mut open = true;
    egui::Window::new("Question")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .open(&mut open)
        .fixed_pos(pos)
        .fixed_size([w, h])
        .frame(
            egui::Frame::new()
                .fill(theme.bg_surface1())
                .stroke(egui::Stroke::new(1.5, theme.border_focus()))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(14.0),
        )
        .show(ctx, |ui| {
            ui.set_width(w - 28.0);

            // Header chip
            egui::Frame::new()
                .fill(theme.primary().gamma_multiply(0.2))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(&question.header)
                            .color(theme.primary())
                            .strong()
                            .small(),
                    );
                });
            ui.add_space(6.0);

            // Question text
            ui.label(
                egui::RichText::new(&question.question)
                    .color(theme.text_primary())
                    .strong()
                    .size(14.0),
            );
            ui.add_space(8.0);

            // Options
            for (idx, opt) in question.options.iter().enumerate() {
                let is_cursor = cursor == idx;
                let is_checked = checked.get(idx).copied().unwrap_or(false);

                let bg = if is_cursor {
                    theme.bg_surface2()
                } else {
                    theme.bg_surface0()
                };

                let resp = egui::Frame::new()
                    .fill(bg)
                    .stroke(if is_cursor {
                        egui::Stroke::new(1.0, theme.primary())
                    } else {
                        egui::Stroke::NONE
                    })
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Cursor indicator
                            if is_cursor {
                                ui.label(egui::RichText::new("❯").color(theme.primary()).strong());
                            } else {
                                ui.label(egui::RichText::new(" ").color(theme.text_dim()));
                            }

                            // Checkbox for multi-select
                            if question.multi_select {
                                let cb = if is_checked { "☑" } else { "☐" };
                                ui.label(egui::RichText::new(cb).color(if is_checked {
                                    theme.success()
                                } else {
                                    theme.text_muted()
                                }));
                            }

                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{}. {}", idx + 1, opt.label))
                                        .color(if is_cursor {
                                            theme.text_primary()
                                        } else {
                                            theme.text_muted()
                                        })
                                        .strong(),
                                );
                                if !opt.description.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&opt.description)
                                            .color(theme.text_dim())
                                            .small(),
                                    );
                                }
                            });
                        });
                    })
                    .response
                    .interact(egui::Sense::click());

                if resp.clicked() {
                    if question.multi_select {
                        // Click toggles — update cursor then action
                        if !is_cursor {
                            result = Some(AppAction::MoveQuestionCursor(
                                (idx as i32) - (cursor as i32),
                            ));
                        } else {
                            result = Some(AppAction::ToggleQuestionChecked);
                        }
                    } else {
                        // Single-select: move cursor then immediately answer
                        if !is_cursor {
                            result = Some(AppAction::MoveQuestionCursor(
                                (idx as i32) - (cursor as i32),
                            ));
                        } else {
                            result = Some(AppAction::AnswerQuestion);
                        }
                    }
                }

                ui.add_space(2.0);
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // Action row
            ui.horizontal(|ui| {
                let submit_label = if question.multi_select {
                    "Submit"
                } else {
                    "Select"
                };
                if ui.button(submit_label).clicked() {
                    result = Some(AppAction::AnswerQuestion);
                }
                if ui.button("Dismiss").clicked() {
                    result = Some(AppAction::DismissQuestion);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("↑↓ navigate · Enter submit · Esc dismiss")
                            .color(theme.text_dim())
                            .small(),
                    );
                });
            });
        });

    if !open && result.is_none() {
        result = Some(AppAction::DismissQuestion);
    }

    result
}

// ── Agents overlay (M19) ──────────────────────────────────────────────
