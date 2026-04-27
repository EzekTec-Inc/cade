//! Skills browser overlay — list available skills, load/unload.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;

pub use crate::api::SkillEntry;

pub fn render_skills_overlay(
    ctx: &egui::Context,
    all_skills: &[SkillEntry],
    loaded_ids: &[String],
    loading: bool,
    filter: &str,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let screen = ctx.content_rect();
    let w = (screen.width() * 0.7).max(500.0).min(screen.width() - 20.0);
    let h = (screen.height() * 0.7)
        .max(300.0)
        .min(screen.height() - 40.0);
    let pos = egui::pos2((screen.width() - w) / 2.0, (screen.height() - h) / 2.0);

    egui::Window::new("Skills Browser")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .fixed_pos(pos)
        .fixed_size([w, h])
        .frame(
            egui::Frame::new()
                .fill(theme.bg_surface1())
                .stroke(egui::Stroke::new(1.0, theme.border_base()))
                .corner_radius(egui::CornerRadius::ZERO)
                .inner_margin(egui::Margin::symmetric(10, 8)),
        )
        .show(ctx, |ui| {
            ui.set_width(w - 20.0);

            // ── Header ──────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(" Skills")
                        .color(theme.primary())
                        .monospace()
                        .strong()
                        .size(13.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} available · {} loaded",
                            all_skills.len(),
                            loaded_ids.len(),
                        ))
                        .color(theme.text_dim())
                        .monospace()
                        .size(10.0),
                    );
                });
            });

            // Separator
            let r = ui.available_rect_before_wrap();
            let r = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), 1.0));
            ui.painter().rect_filled(r, 0.0, theme.border_base());
            ui.advance_cursor_after_rect(r);
            ui.add_space(4.0);

            // ── Filter input ────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("🔍").color(theme.text_dim()).size(12.0));
                let mut q = filter.to_string();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut q)
                        .hint_text("Filter skills…")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                );
                if resp.changed() {
                    result = Some(AppAction::SetSkillsFilter(q));
                }
            });
            ui.add_space(4.0);

            if loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new("Loading skills…")
                            .color(theme.text_dim())
                            .monospace()
                            .size(11.0),
                    );
                });
                return;
            }

            // ── Skills list ─────────────────────────────────────
            let filter_lower = filter.to_lowercase();
            let filtered: Vec<&SkillEntry> = all_skills
                .iter()
                .filter(|s| {
                    filter_lower.is_empty()
                        || s.id.to_lowercase().contains(&filter_lower)
                        || s.name.to_lowercase().contains(&filter_lower)
                        || s.description.to_lowercase().contains(&filter_lower)
                        || s.tags
                            .iter()
                            .any(|t| t.to_lowercase().contains(&filter_lower))
                })
                .collect();

            if filtered.is_empty() {
                ui.label(
                    egui::RichText::new("  No matching skills")
                        .color(theme.text_muted())
                        .monospace()
                        .italics()
                        .size(11.0),
                );
            } else {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(h - 120.0)
                    .show(ui, |ui| {
                        for skill in &filtered {
                            let is_loaded = loaded_ids.contains(&skill.id);
                            let bg = if is_loaded {
                                theme.tinted_bg(theme.success(), 12)
                            } else {
                                egui::Color32::TRANSPARENT
                            };

                            let row_rect = ui.available_rect_before_wrap();
                            let row_rect = egui::Rect::from_min_size(
                                row_rect.min,
                                egui::vec2(row_rect.width(), 36.0),
                            );
                            ui.painter().rect_filled(row_rect, 0.0, bg);

                            ui.horizontal(|ui| {
                                // Status indicator
                                let status = if is_loaded { "●" } else { "○" };
                                let status_color = if is_loaded {
                                    theme.success()
                                } else {
                                    theme.text_dim()
                                };
                                ui.label(
                                    egui::RichText::new(status).color(status_color).size(10.0),
                                );

                                // Skill name + ID
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(&skill.name)
                                                .color(theme.text_primary())
                                                .monospace()
                                                .strong()
                                                .size(11.0),
                                        );
                                        ui.label(
                                            egui::RichText::new(format!("[{}]", skill.scope))
                                                .color(theme.text_dim())
                                                .monospace()
                                                .size(9.0),
                                        );
                                        // Tags
                                        for tag in &skill.tags {
                                            ui.label(
                                                egui::RichText::new(format!("#{tag}"))
                                                    .color(theme.teal())
                                                    .monospace()
                                                    .size(9.0),
                                            );
                                        }
                                    });
                                    ui.label(
                                        egui::RichText::new(&skill.description)
                                            .color(theme.text_muted())
                                            .monospace()
                                            .size(10.0),
                                    );
                                });

                                // Load/Unload button (right-aligned)
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let (label, color) = if is_loaded {
                                            ("unload", theme.error())
                                        } else {
                                            ("load", theme.success())
                                        };
                                        if ui
                                            .add(
                                                egui::Label::new(
                                                    egui::RichText::new(format!("[{label}]"))
                                                        .color(color)
                                                        .monospace()
                                                        .strong()
                                                        .size(10.0),
                                                )
                                                .sense(egui::Sense::click()),
                                            )
                                            .clicked()
                                        {
                                            if is_loaded {
                                                result =
                                                    Some(AppAction::UnloadSkill(skill.id.clone()));
                                            } else {
                                                result =
                                                    Some(AppAction::LoadSkill(skill.id.clone()));
                                            }
                                        }

                                        // Token cost hint
                                        let approx_tok = skill.body_chars / 3;
                                        ui.label(
                                            egui::RichText::new(format!("~{}tok", approx_tok))
                                                .color(theme.text_dim())
                                                .monospace()
                                                .size(9.0),
                                        );
                                    },
                                );
                            });
                            ui.add_space(1.0);
                        }
                    });
            }

            ui.add_space(4.0);
            // Separator
            let r = ui.available_rect_before_wrap();
            let r = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), 1.0));
            ui.painter().rect_filled(r, 0.0, theme.border_base());
            ui.advance_cursor_after_rect(r);
            ui.add_space(2.0);

            // ── Footer ──────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Esc close  ·  Click [load]/[unload] to toggle")
                        .color(theme.text_dim())
                        .monospace()
                        .size(9.0),
                );
            });
        });

    result
}
