//! Checkpoints browser overlay.

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
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let screen = ctx.content_rect();
    let w = 720.0_f32.min(screen.width() - 40.0);
    let h = 480.0_f32.min(screen.height() - 80.0);
    let pos = egui::pos2(
        screen.center().x - w / 2.0,
        screen.center().y - h / 2.0,
    );

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
                .fill(crate::theme::BG_SURFACE1)
                .stroke(egui::Stroke::new(1.0, crate::theme::BORDER_FOCUS))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(12.0),
        )
        .show(ctx, |ui| {
            ui.set_width(w - 24.0);

            // ── Header ────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("🔖  Checkpoints")
                        .color(crate::theme::PRIMARY)
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
                            .color(crate::theme::TEXT_DIM)
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
                    .fill(crate::theme::ERROR.gamma_multiply(0.15))
                    .stroke(egui::Stroke::new(1.0, crate::theme::ERROR))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!("⚠ {err}"))
                                .color(crate::theme::ERROR)
                                .small(),
                        );
                    });
                ui.add_space(6.0);
            }

            if let Some(msg) = notice {
                egui::Frame::new()
                    .fill(crate::theme::SUCCESS.gamma_multiply(0.15))
                    .stroke(egui::Stroke::new(1.0, crate::theme::SUCCESS))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!("✓ {msg}"))
                                .color(crate::theme::SUCCESS)
                                .small(),
                        );
                    });
                ui.add_space(6.0);
            }

            if checkpoints.is_empty() && !loading {
                ui.label(
                    egui::RichText::new("No checkpoints yet — create one via /checkpoints or the CLI.")
                        .color(crate::theme::TEXT_MUTED)
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
                            .fill(crate::theme::BG_SURFACE0)
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // Left: label + meta
                                    ui.vertical(|ui| {
                                        let title = cp
                                            .label
                                            .as_deref()
                                            .unwrap_or("(unlabelled)");
                                        ui.label(
                                            egui::RichText::new(title)
                                                .color(crate::theme::TEXT_PRIMARY)
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
                                            .color(crate::theme::TEXT_MUTED)
                                            .monospace()
                                            .size(10.0),
                                        );
                                        if let Some(desc) = &cp.description {
                                            ui.label(
                                                egui::RichText::new(desc)
                                                    .color(crate::theme::TEXT_MUTED)
                                                    .small(),
                                            );
                                        }
                                        // Human-readable relative timestamp
                                        let age_secs = unix_now() - cp.created_at;
                                        ui.label(
                                            egui::RichText::new(format_age(age_secs))
                                                .color(crate::theme::TEXT_DIM)
                                                .small(),
                                        );
                                    });

                                    // Right: action buttons
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let del = egui::Button::new(
                                                egui::RichText::new("🗑")
                                                    .color(crate::theme::ERROR),
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
                                            let restore =
                                                egui::Button::new("⏪ Restore").small();
                                            if ui
                                                .add_enabled(!busy, restore)
                                                .on_hover_text("Restore working tree to this checkpoint")
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
