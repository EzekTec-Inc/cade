use crate::theme::EguiThemeExt;
use eframe::egui;

pub fn render(
    ui: &mut egui::Ui,
    profile_name: &mut String,
    profile_email: &mut String,
    _session: &crate::session::ConnectedSession,
    theme: &crate::theme::ThemeColors,
) {
    let background_frame = egui::Frame::NONE
        .fill(theme.bg_base())
        .inner_margin(egui::Margin::symmetric(32, 24));

    egui::CentralPanel::default()
        .frame(background_frame)
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.set_max_width(900.0); // Constrain width like a beautiful web dashboard

                // ── PAGE TITLE AND SUBTITLE ──────────────────────────────────
                ui.vertical(|ui| {
                    ui.heading(
                        egui::RichText::new("Account overview")
                            .color(theme.text_primary())
                            .size(24.0)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "Manage your personal information, subscription, and connected applications.",
                        )
                        .color(theme.text_muted())
                        .size(12.0),
                    );
                });
                ui.add_space(28.0);

                // Define Card style matching the high-fidelity screenshot
                let card_frame = egui::Frame::NONE
                    .fill(theme.bg_card())
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(24))
                    .stroke(egui::Stroke::new(1.0, theme.border_base()));

                // ── TWO-COLUMN GRID LAYOUT ────────────────────────────────────
                ui.columns(2, |columns| {
                    // ── LEFT COLUMN: PROFILE CARD ─────────────────────────────
                    columns[0].vertical(|ui| {
                        card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());

                            ui.label(
                                egui::RichText::new("Profile")
                                    .color(theme.text_primary())
                                    .size(18.0)
                                    .strong(),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new("Manage your personal information and contact details.")
                                    .color(theme.text_muted())
                                    .size(11.0),
                            );
                            ui.add_space(20.0);

                            // Form field: Name
                            ui.label(
                                egui::RichText::new("Name")
                                    .color(theme.text_primary())
                                    .size(11.0)
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.add(
                                egui::TextEdit::singleline(profile_name)
                                    .desired_width(f32::INFINITY)
                                    .margin(egui::Margin::symmetric(12, 8)),
                            );
                            ui.add_space(16.0);

                            // Form field: Email
                            ui.label(
                                egui::RichText::new("Email")
                                    .color(theme.text_primary())
                                    .size(11.0)
                                    .strong(),
                            );
                            ui.add_space(4.0);
                            ui.add(
                                egui::TextEdit::singleline(profile_email)
                                    .desired_width(f32::INFINITY)
                                    .margin(egui::Margin::symmetric(12, 8)),
                            );
                            ui.add_space(24.0);

                            // Save changes button (aligned to the right)
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let save_btn = egui::Button::new(
                                        egui::RichText::new("Save changes")
                                            .color(theme.bg_base())
                                            .strong(),
                                    )
                                    .fill(theme.primary())
                                    .corner_radius(egui::CornerRadius::same(4));

                                    if ui.add_sized([110.0, 30.0], save_btn).clicked() {
                                        crate::storage::save(crate::storage::StorageKey::ProfileName, profile_name);
                                        crate::storage::save(crate::storage::StorageKey::ProfileEmail, profile_email);
                                    }
                                });
                            });
                        });
                    });

                    // ── RIGHT COLUMN: YOUR PLAN, ONBOARDING, CONNECTED APPS ──
                    columns[1].vertical(|ui| {
                        // Card 1: Your Plan
                        card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());

                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("Your plan")
                                        .color(theme.text_primary())
                                        .size(18.0)
                                        .strong(),
                                );
                                ui.add_space(8.0);
                                egui::Frame::NONE
                                    .fill(theme.tinted_bg(theme.success(), 32))
                                    .corner_radius(egui::CornerRadius::same(2))
                                    .inner_margin(egui::Margin::symmetric(6, 2))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new("Active")
                                                .color(theme.success())
                                                .size(10.0)
                                                .strong(),
                                        );
                                    });
                            });
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new("Free tier")
                                    .color(theme.text_muted())
                                    .size(11.0),
                            );
                            ui.add_space(16.0);

                            // Features list with checkmarks
                            let check_color = theme.success();
                            let features = [
                                "3 stateful agents",
                                "Connect your own LLM API keys (BYOK)",
                                "Chat with agents in the ADE",
                                "Run your agents locally with CADE Code",
                                "OAuth only",
                            ];

                            for feature in features {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("✔").color(check_color).strong());
                                    ui.add_space(4.0);
                                    ui.label(
                                        egui::RichText::new(feature)
                                            .color(theme.text_primary())
                                            .size(11.0),
                                    );
                                });
                                ui.add_space(8.0);
                            }
                            ui.add_space(12.0);

                            // Upgrade button (aligned to the right)
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let upgrade_btn = egui::Button::new(
                                        egui::RichText::new("Upgrade plan")
                                            .color(theme.text_primary())
                                            .strong(),
                                    )
                                    .fill(theme.bg_surface0())
                                    .stroke(egui::Stroke::new(1.0, theme.border_base()))
                                    .corner_radius(egui::CornerRadius::same(4));

                                    if ui.add_sized([110.0, 30.0], upgrade_btn).clicked() {}
                                });
                            });
                        });
                        ui.add_space(20.0);

                        // Card 2: Onboarding Status
                        card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());

                            ui.label(
                                egui::RichText::new("Onboarding status")
                                    .color(theme.text_primary())
                                    .size(16.0)
                                    .strong(),
                            );
                            ui.add_space(12.0);

                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("Completed")
                                        .color(theme.success())
                                        .size(14.0)
                                        .strong(),
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    egui::RichText::new("✔")
                                        .color(theme.success())
                                        .size(14.0)
                                        .strong(),
                                );
                            });
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new("Press the button below to do onboarding again!")
                                    .color(theme.text_muted())
                                    .size(11.0),
                            );
                            ui.add_space(16.0);

                            // Retry button (aligned to the right)
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let retry_btn = egui::Button::new(
                                        egui::RichText::new("Retry onboarding")
                                            .color(theme.text_primary())
                                            .strong(),
                                    )
                                    .fill(theme.bg_surface0())
                                    .stroke(egui::Stroke::new(1.0, theme.border_base()))
                                    .corner_radius(egui::CornerRadius::same(4));

                                    if ui.add_sized([130.0, 30.0], retry_btn).clicked() {}
                                });
                            });
                        });
                        ui.add_space(20.0);

                        // Card 3: Connected Applications
                        card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());

                            ui.label(
                                egui::RichText::new("Connected Applications")
                                    .color(theme.text_primary())
                                    .size(16.0)
                                    .strong(),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new("Manage external connections and access.")
                                    .color(theme.text_muted())
                                    .size(11.0),
                            );
                            ui.add_space(16.0);

                            ui.label(
                                egui::RichText::new("CADE Desktop App")
                                    .color(theme.text_primary())
                                    .size(14.0)
                                    .strong(),
                            );
                            ui.add_space(6.0);

                            ui.horizontal(|ui| {
                                egui::Frame::NONE
                                    .fill(theme.tinted_bg(theme.success(), 32))
                                    .corner_radius(egui::CornerRadius::same(2))
                                    .inner_margin(egui::Margin::symmetric(6, 2))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new("Active")
                                                .color(theme.success())
                                                .size(10.0)
                                                .strong(),
                                        );
                                    });
                                ui.add_space(8.0);
                                ui.label(
                                    egui::RichText::new("Last connected: 5/22/2026")
                                        .color(theme.text_muted())
                                        .size(11.0),
                                );
                            });
                            ui.add_space(16.0);

                            // View connections button (aligned to the right)
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let view_btn = egui::Button::new(
                                        egui::RichText::new("View connections")
                                            .color(theme.text_primary())
                                            .strong(),
                                    )
                                    .fill(theme.bg_surface0())
                                    .stroke(egui::Stroke::new(1.0, theme.border_base()))
                                    .corner_radius(egui::CornerRadius::same(4));

                                    if ui.add_sized([130.0, 30.0], view_btn).clicked() {}
                                });
                            });
                        });
                    });
                });
            });
        });
}
