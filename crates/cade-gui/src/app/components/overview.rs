use eframe::egui;

pub fn render(
    ui: &mut egui::Ui,
    profile_name: &mut String,
    profile_email: &mut String,
    _session: &crate::session::ConnectedSession,
    _theme: &crate::theme::ThemeColors,
) {
    let frame = egui::Frame::NONE
        .fill(egui::Color32::from_rgb(28, 28, 28)) // #1C1C1C
        .inner_margin(egui::Margin::symmetric(40, 32));

    egui::CentralPanel::default().frame(frame).show_inside(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_max_width(800.0); // Constrain width like a web dashboard

            // Header
            ui.heading(
                egui::RichText::new("Account overview")
                    .color(egui::Color32::from_gray(230))
                    .size(16.0)
                    .strong()
            );
            ui.add_space(16.0);

            // Profile Section
            ui.label(
                egui::RichText::new("Profile")
                    .color(egui::Color32::from_gray(230))
                    .size(20.0)
            );
            ui.add_space(16.0);

            let label_color = egui::Color32::from_gray(160);

            // Name
            ui.label(egui::RichText::new("Name").color(label_color).size(12.0));
            ui.add_space(4.0);
            ui.add_sized(
                [ui.available_width(), 36.0],
                egui::TextEdit::singleline(profile_name).margin(egui::Margin::symmetric(12, 8))
            );
            ui.add_space(16.0);

            // Email
            ui.label(egui::RichText::new("Email").color(label_color).size(12.0));
            ui.add_space(4.0);
            ui.add_sized(
                [ui.available_width(), 36.0],
                egui::TextEdit::singleline(profile_email).margin(egui::Margin::symmetric(12, 8))
            );
            ui.add_space(16.0);

            // Save Profile Button
            let save_btn = egui::Button::new(
                egui::RichText::new("Save profile").color(egui::Color32::from_gray(220))
            )
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(80)));
            if ui.add_sized([100.0, 32.0], save_btn).clicked() {
                crate::storage::save(crate::storage::StorageKey::ProfileName, profile_name);
                crate::storage::save(crate::storage::StorageKey::ProfileEmail, profile_email);
            }

            ui.add_space(32.0);
            ui.separator();
            ui.add_space(32.0);

            // Current Plan Section
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Current plan").color(egui::Color32::from_gray(230)).size(16.0));
                
                egui::Frame::NONE
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(100)))
                    .corner_radius(egui::CornerRadius::same(2))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Free").color(egui::Color32::from_gray(180)).size(11.0));
                    });
            });
            ui.add_space(16.0);

            let check_color = egui::Color32::from_rgb(60, 180, 100);
            let features = [
                "3 stateful agents",
                "Connect your own LLM API keys (BYOK)",
                "Chat with agents in the ADE",
                "Run your agents locally with CADE Code",
                "OAuth only",
            ];

            for feature in features {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("✔").color(check_color));
                    ui.label(egui::RichText::new(feature).color(egui::Color32::from_gray(200)));
                });
                ui.add_space(8.0);
            }
            ui.add_space(8.0);

            let upgrade_btn = egui::Button::new(
                egui::RichText::new("Upgrade plan").color(egui::Color32::from_gray(220))
            )
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(80)));
            if ui.add_sized([110.0, 32.0], upgrade_btn).clicked() { }

            ui.add_space(32.0);
            ui.separator();
            ui.add_space(32.0);

            // Onboarding Status Section
            ui.label(egui::RichText::new("Onboarding status").color(egui::Color32::from_gray(230)).size(16.0));
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Press the button below to do onboarding again!").color(egui::Color32::from_gray(180)));
            ui.add_space(16.0);
            
            let retry_btn = egui::Button::new(
                egui::RichText::new("Retry onboarding").color(egui::Color32::from_gray(220))
            )
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(80)));
            if ui.add_sized([130.0, 32.0], retry_btn).clicked() { }

            ui.add_space(32.0);
            ui.separator();
            ui.add_space(32.0);

            // Connected Applications Section
            ui.label(egui::RichText::new("Connected Applications").color(egui::Color32::from_gray(230)).size(16.0));
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Manage any external connections and their access to your account.").color(egui::Color32::from_gray(180)));
            ui.add_space(16.0);

            // App Card
            egui::Frame::NONE
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(egui::RichText::new("CADE Desktop App").color(egui::Color32::from_gray(230)).size(16.0));
                    ui.add_space(16.0);
                    
                    ui.horizontal(|ui| {
                        // Connection badge
                        egui::Frame::NONE
                            .fill(egui::Color32::from_rgb(30, 80, 50))
                            .corner_radius(egui::CornerRadius::same(2))
                            .inner_margin(egui::Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                ui.label(egui::RichText::new("1 connection").color(egui::Color32::from_rgb(100, 200, 120)).size(10.0));
                            });
                        
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Last connected: 5/22/2026").color(egui::Color32::from_gray(140)).size(12.0));
                        
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let view_btn = egui::Button::new(
                                egui::RichText::new("View connections").color(egui::Color32::from_gray(220))
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(80)));
                            if ui.add(view_btn).clicked() { }
                        });
                    });
                });

            ui.add_space(16.0);

            // Pagination
            ui.horizontal(|ui| {
                let btn_prev = egui::Button::new(egui::RichText::new("Previous").color(egui::Color32::from_gray(220)))
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(80)));
                if ui.add(btn_prev).clicked() { }
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let btn_next = egui::Button::new(egui::RichText::new("Next").color(egui::Color32::from_gray(220)))
                        .fill(egui::Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(80)));
                    if ui.add(btn_next).clicked() { }

                    ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                        ui.label(egui::RichText::new("Page 1 of 1").color(egui::Color32::from_gray(140)));
                    });
                });
            });

            ui.add_space(32.0);
        });
    });
}
