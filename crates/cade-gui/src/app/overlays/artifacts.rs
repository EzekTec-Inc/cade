//! Artifacts browser overlay.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;
pub fn render_artifacts_overlay(
    ctx: &egui::Context,
    artifacts: &[crate::api::ArtifactInfo],
    selection: Option<usize>,
    detail: Option<&crate::api::ArtifactDetail>,
    loading: bool,
    busy: bool,
    error: Option<&str>,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let rect = crate::responsive::overlay_rect(ctx, 800.0, 520.0, None);
    let w = rect.width();
    let h = rect.height();
    let pos = rect.min;

    let mut open = true;
    egui::Window::new("Artifacts")
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
                    egui::RichText::new("📦  Artifacts")
                        .color(theme.primary())
                        .strong()
                        .size(16.0),
                );
                if loading || busy {
                    ui.spinner();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        result = Some(AppAction::CloseArtifactsOverlay);
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

            // ── Error banner ──────────────────────────────────────
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

            if artifacts.is_empty() && !loading {
                ui.label(
                    egui::RichText::new("No artifacts yet.")
                        .color(theme.text_muted())
                        .italics(),
                );
                return;
            }

            // ── Split: left list, right detail ────────────────────
            let body_h = ui.available_height();
            let list_w = 240.0;

            ui.horizontal(|ui| {
                // Left: artifact list
                ui.allocate_ui(egui::vec2(list_w, body_h), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("art_list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (idx, art) in artifacts.iter().enumerate() {
                                let is_sel = selection == Some(idx);
                                let bg = if is_sel {
                                    theme.bg_surface2()
                                } else {
                                    theme.bg_surface0()
                                };
                                let resp = egui::Frame::new()
                                    .fill(bg)
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(8, 5))
                                    .show(ui, |ui| {
                                        ui.set_width(list_w - 16.0);
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new(kind_icon(&art.kind))
                                                    .color(kind_color(&art.kind, theme))
                                                    .strong()
                                                    .size(11.0),
                                            );
                                            let short_id = if art.id.len() > 14 {
                                                format!("{}…", &art.id[..14])
                                            } else {
                                                art.id.clone()
                                            };
                                            ui.label(
                                                egui::RichText::new(&short_id)
                                                    .color(theme.text_muted())
                                                    .monospace()
                                                    .size(9.0),
                                            );
                                            ui.label(
                                                egui::RichText::new(format_size(art.size_bytes))
                                                    .color(theme.text_dim())
                                                    .size(9.0),
                                            );
                                        });
                                    })
                                    .response
                                    .interact(egui::Sense::click());
                                if resp.clicked() && !is_sel {
                                    result = Some(AppAction::SelectArtifact(idx));
                                }
                                ui.add_space(2.0);
                            }
                        });
                });

                ui.separator();

                // Right: detail pane
                ui.vertical(|ui| {
                    match detail {
                        None if busy => {
                            ui.add_space(40.0);
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("Loading…")
                                    .color(theme.text_muted())
                                    .small(),
                            );
                        }
                        None => {
                            ui.add_space(40.0);
                            ui.label(
                                egui::RichText::new("← Select an artifact")
                                    .color(theme.text_muted())
                                    .italics(),
                            );
                        }
                        Some(d) => {
                            // Header row: kind + delete button
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{}  {}",
                                        kind_icon(&d.kind),
                                        d.kind
                                    ))
                                    .color(kind_color(&d.kind, theme))
                                    .strong(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let del = egui::Button::new(
                                            egui::RichText::new("🗑 Delete")
                                                .color(theme.error())
                                                .small(),
                                        );
                                        if ui
                                            .add_enabled(!busy, del)
                                            .on_hover_text("Permanently remove this artifact")
                                            .clicked()
                                        {
                                            result = Some(AppAction::DeleteSelectedArtifact);
                                        }
                                    },
                                );
                            });

                            ui.add_space(2.0);

                            // Meta: id + content-type + size
                            ui.label(
                                egui::RichText::new(&d.id)
                                    .monospace()
                                    .color(theme.text_muted())
                                    .size(10.0),
                            );
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&d.content_type)
                                        .color(theme.text_dim())
                                        .size(10.0),
                                );
                                ui.label(
                                    egui::RichText::new(format_size(d.size_bytes))
                                        .color(theme.text_dim())
                                        .size(10.0),
                                );
                            });

                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(4.0);

                            // Content preview (text artifacts only)
                            match &d.data_text {
                                Some(text) if !text.is_empty() => {
                                    // Truncate to avoid rendering megabytes
                                    let preview: String = text.chars().take(4096).collect();
                                    let suffix = if text.len() > 4096 {
                                        "\n…(truncated)"
                                    } else {
                                        ""
                                    };
                                    let full = format!("{preview}{suffix}");
                                    egui::ScrollArea::vertical()
                                        .id_salt("art_content")
                                        .auto_shrink([false, false])
                                        .max_height(body_h - 120.0)
                                        .show(ui, |ui| {
                                            ui.add(
                                                egui::TextEdit::multiline(&mut full.as_str())
                                                    .desired_width(ui.available_width())
                                                    .font(egui::TextStyle::Monospace),
                                            );
                                        });
                                }
                                _ => {
                                    ui.label(
                                        egui::RichText::new("(binary / no text content)")
                                            .color(theme.text_dim())
                                            .italics()
                                            .small(),
                                    );
                                }
                            }
                        }
                    }
                });
            });
        });

    if !open && result.is_none() {
        result = Some(AppAction::CloseArtifactsOverlay);
    }

    result
}

// ── Shared overlay helpers ────────────────────────────────────────────

/// Return a current Unix timestamp in seconds (best-effort; falls back
/// to 0 on WASM where `SystemTime` is not reliably available).
pub fn unix_now() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
    }
    #[cfg(target_arch = "wasm32")]
    {
        (web_sys::js_sys::Date::now() / 1000.0) as i64
    }
}

/// Convert a duration in seconds into a human-readable relative string.
pub fn format_age(age_secs: i64) -> String {
    let age = age_secs.max(0) as u64;
    if age < 60 {
        "just now".into()
    } else if age < 3600 {
        format!("{}m ago", age / 60)
    } else if age < 86400 {
        format!("{}h ago", age / 3600)
    } else {
        format!("{}d ago", age / 86400)
    }
}

/// Format a byte count as a compact human-readable string.
pub fn format_size(bytes: i64) -> String {
    let b = bytes.max(0) as u64;
    if b < 1024 {
        format!("{b} B")
    } else if b < 1024 * 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
    }
}

/// Pick an emoji icon for a known artifact kind.
pub fn kind_icon(kind: &str) -> String {
    match kind {
        "log" => "📋 log".into(),
        "diff" => "📝 diff".into(),
        "test_report" => "🧪 test_report".into(),
        "screenshot" => "🖼 screenshot".into(),
        "fetched_doc" => "🌐 fetched_doc".into(),
        "pdf" => "📄 pdf".into(),
        "trace" => "🔍 trace".into(),
        other => format!("📦 {other}"),
    }
}

/// Pick a theme color for a known artifact kind.
pub fn kind_color(kind: &str, theme: &crate::theme::ThemeColors) -> egui::Color32 {
    match kind {
        "log" => theme.text_primary(),
        "diff" => theme.warning(),
        "test_report" => theme.success(),
        "screenshot" => theme.teal(),
        "fetched_doc" => theme.primary(),
        "pdf" => theme.purple(),
        _ => theme.text_muted(),
    }
}
