//! Model picker overlay — lets users browse and select from available models.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;

/// Actions the model picker overlay can produce.
pub enum ModelPickerAction {
    /// User selected a model — carry its ID.
    Select(String),
    /// Close the overlay without selecting.
    Close,
    /// Query text changed.
    QueryChanged(String),
    /// Selection index moved.
    SelectionMoved(usize),
}

/// Render the model picker overlay.
///
/// Returns an `AppAction` when the user interacts (select, close, etc.).
#[allow(clippy::too_many_arguments)]
pub fn render_model_picker(
    ctx: &egui::Context,
    models: &[crate::api::ModelInfo],
    custom_providers: &[String],
    query: &str,
    selection: usize,
    loading: bool,
    error: Option<&str>,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let screen = ctx.content_rect();
    let w = 600.0_f32.min(screen.width() - 40.0);
    let h = 480.0_f32.min(screen.height() - 60.0);

    let rect = crate::responsive::overlay_rect(ctx, w, h, None);
    let w = rect.width();
    let h = rect.height();
    let pos = rect.min;

    // Dim backdrop
    let backdrop_rect = screen;
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("model_picker_backdrop"),
    ));
    painter.rect_filled(backdrop_rect, 0.0, theme.overlay_backdrop());

    // ESC closes
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        result = Some(AppAction::CloseModelPicker);
    }

    let mut query_buf = query.to_string();

    egui::Area::new(egui::Id::new("model_picker_area"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme.bg_surface0())
                .stroke(egui::Stroke::new(1.0, theme.border_base()))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_min_size(egui::vec2(w - 32.0, h - 32.0));
                    ui.set_max_size(egui::vec2(w - 32.0, h - 32.0));

                    // Title
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Select Model")
                                .color(theme.text_primary())
                                .strong()
                                .size(16.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(egui::RichText::new("✕").color(theme.text_dim()))
                                .clicked()
                            {
                                result = Some(AppAction::CloseModelPicker);
                            }
                        });
                    });

                    ui.add_space(8.0);

                    // Search box
                    let search_resp = ui.add(
                        egui::TextEdit::singleline(&mut query_buf)
                            .hint_text(
                                egui::RichText::new("Search models…").color(theme.text_dim()),
                            )
                            .desired_width(ui.available_width()),
                    );
                    search_resp.request_focus();

                    if search_resp.changed() && query_buf != query {
                        result = Some(AppAction::SetModelPickerQuery(query_buf.clone()));
                    }

                    ui.add_space(8.0);

                    // Error banner
                    if let Some(err) = error {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("⚠ {err}"))
                                    .color(theme.error())
                                    .size(12.0),
                            );
                        });
                        ui.add_space(4.0);
                    }

                    // Loading state
                    if loading {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("Loading models…").color(theme.text_muted()),
                            );
                        });
                        return;
                    }

                    // Filter models
                    let filtered = crate::session::filter_models(models, &query_buf);

                    if filtered.is_empty() && custom_providers.is_empty() {
                        ui.label(egui::RichText::new("No models found").color(theme.text_muted()));
                        return;
                    }

                    // Keyboard navigation
                    let max_idx = filtered.len().saturating_sub(1);
                    let (up, down) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::ArrowUp),
                            i.key_pressed(egui::Key::ArrowDown),
                        )
                    });
                    if up && selection > 0 {
                        result = Some(AppAction::SetModelPickerSelection(selection - 1));
                    }
                    if down && selection < max_idx {
                        result = Some(AppAction::SetModelPickerSelection(selection + 1));
                    }
                    // Enter selects
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        if let Some(m) = filtered.get(selection) {
                            result = Some(AppAction::SelectModel(m.id.clone()));
                        }
                    }

                    // Model count
                    ui.label(
                        egui::RichText::new(format!(
                            "{} model{}",
                            filtered.len(),
                            if filtered.len() == 1 { "" } else { "s" }
                        ))
                        .color(theme.text_dim())
                        .size(11.0),
                    );
                    ui.add_space(4.0);

                    // Scrollable model list
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // Group by provider
                            let mut current_provider: Option<&str> = None;

                            for (i, m) in filtered.iter().enumerate() {
                                // Provider header
                                if current_provider != Some(&m.provider) {
                                    current_provider = Some(&m.provider);
                                    if i > 0 {
                                        ui.add_space(8.0);
                                    }
                                    ui.label(
                                        egui::RichText::new(m.provider.to_uppercase())
                                            .color(theme.text_dim())
                                            .strong()
                                            .size(10.0),
                                    );
                                    ui.add_space(2.0);
                                }

                                let is_selected = i == selection;
                                let bg = if is_selected {
                                    theme.bg_surface2()
                                } else {
                                    egui::Color32::TRANSPARENT
                                };

                                egui::Frame::new()
                                    .fill(bg)
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(8, 4))
                                    .show(ui, |ui| {
                                        let resp = ui
                                            .horizontal(|ui| {
                                                // Model name
                                                let name = if m.display_name.is_empty() {
                                                    &m.id
                                                } else {
                                                    &m.display_name
                                                };
                                                ui.label(
                                                    egui::RichText::new(name)
                                                        .color(if is_selected {
                                                            theme.primary()
                                                        } else {
                                                            theme.text_primary()
                                                        })
                                                        .size(13.0),
                                                );

                                                // Model ID (if different from display name)
                                                if !m.display_name.is_empty()
                                                    && m.display_name != m.id
                                                {
                                                    ui.label(
                                                        egui::RichText::new(&m.id)
                                                            .color(theme.text_dim())
                                                            .size(11.0),
                                                    );
                                                }

                                                // Context window
                                                if m.context_window > 0 {
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            let ctx_label = if m.context_window
                                                                >= 1_000_000
                                                            {
                                                                format!(
                                                                    "{}M ctx",
                                                                    m.context_window / 1_000_000
                                                                )
                                                            } else {
                                                                format!(
                                                                    "{}K ctx",
                                                                    m.context_window / 1_000
                                                                )
                                                            };
                                                            ui.label(
                                                                egui::RichText::new(ctx_label)
                                                                    .color(theme.text_dim())
                                                                    .size(10.0),
                                                            );
                                                        },
                                                    );
                                                }
                                            })
                                            .response;

                                        if resp.interact(egui::Sense::click()).clicked() {
                                            result = Some(AppAction::SelectModel(m.id.clone()));
                                        }

                                        // Hover changes selection
                                        if resp.interact(egui::Sense::hover()).hovered()
                                            && !is_selected
                                        {
                                            result = Some(AppAction::SetModelPickerSelection(i));
                                        }
                                    });
                            }

                            // Custom providers section
                            if !custom_providers.is_empty() {
                                ui.add_space(12.0);
                                ui.label(
                                    egui::RichText::new(
                                        "CUSTOM PROVIDERS (type model ID manually)",
                                    )
                                    .color(theme.text_dim())
                                    .strong()
                                    .size(10.0),
                                );
                                ui.add_space(2.0);
                                for p in custom_providers {
                                    ui.label(
                                        egui::RichText::new(format!("  • {p}"))
                                            .color(theme.text_muted())
                                            .size(12.0),
                                    );
                                }
                            }
                        });
                });
        });

    result
}
