use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;

pub fn render_profiles_overlay(
    ctx: &egui::Context,
    profiles: &[(String, String, String)],
    edit_name: &str,
    edit_url: &str,
    edit_token: &str,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let _screen = ctx.content_rect();
    let w = 400.0;
    let h = 400.0;

    let rect = crate::responsive::overlay_rect(ctx, w, h, None);
    let pos = rect.min;

    let mut open = true;
    egui::Window::new("Environment Profiles")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .open(&mut open)
        .fixed_pos(pos)
        .fixed_size([w, h])
        .frame(
            egui::Frame::NONE
                .fill(theme.bg_surface1())
                .stroke(egui::Stroke::new(1.0, theme.border_base()))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(12.0),
        )
        .show(ctx, |ui| {
            ui.set_width(w - 24.0);

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("🌍 Environment Profiles")
                        .color(theme.primary())
                        .strong()
                        .size(16.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").clicked() {
                        result = Some(AppAction::ToggleProfilesOverlay);
                    }
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            if profiles.is_empty() {
                ui.label(
                    egui::RichText::new("No saved profiles.")
                        .color(theme.text_muted())
                        .italics(),
                );
            } else {
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        for (idx, (name, url, token)) in profiles.iter().enumerate() {
                            egui::Frame::NONE
                                .fill(theme.bg_surface2())
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(egui::RichText::new(name).strong());
                                            ui.label(
                                                egui::RichText::new(url)
                                                    .small()
                                                    .color(theme.text_muted()),
                                            );
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.button("Connect").clicked() {
                                                    result = Some(AppAction::ConnectProfile(
                                                        url.clone(),
                                                        token.clone(),
                                                    ));
                                                }
                                                if ui.button("Delete").clicked() {
                                                    result = Some(AppAction::DeleteProfile(idx));
                                                }
                                            },
                                        );
                                    });
                                });
                            ui.add_space(4.0);
                        }
                    });
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(egui::RichText::new("Add New Profile").strong());
            ui.add_space(4.0);

            let mut n = edit_name.to_string();
            let mut u = edit_url.to_string();
            let mut t = edit_token.to_string();

            ui.horizontal(|ui| {
                ui.label("Name:  ");
                if ui
                    .add(egui::TextEdit::singleline(&mut n).desired_width(ui.available_width()))
                    .changed()
                {
                    result = Some(AppAction::SetProfileEdit(n.clone(), u.clone(), t.clone()));
                }
            });
            ui.horizontal(|ui| {
                ui.label("URL:   ");
                if ui
                    .add(egui::TextEdit::singleline(&mut u).desired_width(ui.available_width()))
                    .changed()
                {
                    result = Some(AppAction::SetProfileEdit(n.clone(), u.clone(), t.clone()));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Token: ");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut t)
                            .password(true)
                            .desired_width(ui.available_width()),
                    )
                    .changed()
                {
                    result = Some(AppAction::SetProfileEdit(n.clone(), u.clone(), t.clone()));
                }
            });

            ui.add_space(8.0);
            if ui.button("Save Profile").clicked() {
                if !n.is_empty() && !u.is_empty() {
                    result = Some(AppAction::SaveProfile(n, u, t));
                }
            }
        });

    if !open && result.is_none() {
        result = Some(AppAction::ToggleProfilesOverlay);
    }

    result
}
