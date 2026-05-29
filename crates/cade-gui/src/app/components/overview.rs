use eframe::egui;
use crate::theme::EguiThemeExt;

pub fn render(
    ui: &mut egui::Ui,
    profile_name: &mut String,
    profile_email: &mut String,
    session: &crate::session::ConnectedSession,
    theme: &crate::theme::ThemeColors,
) {
    let background_frame = egui::Frame::NONE
        .fill(theme.bg_base())
        .inner_margin(egui::Margin::symmetric(24, 24));

    egui::CentralPanel::default()
        .frame(background_frame)
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // Top Search/Action Row
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading(
                            egui::RichText::new("CADE Command Center")
                                .color(theme.text_primary())
                                .size(24.0)
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new("Real-time agent execution & platform telemetry")
                                .color(theme.text_muted())
                                .size(11.0),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Profile Avatar and Info Card on the right
                        ui.horizontal(|ui| {
                            let initial = profile_name.chars().next().unwrap_or('C').to_string();
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                            ui.painter().circle_filled(
                                rect.center(),
                                12.0,
                                theme.primary(),
                            );
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                initial,
                                egui::FontId::proportional(11.0),
                                theme.bg_base(),
                            );

                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new(&*profile_name)
                                        .color(theme.text_primary())
                                        .size(11.0)
                                        .strong()
                                );
                                ui.label(
                                    egui::RichText::new(&*profile_email)
                                        .color(theme.text_muted())
                                        .size(9.0)
                                );
                            });
                        });

                        ui.add_space(16.0);

                        // Quick-action button
                        let btn = egui::Button::new(
                            egui::RichText::new("⚡ Run Command").color(theme.bg_base()),
                        )
                        .fill(theme.primary())
                        .corner_radius(egui::CornerRadius::same(4));
                        
                        if ui.add(btn).on_hover_text("Open CADE Command Palette (Ctrl+P)").clicked() {
                            // Open command palette
                        }

                        ui.add_space(12.0);

                        // Search box placeholder
                        let mut search_query = "".to_string();
                        ui.add_sized(
                            egui::vec2(220.0, 26.0),
                            egui::TextEdit::singleline(&mut search_query)
                                .hint_text("Search metrics, nodes, or commands...")
                                .margin(egui::Margin::symmetric(8, 4))
                        );
                    });
                });
                ui.add_space(24.0);

                // 3-Column Grid Layout
                ui.columns(3, |columns| {
                    // --- COLUMN 0: METRICS ---
                    let left_col = &mut columns[0];
                    left_col.vertical(|ui| {
                        ui.heading(
                            egui::RichText::new("Platform Metrics")
                                .color(theme.text_primary())
                                .size(15.0)
                                .strong(),
                        );
                        ui.add_space(12.0);

                        // Define Card frame (consistent with our layout)
                        let card_frame = egui::Frame::NONE
                            .fill(theme.bg_card())
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::same(16))
                            .stroke(egui::Stroke::new(1.0, theme.border_base()));

                        // Metric Card 1: Account Balance
                        card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new("Account Balance")
                                    .color(theme.text_muted())
                                    .size(11.0),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new("$1,245.50")
                                    .color(theme.text_primary())
                                    .size(24.0)
                                    .strong(),
                            );
                            ui.add_space(12.0);
                            egui::Frame::NONE
                                .fill(theme.tinted_bg(theme.success(), 32))
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new("+12.4% from last month")
                                            .color(theme.success())
                                            .size(10.0)
                                            .strong(),
                                    );
                                });
                        });
                        ui.add_space(16.0);

                        // Metric Card 2: Monthly Spend
                        card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new("Monthly Spend")
                                    .color(theme.text_muted())
                                    .size(11.0),
                            );
                            ui.add_space(8.0);
                            let monthly_spend_val = if session.total_input_tokens > 0 {
                                let total_tokens =
                                    session.total_input_tokens + session.total_output_tokens;
                                format!("${:.2}", total_tokens as f64 * 0.00015)
                            } else {
                                "$320.15".to_string()
                            };
                            ui.label(
                                egui::RichText::new(monthly_spend_val)
                                    .color(theme.text_primary())
                                    .size(24.0)
                                    .strong(),
                            );
                            ui.add_space(12.0);
                            egui::Frame::NONE
                                .fill(theme.tinted_bg(theme.success(), 32))
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new("-4.2% vs last month")
                                            .color(theme.success())
                                            .size(10.0)
                                            .strong(),
                                    );
                                });
                        });
                        ui.add_space(16.0);

                        // Metric Card 3: Active Sessions
                        card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new("Active Sessions")
                                    .color(theme.text_muted())
                                    .size(11.0),
                            );
                            ui.add_space(8.0);
                            let active_agents_count = session.agents.len().max(1);
                            ui.label(
                                egui::RichText::new(active_agents_count.to_string())
                                    .color(theme.text_primary())
                                    .size(24.0)
                                    .strong(),
                            );
                            ui.add_space(12.0);
                            egui::Frame::NONE
                                .fill(theme.tinted_bg(theme.primary(), 32))
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new("2 in background")
                                            .color(theme.primary())
                                            .size(10.0)
                                            .strong(),
                                    );
                                });
                        });
                        ui.add_space(16.0);

                        // Metric Card 4: Token / Context Usage Info
                        card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new("Context Usage")
                                    .color(theme.text_muted())
                                    .size(11.0),
                            );
                            ui.add_space(8.0);
                            let total_tokens = session.total_input_tokens + session.total_output_tokens;
                            ui.label(
                                egui::RichText::new(format!("{total_tokens} / 128k"))
                                    .color(theme.text_primary())
                                    .size(24.0)
                                    .strong(),
                            );
                            ui.add_space(12.0);
                            let fraction = if total_tokens > 0 {
                                (total_tokens as f32 / 128000.0).min(1.0)
                            } else {
                                0.0
                            };
                            let bar_color = if fraction > 0.85 {
                                theme.error()
                            } else if fraction > 0.5 {
                                theme.warning()
                            } else {
                                theme.success()
                            };
                            ui.add(
                                egui::ProgressBar::new(fraction)
                                    .desired_height(4.0)
                                    .fill(bar_color)
                            );
                        });
                    });

                    // --- COLUMN 1: CENTER ACTIVITY & GRAPHS ---
                    let center_col = &mut columns[1];
                    center_col.vertical(|ui| {
                        ui.heading(
                            egui::RichText::new("Graph & Network Topology")
                                .color(theme.text_primary())
                                .size(15.0)
                                .strong(),
                        );
                        ui.add_space(12.0);

                        crate::app::components::network_graph::render(ui, session, theme);
                        ui.add_space(24.0);

                        // Token Usage Trend
                        let chart_frame = egui::Frame::NONE
                            .fill(theme.bg_card())
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::same(20))
                            .stroke(egui::Stroke::new(1.0, theme.border_base()));

                        chart_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new("Token Usage Trend (Daily)")
                                    .color(theme.text_primary())
                                    .size(16.0)
                                    .strong(),
                            );
                            ui.add_space(20.0);

                            ui.horizontal(|ui| {
                                let bar_values = [42, 65, 88, 55, 92, 110, 75];
                                let weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

                                for i in 0..7 {
                                    ui.vertical(|ui| {
                                        ui.set_width(32.0); // reduced width for 3-col layout fit

                                        let (rect, _resp) = ui.allocate_exact_size(
                                            egui::vec2(18.0, 100.0),
                                            egui::Sense::hover(),
                                        );

                                        ui.painter().rect_filled(
                                            rect,
                                            egui::CornerRadius::same(4),
                                            theme.bg_surface0(),
                                        );

                                        let filled_h = (bar_values[i] as f32 / 120.0) * 100.0;
                                        let filled_rect = egui::Rect::from_min_max(
                                            egui::pos2(rect.min.x, rect.max.y - filled_h),
                                            egui::pos2(rect.max.x, rect.max.y),
                                        );
                                        ui.painter().rect_filled(
                                            filled_rect,
                                            egui::CornerRadius::same(4),
                                            theme.primary(),
                                        );

                                        ui.add_space(6.0);
                                        ui.label(
                                            egui::RichText::new(weekdays[i])
                                                .color(theme.text_muted())
                                                .size(10.0),
                                        );
                                    });
                                    ui.add_space(8.0);
                                }
                            });
                        });
                    });

                    // --- COLUMN 2: OPERATIONS & RECENT LOGS ---
                    let right_col = &mut columns[2];
                    right_col.vertical(|ui| {
                        ui.heading(
                            egui::RichText::new("Platform Operations")
                                .color(theme.text_primary())
                                .size(15.0)
                                .strong(),
                        );
                        ui.add_space(12.0);

                        let bottom_card_frame = egui::Frame::NONE
                            .fill(theme.bg_card())
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::same(16))
                            .stroke(egui::Stroke::new(1.0, theme.border_base()));

                        // Recent Tool Executions
                        bottom_card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new("Recent Tool Executions")
                                    .color(theme.text_primary())
                                    .size(16.0)
                                    .strong(),
                            );
                            ui.add_space(16.0);

                            let tools = if !session.tools.is_empty() {
                                session
                                    .tools
                                    .iter()
                                    .map(|t| t.name.clone())
                                    .take(4)
                                    .collect::<Vec<_>>()
                            } else {
                                vec![
                                    "read_file".to_string(),
                                    "grep".to_string(),
                                    "bash".to_string(),
                                    "write_file".to_string(),
                                ]
                            };

                            for (idx, tool_name) in tools.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let (badge_text, badge_color, bg_color) = if idx == 3 {
                                        (
                                            "PENDING",
                                            theme.warning(),
                                            theme.tinted_bg(theme.warning(), 32),
                                        )
                                    } else {
                                        (
                                            "SUCCESS",
                                            theme.success(),
                                            theme.tinted_bg(theme.success(), 32),
                                        )
                                    };

                                    egui::Frame::NONE
                                        .fill(bg_color)
                                        .corner_radius(egui::CornerRadius::same(2))
                                        .inner_margin(egui::Margin::symmetric(6, 2))
                                        .show(ui, |ui| {
                                            ui.label(
                                                egui::RichText::new(badge_text)
                                                    .color(badge_color)
                                                    .size(10.0)
                                                    .strong(),
                                            );
                                        });

                                    ui.add_space(8.0);
                                    ui.label(
                                        egui::RichText::new(tool_name)
                                            .color(theme.text_primary())
                                            .size(13.0),
                                    );
                                });
                                ui.add_space(10.0);
                            }
                        });
                        ui.add_space(16.0);

                        // Memory Blocks
                        bottom_card_frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                egui::RichText::new("Memory Blocks")
                                    .color(theme.text_primary())
                                    .size(16.0)
                                    .strong(),
                            );
                            ui.add_space(16.0);

                            let blocks = if !session.memory_blocks.is_empty() {
                                session
                                    .memory_blocks
                                    .iter()
                                    .map(|b| (b.label.clone(), b.value.clone()))
                                    .take(4)
                                    .collect::<Vec<_>>()
                            } else {
                                vec![
                                    ("persona".to_string(), "CADE assistant...".to_string()),
                                    ("project".to_string(), "Workspace config...".to_string()),
                                    ("human".to_string(), "User preferences...".to_string()),
                                    (
                                        "active_goal".to_string(),
                                        "Refactoring CADE web GUI...".to_string(),
                                    ),
                                ]
                            };

                            for (label, val) in blocks {
                                ui.horizontal(|ui| {
                                    egui::Frame::NONE
                                        .fill(theme.bg_surface0())
                                        .corner_radius(egui::CornerRadius::same(3))
                                        .inner_margin(egui::Margin::symmetric(8, 4))
                                        .show(ui, |ui| {
                                            ui.label(
                                                egui::RichText::new(format!("📌 {}", label))
                                                    .color(theme.text_primary())
                                                    .size(11.0)
                                                    .strong(),
                                            );
                                        });

                                    ui.add_space(8.0);
                                    let preview: String = val.chars().take(18).collect();
                                    let suffix = if val.chars().count() > 18 { "..." } else { "" };
                                    ui.label(
                                        egui::RichText::new(format!("{}{}", preview, suffix))
                                            .color(theme.text_muted())
                                            .size(11.0),
                                    );
                                });
                                ui.add_space(8.0);
                            }
                        });
                    });
                });
            });
        });
}

