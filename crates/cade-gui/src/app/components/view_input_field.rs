use crate::theme::EguiThemeExt;
use eframe::egui;
use crate::app::AppAction;

pub fn render(
    ctx: &egui::Context,
    has_agent: bool,
    is_streaming: bool,
    input_buffer: &mut String,
    input_id: egui::Id,
    request_focus_input: bool,
    action: &mut AppAction,
    theme: &crate::theme::ThemeColors, 
) {
    egui::TopBottomPanel::bottom("input_bar")
        .min_size(48.0)
        .show(ctx, |ui| {
            let can_edit = has_agent && !is_streaming;

            let sep_color = if is_streaming {
                theme.primary()
            } else {
                theme.border_base()
            };
            let sep_rect = ui.available_rect_before_wrap();
            let sep_rect = egui::Rect::from_min_size(
                sep_rect.min,
                egui::vec2(sep_rect.width(), 1.0),
            );
            ui.painter().rect_filled(sep_rect, 0.0, sep_color);
            ui.advance_cursor_after_rect(sep_rect);

            egui::Frame::new()
                .fill(theme.bg_input())
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let badge_text = if is_streaming {
                            " WAIT "
                        } else if !has_agent {
                            " ··· "
                        } else {
                            " CHAT "
                        };
                        let badge_bg = if is_streaming {
                            theme.warning()
                        } else {
                            theme.bg_surface2()
                        };
                        let badge = egui::RichText::new(badge_text)
                            .color(theme.text_primary())
                            .strong()
                            .size(11.0)
                            .background_color(badge_bg);
                        ui.label(badge);

                        ui.label(
                            egui::RichText::new("> ")
                                .color(theme.text_dim())
                                .monospace()
                                .size(14.0),
                        );

                        let hint = if !has_agent {
                            "Select an agent first…"
                        } else if is_streaming {
                            "Waiting for response…"
                        } else {
                            "Type a message or paste code…"
                        };

                        let desired_w = ui.available_width() - 40.0;
                        let mut temp_input = input_buffer.clone();
                        let resp = ui.add_enabled(
                            can_edit,
                            egui::TextEdit::multiline(&mut temp_input)
                                .id(input_id)
                                .hint_text(
                                    egui::RichText::new(hint)
                                        .color(theme.text_dim()),
                                )
                                .desired_width(desired_w)
                                .desired_rows(1)
                                .lock_focus(true)
                                .font(egui::TextStyle::Monospace),
                        );

                        if request_focus_input {
                            resp.request_focus();
                        }

                        if resp.changed() {
                            *input_buffer = temp_input.clone();
                        }

                        let enter_pressed = ui.input(|i| {
                            i.key_pressed(egui::Key::Enter)
                                && !i.modifiers.shift
                        }) && resp.has_focus();
                        let send_enabled =
                            can_edit && !temp_input.trim().is_empty();

                        if is_streaming {
                            ui.spinner();
                        } else {
                            let send_btn = egui::Button::new(
                                egui::RichText::new("↑")
                                    .color(if send_enabled {
                                        theme.bg_base()
                                    } else {
                                        theme.text_dim()
                                    })
                                    .strong()
                                    .size(15.0),
                            )
                            .fill(if send_enabled {
                                theme.primary()
                            } else {
                                theme.bg_surface2()
                            })
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(egui::CornerRadius::same(14))
                            .min_size(egui::vec2(28.0, 28.0));

                            if ui
                                .add_enabled(send_enabled, send_btn)
                                .on_hover_text("Send message (Enter)")
                                .clicked()
                                || (enter_pressed && send_enabled)
                            {
                                *action = AppAction::SendMessage;
                            }
                        }
                    });
                });
        });
}
