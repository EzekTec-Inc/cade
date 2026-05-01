//! Checkpoints browser overlay.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;
use super::artifacts::{format_age, unix_now};
pub fn render_checkpoints_overlay(
    ctx: &egui::Context,
    checkpoints: &[crate::api::CheckpointRow],
    loading: bool,
    busy: bool,
    error: Option<&str>,
    notice: Option<&str>,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let screen = ctx.content_rect();
    let rect = crate::responsive::overlay_rect(ctx, 720.0, 480.0, None);
    let w = rect.width();
    let h = rect.height();
    let pos = rect.min;

    let mut open = true;
    egui::Window::new("Checkpoints")
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

            // ── Header ────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("🔖  Checkpoints")
                        .color(theme.primary())
                        .strong()
                        .size(16.0),
                );
                if loading || busy {
                    ui.spinner();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        result = Some(AppAction::CloseCheckpointsOverlay);
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

            // ── Banners ───────────────────────────────────────────
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

            if let Some(msg) = notice {
                egui::Frame::new()
                    .fill(theme.success().gamma_multiply(0.15))
                    .stroke(egui::Stroke::new(1.0, theme.success()))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!("✓ {msg}"))
                                .color(theme.success())
                                .small(),
                        );
                    });
                ui.add_space(6.0);
            }

            if checkpoints.is_empty() && !loading {
                ui.label(
                    egui::RichText::new(
                        "No checkpoints yet — create one via /checkpoints or the CLI.",
                    )
                    .color(theme.text_muted())
                    .italics(),
                );
                return;
            }

            // ── Checkpoint list ───────────────────────────────────
            egui::ScrollArea::vertical()
                .id_salt("cp_list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for cp in checkpoints.iter() {
                        egui::Frame::new()
                            .fill(theme.bg_surface0())
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // Left: label + meta
                                    ui.vertical(|ui| {
                                        let title = cp.label.as_deref().unwrap_or("(unlabelled)");
                                        ui.label(
                                            egui::RichText::new(title)
                                                .color(theme.text_primary())
                                                .strong(),
                                        );
                                        // Short id + branch
                                        let short_id = if cp.id.len() > 12 {
                                            format!("{}…", &cp.id[..12])
                                        } else {
                                            cp.id.clone()
                                        };
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{short_id}  •  {}",
                                                cp.branch_id
                                            ))
                                            .color(theme.text_muted())
                                            .monospace()
                                            .size(10.0),
                                        );
                                        if let Some(desc) = &cp.description {
                                            ui.label(
                                                egui::RichText::new(desc)
                                                    .color(theme.text_muted())
                                                    .small(),
                                            );
                                        }
                                        // Human-readable relative timestamp
                                        let age_secs = unix_now() - cp.created_at;
                                        ui.label(
                                            egui::RichText::new(format_age(age_secs))
                                                .color(theme.text_dim())
                                                .small(),
                                        );
                                    });

                                    // Right: action buttons
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let del = egui::Button::new(
                                                egui::RichText::new("🗑").color(theme.error()),
                                            )
                                            .small();
                                            if ui
                                                .add_enabled(!busy, del)
                                                .on_hover_text("Delete checkpoint")
                                                .clicked()
                                            {
                                                result = Some(AppAction::DeleteCheckpoint(
                                                    cp.id.clone(),
                                                ));
                                            }
                                            let restore = egui::Button::new("⏪ Restore").small();
                                            if ui
                                                .add_enabled(!busy, restore)
                                                .on_hover_text(
                                                    "Restore working tree to this checkpoint",
                                                )
                                                .clicked()
                                            {
                                                result = Some(AppAction::RestoreCheckpoint(
                                                    cp.id.clone(),
                                                ));
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(4.0);
                    }
                });
        });

    if !open && result.is_none() {
        result = Some(AppAction::CloseCheckpointsOverlay);
    }

    result
}
