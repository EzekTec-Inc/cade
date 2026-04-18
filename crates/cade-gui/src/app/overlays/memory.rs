//! Memory-block viewer and editor overlay.

use eframe::egui;

use super::super::AppAction;
#[allow(clippy::too_many_arguments)]
pub fn render_memory_overlay(
    ctx: &egui::Context,
    blocks: &[crate::api::MemoryBlock],
    selection: usize,
    edit_buffer: &str,
    loading: bool,
    saving: bool,
    error: Option<&str>,
    save_notice: Option<&str>,
    dirty: bool,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let screen = ctx.content_rect();
    let w = 760.0_f32.min(screen.width() - 40.0);
    let h = 520.0_f32.min(screen.height() - 80.0);
    let pos = egui::pos2(
        screen.center().x - w / 2.0,
        screen.center().y - h / 2.0,
    );

    let mut open = true;
    egui::Window::new("Memory")
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

            // ── Header ─────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("🧠  Agent memory")
                        .color(crate::theme::PRIMARY)
                        .strong()
                        .size(16.0),
                );
                if loading {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new("loading…")
                            .color(crate::theme::TEXT_MUTED)
                            .small(),
                    );
                }
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.small_button("✕").clicked() {
                            result = Some(AppAction::CloseMemoryOverlay);
                        }
                    },
                );
            });

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            // ── Per-overlay error ────────────────────────────────
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

            // ── Per-overlay save-success notice ──────────────────
            if let Some(notice) = save_notice {
                egui::Frame::new()
                    .fill(crate::theme::SUCCESS.gamma_multiply(0.15))
                    .stroke(egui::Stroke::new(1.0, crate::theme::SUCCESS))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(format!("✓ {notice}"))
                                .color(crate::theme::SUCCESS)
                                .small(),
                        );
                    });
                ui.add_space(6.0);
            }

            if blocks.is_empty() && !loading {
                ui.label(
                    egui::RichText::new("No memory blocks — nothing to show.")
                        .color(crate::theme::TEXT_MUTED)
                        .italics(),
                );
                return;
            }

            // ── Split: left list, right editor ─────────────────
            let body_height = ui.available_height() - 40.0;
            let list_width = 180.0;

            ui.horizontal(|ui| {
                // Left column — block list
                ui.allocate_ui(egui::vec2(list_width, body_height), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("mem_block_list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (idx, block) in blocks.iter().enumerate() {
                                let is_sel = idx == selection;
                                let bg = if is_sel {
                                    crate::theme::BG_SURFACE2
                                } else {
                                    crate::theme::BG_SURFACE0
                                };
                                let resp = egui::Frame::new()
                                    .fill(bg)
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(8, 6))
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new(&block.label)
                                                    .color(crate::theme::PRIMARY)
                                                    .monospace()
                                                    .strong(),
                                            );
                                            if let Some(tier) = &block.tier {
                                                ui.label(
                                                    egui::RichText::new(tier)
                                                        .color(crate::theme::TEXT_MUTED)
                                                        .small(),
                                                );
                                            }
                                        });
                                    })
                                    .response
                                    .interact(egui::Sense::click());
                                if resp.clicked() && !is_sel {
                                    result = Some(AppAction::SelectMemoryBlock(idx));
                                }
                                ui.add_space(2.0);
                            }
                        });
                });

                ui.separator();

                // Right column — editor
                ui.vertical(|ui| {
                    let selected = blocks.get(selection);
                    if let Some(block) = selected {
                        ui.label(
                            egui::RichText::new(format!("/{}", block.label))
                                .color(crate::theme::TEXT_PRIMARY)
                                .monospace()
                                .strong(),
                        );
                        if let Some(d) = &block.description {
                            ui.label(
                                egui::RichText::new(d)
                                    .color(crate::theme::TEXT_MUTED)
                                    .small(),
                            );
                        }
                        ui.add_space(4.0);

                        let mut buf = edit_buffer.to_string();
                        let editor_height = body_height - 70.0;
                        let resp = ui.add(
                            egui::TextEdit::multiline(&mut buf)
                                .desired_rows(12)
                                .desired_width(ui.available_width())
                                .min_size(egui::vec2(
                                    ui.available_width(),
                                    editor_height,
                                )),
                        );
                        if resp.changed() {
                            result = Some(AppAction::SetMemoryEditBuffer(buf.clone()));
                        }

                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let save_label = if saving {
                                "Saving…"
                            } else if dirty {
                                "Save"
                            } else {
                                "Saved"
                            };
                            let save = egui::Button::new(save_label);
                            // Disabled when a request is in flight or the
                            // buffer matches the saved value (nothing to
                            // save).  Avoids spurious PUTs and gives the
                            // user a clear "everything is persisted" cue.
                            if ui
                                .add_enabled(!saving && dirty, save)
                                .on_hover_text("Ctrl+S")
                                .clicked()
                            {
                                result = Some(AppAction::SaveMemoryBlock);
                            }
                            if saving {
                                ui.spinner();
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if dirty {
                                        ui.label(
                                            egui::RichText::new("● unsaved changes")
                                                .color(crate::theme::WARNING)
                                                .small(),
                                        );
                                    }
                                },
                            );
                        });
                    }
                });
            });
        });

    // `open = false` from the window's built-in ✕ button.
    if !open && result.is_none() {
        result = Some(AppAction::CloseMemoryOverlay);
    }

    result
}
