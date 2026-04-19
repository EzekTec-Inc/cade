use crate::theme::EguiThemeExt;
use eframe::egui;
use crate::app::AppAction;
use crate::session::SessionState;
use egui_commonmark::CommonMarkCache;

pub fn render(
    ctx: &egui::Context,
    md_cache: &mut CommonMarkCache,
    session_snapshot: &Option<SessionState>,
    action: &mut AppAction,
    theme: &crate::theme::ThemeColors, 
) {
    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(crate::session::SessionState::Connected {
            total_input_tokens,
            total_output_tokens,
            ..
        }) = session_snapshot
        {
            const DEFAULT_WINDOW: u64 = 128_000;
            let total = total_input_tokens + total_output_tokens;
            let frac = crate::theme::context_fill_fraction(total, DEFAULT_WINDOW);
            if total > 0 {
                let bar_color = crate::theme::context_fill_color(frac);
                let hover_text = format!(
                    "{} / {} tokens ({:.0}%)",
                    total, DEFAULT_WINDOW,
                    frac * 100.0
                );
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_height(4.0)
                        .fill(bar_color),
                )
                .on_hover_text(hover_text);
            }
        }

        match session_snapshot {
            Some(SessionState::Connecting { .. }) => {
                ui.label("Connecting to server...");
                ui.spinner();
            }
            Some(SessionState::HealthOk { .. }) => {
                ui.label("Server reached — loading agents...");
                ui.spinner();
            }
            Some(SessionState::Connected {
                selected_agent,
                messages,
                error_toast,
                has_more_messages,
                auto_scroll,
                streaming,
                last_usage,
                last_finish_reason,
                ..
            }) => {
                let has_agent = selected_agent.is_some();
                let is_streaming = *streaming;

                egui::Frame::NONE
                    .fill(theme.bg_base())
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        if !has_agent {
                            crate::app::views::render_welcome(ui, md_cache, theme);
                        } else {
                            let footer_h = if last_usage.is_some() { 22.0 } else { 0.0 };
                            let toast_h = if error_toast.is_some() { 42.0 } else { 0.0 };
                            let reserved = footer_h + toast_h + 4.0;
                            let avail_h = (ui.available_height() - reserved).max(60.0);

                            let mut scroll_area = egui::ScrollArea::vertical()
                                .id_salt("timeline_scroll")
                                .auto_shrink([false; 2])
                                .max_height(avail_h);

                            if *auto_scroll {
                                scroll_area = scroll_area.stick_to_bottom(true);
                            }

                            let scroll_output = scroll_area.show(ui, |ui| {
                                let pad = 16.0;
                                ui.add_space(4.0);

                                if messages.is_empty() && !is_streaming {
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
                                    if *has_more_messages {
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
                                                *action = AppAction::LoadMore;
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
                                                    *action = a;
                                                }
                                            });
                                        });
                                    }

                                    if is_streaming {
                                        ui.horizontal(|ui| {
                                            ui.add_space(pad);
                                            ui.add(egui::Separator::default().horizontal().spacing(2.0));
                                        });
                                        ui.horizontal(|ui| {
                                            ui.add_space(pad);
                                            ui.label(
                                                egui::RichText::new("▍ CADE")
                                                    .color(theme.primary())
                                                    .strong()
                                                    .size(13.0),
                                            );
                                            ui.add_space(6.0);
                                            ui.spinner();
                                        });
                                    }
                                    ui.add_space(8.0);
                                }
                            });

                            let scroll_id = egui::Id::new("timeline_scroll");
                            let mem = ui.ctx().memory(|m| {
                                m.data.get_temp::<egui::scroll_area::State>(scroll_id)
                            });
                            if let Some(st) = mem {
                                if st.velocity().y < -5.0 && *auto_scroll {
                                    *action = AppAction::DisableAutoScroll;
                                }
                            }

                            if !*auto_scroll {
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
                                    egui::CornerRadius::same(16),
                                    bg,
                                );
                                ui.painter().rect_stroke(
                                    btn_rect,
                                    egui::CornerRadius::same(16),
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
                                    *action = AppAction::ScrollToBottom;
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
                                        "{inp} tokens in · {out} tokens out{model_str}{finish}"
                                    ))
                                    .size(10.0)
                                    .color(theme.text_dim()),
                                );
                            }

                            if let Some(e) = error_toast {
                                ui.add_space(8.0);
                                egui::Frame::new()
                                    .fill(theme.error().linear_multiply(0.1))
                                    .stroke(egui::Stroke::new(1.0, theme.error()))
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new("⚠")
                                                    .color(theme.error()),
                                            );
                                            ui.label(
                                                egui::RichText::new(e).color(theme.error()),
                                            );
                                        });
                                    });
                            }
                        }
                    });
            }
            _ => {
                ui.label("Disconnected.");
            }
        }
    });
}
