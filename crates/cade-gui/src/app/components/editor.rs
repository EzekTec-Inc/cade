use crate::theme::EguiThemeExt;
use eframe::egui;
use crate::app::AppAction;
use crate::session::SessionState;
use std::rc::Rc;
use std::cell::RefCell;

pub fn render(
    ui: &mut egui::Ui,
    mut input_edit: String,
    has_agent: bool,
    is_streaming: bool,
    request_focus_input: bool,
    input_id: egui::Id,
    session: &Rc<RefCell<Option<SessionState>>>,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut action: Option<AppAction> = None;
    let can_edit = has_agent && !is_streaming;

    egui::Panel::bottom("input_bar")
        .min_size(36.0)
        .show_inside(ui, |ui| {
            // ── Top separator ─────────────────────────
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

            // ── Input row ─────────────────────────────
            egui::Frame::new()
                .fill(theme.bg_input())
                .inner_margin(egui::Margin::symmetric(4, 3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Mode badge — mirrors TUI's input_mode_badge
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

                        // Prompt prefix "> "
                        ui.label(
                            egui::RichText::new("> ")
                                .color(theme.text_dim())
                                .monospace()
                                .size(14.0),
                        );

                        // Text input — multiline, full width
                        let hint = if !has_agent {
                            "Select an agent first…"
                        } else if is_streaming {
                            "Waiting for response…"
                        } else {
                            "Type a message or paste code…"
                        };

                        let desired_w = ui.available_width() - 40.0;
                        let resp = ui.add_enabled(
                            can_edit,
                            egui::TextEdit::multiline(&mut input_edit)
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
                            if let Some(SessionState::Connected {
                                input_buffer: buf, ..
                            }) = session.borrow_mut().as_mut()
                            {
                                *buf = input_edit.clone();
                            }
                        }

                        // Enter sends (Shift+Enter for newline in multiline)
                        let enter_pressed = ui.input(|i| {
                            i.key_pressed(egui::Key::Enter)
                                && !i.modifiers.shift
                        }) && resp.has_focus();
                        let send_enabled =
                            can_edit && !input_edit.trim().is_empty();

                        if is_streaming {
                            ui.spinner();
                        } else {
                            // Send button — square, TUI-matched
                            let send_btn = egui::Button::new(
                                egui::RichText::new("↑")
                                    .color(if send_enabled {
                                        theme.bg_base()
                                    } else {
                                        theme.text_dim()
                                    })
                                    .strong()
                                    .monospace()
                                    .size(14.0),
                            )
                            .fill(if send_enabled {
                                theme.primary()
                            } else {
                                theme.bg_surface2()
                            })
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(egui::CornerRadius::ZERO)
                            .min_size(egui::vec2(24.0, 24.0));

                            if ui
                                .add_enabled(send_enabled, send_btn)
                                .on_hover_text("Send (Enter)")
                                .clicked()
                                || (enter_pressed && send_enabled)
                            {
                                action = Some(AppAction::SendMessage);
                            }
                        }
                    });
                });

            // ── Bottom separator ──────────────────────
            let sep_rect = ui.available_rect_before_wrap();
            let sep_rect = egui::Rect::from_min_size(
                sep_rect.min,
                egui::vec2(sep_rect.width(), 1.0),
            );
            ui.painter().rect_filled(sep_rect, 0.0, theme.border_base());
            ui.advance_cursor_after_rect(sep_rect);
        });

    action
}