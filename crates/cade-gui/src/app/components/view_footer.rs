use eframe::egui;
use crate::session::SessionState;

pub fn render(ctx: &egui::Context, session_snapshot: &Option<SessionState>) {
    if let Some(SessionState::Connected {
        streaming,
        last_usage,
        ..
    }) = session_snapshot
    {
        egui::TopBottomPanel::bottom("cade_status_bar")
            .exact_size(18.0)
            .frame(egui::Frame::new().fill(crate::theme::BG_SURFACE0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some((input_tokens, output_tokens, _)) = *last_usage {
                        ui.label(
                            egui::RichText::new(format!("{input_tokens}in {output_tokens}out"))
                                .size(10.0)
                                .color(crate::theme::TEXT_DIM),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if *streaming {
                            ui.label(
                                egui::RichText::new("streaming…")
                                    .size(10.0)
                                    .color(crate::theme::WARNING),
                            );
                        }
                    });
                });
            });
    }
}
