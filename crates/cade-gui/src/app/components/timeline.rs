use crate::theme::EguiThemeExt;
use eframe::egui;
use crate::app::AppAction;
use egui_commonmark::CommonMarkCache;

#[allow(clippy::too_many_arguments)]
pub fn render(
    ui: &mut egui::Ui,
    md_cache: &mut CommonMarkCache,
    selected_agent: Option<usize>,
    messages: &[cade_api_types::ChatMessage],
    has_more_messages: bool,
    is_streaming: bool,
    auto_scroll: bool,
    error_toast: Option<&String>,
    last_usage: Option<&(u64, u64, Option<String>)>,
    last_finish_reason: Option<&String>,
    live_outputs: &[crate::session::LiveOutputBlock],
    subagent_cards: &[crate::session::SubagentCardState],
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut action = None;

    egui::CentralPanel::default().show_inside(ui, |ui| {
        let footer_h = if last_usage.is_some() { 22.0 } else { 0.0 };
        let reserved = footer_h + 4.0;
        let avail_h = (ui.available_height() - reserved).max(60.0);

        egui::ScrollArea::vertical()
            .id_salt("timeline_scroll")
            .stick_to_bottom(auto_scroll)
            .max_height(avail_h)
            .show(ui, |ui| {
                let pad = 4.0;
                ui.add_space(2.0);

                if selected_agent.is_none() {
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        ui.vertical(|ui| {
                            crate::app::views::render_welcome(ui, md_cache, theme);
                        });
                    });
                } else if messages.is_empty() && !is_streaming {
                    ui.add_space(24.0);
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        ui.label(
                            egui::RichText::new("No messages yet. Send one to start a conversation.")
                                .color(theme.text_muted())
                                .italics()
                                .size(12.0),
                        );
                    });
                } else {
                    if has_more_messages {
                        ui.horizontal(|ui| {
                            ui.add_space(pad);
                            if ui.add(
                                egui::Button::new(
                                    egui::RichText::new("⬆  Load older messages")
                                        .color(theme.text_muted())
                                        .size(11.0),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .stroke(egui::Stroke::new(1.0, theme.border_base()))
                            ).clicked() {
                                action = Some(AppAction::LoadMore);
                            }
                        });
                        ui.add(egui::Separator::default().horizontal().spacing(4.0));
                    }

                    for (i, msg) in messages.iter().enumerate() {
                        if i > 0 {
                            ui.horizontal(|ui| {
                                ui.add_space(pad);
                                ui.add(egui::Separator::default().horizontal().spacing(2.0));
                            });
                        }
                        ui.horizontal(|ui| {
                            ui.add_space(pad);
                            ui.vertical(|ui| {
                                if let Some(a) = crate::app::views::render_timeline_message(
                                    ui,
                                    md_cache,
                                    msg,
                                    theme,
                                ) {
                                    action = Some(a);
                                }
                            });
                        });
                    }

                    if is_streaming {
                        // Render any active live-output blocks
                        for block in live_outputs.iter().filter(|b| !b.done) {
                            ui.horizontal(|ui| {
                                ui.add_space(pad);
                                ui.vertical(|ui| {
                                    crate::app::views::render_live_output(ui, block, theme);
                                });
                            });
                        }

                        // Render subagent progress cards
                        for card_state in subagent_cards {
                            let card = crate::app::views::SubagentCard {
                                subagent_id: card_state.subagent_id.clone(),
                                task: card_state.task.clone(),
                                mode: card_state.mode.clone(),
                                model: card_state.model.clone(),
                                status: match card_state.status.as_str() {
                                    "complete" => crate::app::views::SubagentStatus::Complete,
                                    "error" => crate::app::views::SubagentStatus::Error,
                                    _ => crate::app::views::SubagentStatus::Running,
                                },
                                elapsed_secs: card_state.elapsed_secs,
                                tool_calls: card_state.tool_calls,
                                output_lines: card_state.output_lines,
                                result_preview: card_state.result_preview.clone(),
                                is_error: card_state.is_error,
                            };
                            ui.horizontal(|ui| {
                                ui.add_space(pad);
                                ui.vertical(|ui| {
                                    crate::app::views::render_subagent_card(ui, &card, theme);
                                });
                            });
                            ui.add_space(2.0);
                        }

                        ui.horizontal(|ui| {
                            ui.add_space(pad);
                            ui.add(egui::Separator::default().horizontal().spacing(1.0));
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(pad);
                            ui.label(
                                egui::RichText::new("▍ CADE")
                                    .color(theme.primary())
                                    .strong()
                                    .size(13.0),
                            );
                            ui.add_space(4.0);
                            ui.spinner();
                        });
                    }
                    ui.add_space(4.0);
                }
            });

        let scroll_id = egui::Id::new("timeline_scroll");
        let mem = ui.ctx().memory(|m| {
            m.data.get_temp::<egui::scroll_area::State>(scroll_id)
        });
        if let Some(st) = mem {
            if st.velocity().y < -5.0 && auto_scroll {
                action = Some(AppAction::DisableAutoScroll);
            }
        }

        if !auto_scroll {
            let panel_rect = ui.max_rect();
            let btn_size = egui::vec2(32.0, 32.0);
            let btn_pos = egui::pos2(
                panel_rect.right() - btn_size.x - 10.0,
                panel_rect.bottom() - reserved - btn_size.y - 8.0,
            );
            let btn_rect = egui::Rect::from_min_size(btn_pos, btn_size);
            let resp = ui.interact(
                btn_rect,
                egui::Id::new("scroll_to_bottom_btn"),
                egui::Sense::click(),
            );
            let bg = if resp.hovered() {
                theme.bg_surface2()
            } else {
                theme.bg_surface1()
            };
            ui.painter().rect_filled(
                btn_rect,
                egui::CornerRadius::ZERO,
                bg,
            );
            ui.painter().rect_stroke(
                btn_rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0, theme.border_base()),
                egui::StrokeKind::Outside,
            );
            ui.painter().text(
                btn_rect.center(),
                egui::Align2::CENTER_CENTER,
                "↓",
                egui::FontId::proportional(16.0),
                theme.text_primary(),
            );
            if resp.clicked() {
                action = Some(AppAction::ScrollToBottom);
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }

        if let Some((inp, out, model)) = last_usage {
            ui.add_space(2.0);
            let model_str = model
                .as_ref()
                .map(|m| format!(" · {m}"))
                .unwrap_or_default();
            let finish = last_finish_reason
                .as_ref()
                .map(|r| format!(" · {r}"))
                .unwrap_or_default();
            ui.label(
                egui::RichText::new(format!(
                    "↑{inp} ↓{out} tokens{model_str}{finish}"
                ))
                .color(theme.text_dim())
                .size(11.0),
            );
        }

        if let Some(err) = error_toast {
            let screen = ui.ctx().content_rect();
            let toast_width = 300.0;
            let toast_height = 42.0;
            let pos = egui::pos2(
                screen.right() - toast_width - 16.0,
                screen.bottom() - toast_height - 64.0, // above the input bar
            );

            egui::Window::new("error_toast_window")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .fixed_pos(pos)
                .frame(
                    egui::Frame::new()
                        .fill(theme.error().linear_multiply(0.12))
                        .stroke(egui::Stroke::new(1.0, theme.error()))
                        .corner_radius(egui::CornerRadius::ZERO)
                        .inner_margin(egui::Margin::symmetric(8, 6)),
                )
                .show(ui.ctx(), |ui| {
                    ui.set_width(toast_width);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("⚠")
                                .color(theme.error())
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new(err.as_str())
                                .color(theme.text_primary())
                                .size(13.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                action = Some(AppAction::DismissError);
                            }
                        });
                    });
                });
        }
    });

    action
}
