use crate::session::SessionState;
use crate::theme::EguiThemeExt;
use eframe::egui;

pub fn render(
    ui: &mut egui::Ui,
    session_snapshot: &Option<SessionState>,
    theme: &crate::theme::ThemeColors,
    viewport: crate::responsive::Viewport,
) -> bool {
    let mut toggle_sidebar = false;

    // Height increases slightly on touch devices
    let height = if viewport.is_desktop() { 24.0 } else { 32.0 };

    egui::Panel::top("cade_toolbar")
        .exact_size(height)
        .frame(
            egui::Frame::new()
                .fill(theme.bg_surface0())
                .inner_margin(egui::Margin::symmetric(6, 0)),
        )
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if !viewport.is_desktop() {
                    let btn = egui::Button::new(
                        egui::RichText::new("☰")
                            .size(16.0)
                            .color(theme.text_primary()),
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .frame(false);
                    if ui.add(btn).clicked() {
                        toggle_sidebar = true;
                    }
                    ui.add_space(4.0);
                }

                ui.label(
                    egui::RichText::new("CADE")
                        .strong()
                        .size(13.0)
                        .color(theme.primary()),
                );

                if let Some(SessionState::Connected { last_usage, .. }) = session_snapshot {
                    if let Some((_, _, Some(ref model))) = *last_usage {
                        ui.add_space(4.0);
                        egui::Frame::new()
                            .fill(theme.bg_surface1())
                            .corner_radius(egui::CornerRadius::ZERO)
                            .inner_margin(egui::Margin::symmetric(4, 1))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(model.as_str())
                                        .monospace()
                                        .size(10.0)
                                        .color(theme.text_muted()),
                                );
                            });
                    }
                }

                if let Some(SessionState::Connected {
                    streaming, health, ..
                }) = session_snapshot
                {
                    let version = health.version.as_deref().unwrap_or("unknown");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("v{version}"))
                                .small()
                                .color(theme.text_dim()),
                        );
                        ui.add_space(4.0);
                        let dot_color = crate::app::status_dot_color(*streaming, theme);
                        let (resp, painter) =
                            ui.allocate_painter(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        painter.circle_filled(resp.rect.center(), 5.0, dot_color);
                    });
                }
            });
        });

    toggle_sidebar
}
