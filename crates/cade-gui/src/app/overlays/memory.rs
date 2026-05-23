//! Memory-block viewer and editor overlay.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;
#[allow(clippy::too_many_arguments)]
pub fn render(
    ui: &mut egui::Ui,
    blocks: &[crate::api::MemoryBlock],
    selection: usize,
    edit_buffer: &str,
    loading: bool,
    saving: bool,
    error: Option<&str>,
    save_notice: Option<&str>,
    dirty: bool,
    history_open: bool,
    history: &[crate::api::MemoryHistoryRevision],
    history_loading: bool,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    // ── Header ─────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("🧠  Agent memory")
                .color(theme.primary())
                .strong()
                .size(16.0),
        );
        if loading {
            ui.spinner();
            ui.label(
                egui::RichText::new("loading…")
                    .color(theme.text_muted())
                    .small(),
            );
        }
    });

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    // ── Per-overlay error ────────────────────────────────
    if let Some(err) = error {
        egui::Frame::NONE
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

    // ── Per-overlay save-success notice ──────────────────
    if let Some(notice) = save_notice {
        egui::Frame::NONE
            .fill(theme.success().gamma_multiply(0.15))
            .stroke(egui::Stroke::new(1.0, theme.success()))
            .corner_radius(egui::CornerRadius::same(4))
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("✓ {notice}"))
                        .color(theme.success())
                        .small(),
                );
            });
        ui.add_space(6.0);
    }

    if blocks.is_empty() && !loading {
        ui.label(
            egui::RichText::new("No memory blocks — nothing to show.")
                .color(theme.text_muted())
                .italics(),
        );
        return result;
    }

    // ── Horizontal Layout: Navigation | Editor | History ──
    let body_height = ui.available_height() - 10.0;
    let list_width = 220.0;
    let history_width = 320.0;

    ui.horizontal(|ui| {
        // 1. Left column — block list grouped by functions/tiers
        ui.allocate_ui(egui::vec2(list_width, body_height), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("mem_block_list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Group blocks into Active (short) and Archived (long)
                    let active_blocks: Vec<_> = blocks
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| b.tier.as_deref() == Some("short") || b.tier.is_none())
                        .collect();
                    let archived_blocks: Vec<_> = blocks
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| b.tier.as_deref() == Some("long"))
                        .collect();

                    if !active_blocks.is_empty() {
                        ui.label(
                            egui::RichText::new("Active Memory")
                                .color(theme.text_dim())
                                .strong()
                                .size(11.0),
                        );
                        ui.add_space(4.0);
                        for (idx, block) in active_blocks {
                            render_block_item(ui, idx, block, selection, theme, &mut result);
                        }
                        ui.add_space(10.0);
                    }

                    if !archived_blocks.is_empty() {
                        ui.label(
                            egui::RichText::new("Archived / Long-term")
                                .color(theme.text_dim())
                                .strong()
                                .size(11.0),
                        );
                        ui.add_space(4.0);
                        for (idx, block) in archived_blocks {
                            render_block_item(ui, idx, block, selection, theme, &mut result);
                        }
                    }
                });
        });

        ui.separator();

        // 2. Middle column — editor
        let middle_width = if history_open {
            ui.available_width() - history_width - 16.0
        } else {
            ui.available_width()
        };

        ui.allocate_ui(egui::vec2(middle_width, body_height), |ui| {
            let selected = blocks.get(selection);
            if let Some(block) = selected {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("/{}", block.label))
                            .color(theme.text_primary())
                            .monospace()
                            .strong()
                            .size(16.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn_label = if history_open {
                            "Hide History"
                        } else {
                            "Show History"
                        };
                        let btn = egui::Button::new(btn_label).fill(theme.bg_surface2());
                        if ui.add(btn).clicked() {
                            result = Some(AppAction::ToggleMemoryHistory);
                        }
                    });
                });

                if let Some(d) = &block.description {
                    ui.label(egui::RichText::new(d).color(theme.text_muted()).small());
                }
                ui.add_space(12.0);

                let mut buf = edit_buffer.to_string();
                let editor_height = body_height - 80.0;

                let resp = egui::Frame::NONE
                    .fill(theme.bg_input())
                    .corner_radius(egui::CornerRadius::same(6))
                    .stroke(egui::Stroke::new(1.0, theme.border_base()))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut buf)
                                .desired_rows(12)
                                .desired_width(ui.available_width())
                                .min_size(egui::vec2(ui.available_width(), editor_height)),
                        )
                    })
                    .inner;

                if resp.changed() {
                    result = Some(AppAction::SetMemoryEditBuffer(buf.clone()));
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let save_label = if saving {
                        "Saving…"
                    } else if dirty {
                        "Save Changes"
                    } else {
                        "Saved"
                    };

                    let save_btn = egui::Button::new(egui::RichText::new(save_label).strong())
                        .fill(if dirty {
                            theme.success()
                        } else {
                            theme.bg_surface2()
                        });

                    if ui
                        .add_enabled(!saving && dirty, save_btn)
                        .on_hover_text("Ctrl+S")
                        .clicked()
                    {
                        result = Some(AppAction::SaveMemoryBlock);
                    }

                    if saving {
                        ui.spinner();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if dirty {
                            ui.label(
                                egui::RichText::new("● unsaved changes")
                                    .color(theme.warning())
                                    .small(),
                            );
                        }
                    });
                });
            }
        });

        // 3. Right column — history
        if history_open {
            ui.separator();
            ui.allocate_ui(egui::vec2(ui.available_width(), body_height), |ui| {
                ui.label(
                    egui::RichText::new("Versioning & History")
                        .color(theme.primary())
                        .strong(),
                );
                ui.add_space(8.0);

                if history_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            egui::RichText::new("Loading revisions...")
                                .color(theme.text_muted())
                                .small(),
                        );
                    });
                } else if history.is_empty() {
                    ui.label(
                        egui::RichText::new("No history found for this block.")
                            .color(theme.text_muted())
                            .italics(),
                    );
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("mem_history_list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for rev in history {
                                egui::Frame::NONE
                                    .fill(theme.bg_surface2())
                                    .corner_radius(egui::CornerRadius::same(6))
                                    .stroke(egui::Stroke::new(1.0, theme.border_base()))
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "Rev {}",
                                                    &rev.id[..8]
                                                ))
                                                .strong()
                                                .color(theme.text_primary()),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if ui.button("Restore").clicked() {
                                                        result =
                                                            Some(AppAction::RestoreMemoryRevision(
                                                                rev.id.clone(),
                                                            ));
                                                    }
                                                },
                                            );
                                        });
                                        ui.add_space(6.0);
                                        let preview = if rev.value.len() > 150 {
                                            format!("{}...", &rev.value[..150])
                                        } else {
                                            rev.value.clone()
                                        };
                                        ui.label(
                                            egui::RichText::new(&preview)
                                                .monospace()
                                                .color(theme.text_muted())
                                                .size(10.0),
                                        );
                                    });
                                ui.add_space(6.0);
                            }
                        });
                }
            });
        }
    });

    result
}

fn render_block_item(
    ui: &mut egui::Ui,
    idx: usize,
    block: &crate::api::MemoryBlock,
    selection: usize,
    theme: &crate::theme::ThemeColors,
    result: &mut Option<AppAction>,
) {
    let is_sel = idx == selection;
    let bg = if is_sel {
        theme.bg_surface2()
    } else {
        theme.bg_surface0()
    };
    let resp = egui::Frame::NONE
        .fill(bg)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&block.label)
                        .color(if is_sel {
                            theme.text_primary()
                        } else {
                            theme.primary()
                        })
                        .monospace()
                        .strong(),
                );
            });
        })
        .response
        .interact(egui::Sense::click());

    if resp.clicked() && !is_sel {
        *result = Some(AppAction::SelectMemoryBlock(idx));
    }
    ui.add_space(2.0);
}
