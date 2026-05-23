//! Skills browser overlay — list available skills, load/unload.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;

pub use crate::api::SkillEntry;

pub fn render_skills_overlay(
    ui: &mut egui::Ui,
    all_skills: &[SkillEntry],
    loaded_ids: &[String],
    loading: bool,
    filter: &str,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    // ── Header ──────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(" Skills Library")
                .color(theme.primary())
                .monospace()
                .strong()
                .size(16.0),
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
                .size(12.0),
            );
        });
    });

    // Separator
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    // ── Filter input ────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("🔍").color(theme.text_dim()).size(14.0));
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
                    .size(14.0),
            );
        });
        return result;
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
                .size(14.0),
        );
    } else {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for skill in &filtered {
                    let is_loaded = loaded_ids.contains(&skill.id);
                    let bg = if is_loaded {
                        theme.tinted_bg(theme.success(), 12)
                    } else {
                        egui::Color32::TRANSPARENT
                    };

                    let row_rect = ui.available_rect_before_wrap();
                    let row_rect =
                        egui::Rect::from_min_size(row_rect.min, egui::vec2(row_rect.width(), 36.0));
                    ui.painter().rect_filled(row_rect, 0.0, bg);

                    ui.horizontal(|ui| {
                        // Status indicator
                        let status = if is_loaded { "●" } else { "○" };
                        let status_color = if is_loaded {
                            theme.success()
                        } else {
                            theme.text_dim()
                        };
                        ui.label(egui::RichText::new(status).color(status_color).size(14.0));

                        // Skill name + ID
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&skill.name)
                                        .color(theme.text_primary())
                                        .monospace()
                                        .strong()
                                        .size(14.0),
                                );
                                ui.label(
                                    egui::RichText::new(format!("[{}]", skill.scope))
                                        .color(theme.text_dim())
                                        .monospace()
                                        .size(12.0),
                                );
                                // Tags
                                for tag in &skill.tags {
                                    ui.label(
                                        egui::RichText::new(format!("#{tag}"))
                                            .color(theme.teal())
                                            .monospace()
                                            .size(12.0),
                                    );
                                }
                            });
                            ui.label(
                                egui::RichText::new(&skill.description)
                                    .color(theme.text_muted())
                                    .monospace()
                                    .size(12.0),
                            );
                        });

                        // Load/Unload button (right-aligned)
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                                            .size(12.0),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                if is_loaded {
                                    result = Some(AppAction::UnloadSkill(skill.id.clone()));
                                } else {
                                    result = Some(AppAction::LoadSkill(skill.id.clone()));
                                }
                            }

                            // Token cost hint
                            let approx_tok = skill.body_chars / 3;
                            ui.label(
                                egui::RichText::new(format!("~{}tok", approx_tok))
                                    .color(theme.text_dim())
                                    .monospace()
                                    .size(11.0),
                            );
                        });
                    });
                    ui.add_space(4.0);
                }
            });
    }

    result
}
