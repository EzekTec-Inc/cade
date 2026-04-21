use crate::theme::EguiThemeExt;
use eframe::egui;
use crate::session::SessionState;

pub fn render(ui: &mut egui::Ui, session_snapshot: &Option<SessionState>,
    theme: &crate::theme::ThemeColors,
) {
    if let Some(SessionState::Connected {
        streaming,
        last_usage,
        ..
    }) = session_snapshot
    {
        egui::Panel::bottom("cade_status_bar")
            .exact_size(18.0)
            .frame(egui::Frame::new().fill(theme.bg_surface0()))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    if let Some((input_tokens, output_tokens, _)) = *last_usage {
                        ui.label(
                            egui::RichText::new(format!("{input_tokens}in {output_tokens}out"))
                                .size(10.0)
                                .color(theme.text_dim()),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if *streaming {
                            ui.label(
                                egui::RichText::new("streaming…")
                                    .size(10.0)
                                    .color(theme.warning()),
                            );
                        }
                    });
                });
            });
    }
}
