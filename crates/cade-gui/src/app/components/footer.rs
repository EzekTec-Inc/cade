use crate::session::SessionState;
use crate::theme::EguiThemeExt;
use eframe::egui;

pub fn render(
    ui: &mut egui::Ui,
    session_snapshot: &Option<SessionState>,
    theme: &crate::theme::ThemeColors,
) {
    if let Some(SessionState::Connected {
        streaming,
        last_usage,
        total_input_tokens,
        total_output_tokens,
        ..
    }) = session_snapshot
    {
        egui::Panel::bottom("cade_status_bar")
            .exact_size(18.0)
            .frame(egui::Frame::new().fill(theme.bg_surface0()))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    // Left: mode indicator
                    if *streaming {
                        ui.label(
                            egui::RichText::new(" ● STREAMING ")
                                .color(theme.warning())
                                .monospace()
                                .strong()
                                .size(10.0),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(" ● IDLE ")
                                .color(theme.success())
                                .monospace()
                                .size(10.0),
                        );
                    }

                    // Turn usage (if available)
                    if let Some((in_tok, out_tok, ref model)) = *last_usage {
                        ui.label(
                            egui::RichText::new(format!("↑{in_tok} ↓{out_tok}"))
                                .color(theme.text_dim())
                                .monospace()
                                .size(10.0),
                        );
                        if let Some(m) = model {
                            ui.label(
                                egui::RichText::new(m.as_str())
                                    .color(theme.text_muted())
                                    .monospace()
                                    .size(10.0),
                            );
                        }
                    }

                    // Right-aligned: cumulative tokens + context %
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let total = total_input_tokens + total_output_tokens;
                        if total > 0 {
                            let pct = (total as f64 / 128_000.0 * 100.0).min(100.0);
                            let pct_color = if pct >= 90.0 {
                                theme.error()
                            } else if pct >= 60.0 {
                                theme.warning()
                            } else {
                                theme.text_muted()
                            };
                            ui.label(
                                egui::RichText::new(format!("{:.0}%", pct))
                                    .color(pct_color)
                                    .monospace()
                                    .size(10.0),
                            );
                            ui.label(
                                egui::RichText::new(format!("{}tok", format_tok(total)))
                                    .color(theme.text_dim())
                                    .monospace()
                                    .size(10.0),
                            );
                        }
                    });
                });
            });
    }
}

/// Format token count in compact form: 1234 → "1.2k", 123456 → "123k"
fn format_tok(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{}k", n / 1_000)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
