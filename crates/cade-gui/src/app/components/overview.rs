use eframe::egui;

pub fn render(
    ui: &mut egui::Ui,
    _profile_name: &mut String,
    _profile_email: &mut String,
    _session: &crate::session::ConnectedSession,
    _theme: &crate::theme::ThemeColors,
) {
    let background_frame = egui::Frame::NONE
        .fill(egui::Color32::from_rgb(23, 23, 23)) // #171717
        .inner_margin(egui::Margin::symmetric(24, 24));

    egui::CentralPanel::default()
        .frame(background_frame)
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // Title
                ui.heading(
                    egui::RichText::new("Account Overview")
                        .color(egui::Color32::WHITE)
                        .size(24.0)
                        .strong(),
                );
                ui.add_space(20.0);

                // ── TOP ROW: THREE CARDS ──────────────────────────────────────────
                ui.columns(3, |columns| {
                    // Card 1: Account Balance
                    let card_frame = egui::Frame::NONE
                        .fill(egui::Color32::from_rgb(30, 30, 30)) // #1E1E1E
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::same(16))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(50)));

                    card_frame.show(&mut columns[0], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new("Account Balance")
                                .color(egui::Color32::from_gray(150))
                                .size(12.0),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("$1,245.50")
                                .color(egui::Color32::WHITE)
                                .size(26.0)
                                .strong(),
                        );
                        ui.add_space(12.0);

                        // Green increase pill
                        egui::Frame::NONE
                            .fill(egui::Color32::from_rgb(28, 45, 35))
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("+12.4% from last month")
                                        .color(egui::Color32::from_rgb(60, 180, 100))
                                        .size(11.0)
                                        .strong(),
                                );
                            });
                    });

                    // Card 2: Monthly Spend
                    card_frame.show(&mut columns[1], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new("Monthly Spend")
                                .color(egui::Color32::from_gray(150))
                                .size(12.0),
                        );
                        ui.add_space(8.0);
                        let monthly_spend_val = if _session.total_input_tokens > 0 {
                            let total_tokens =
                                _session.total_input_tokens + _session.total_output_tokens;
                            format!("${:.2}", total_tokens as f64 * 0.00015)
                        } else {
                            "$320.15".to_string()
                        };
                        ui.label(
                            egui::RichText::new(monthly_spend_val)
                                .color(egui::Color32::WHITE)
                                .size(26.0)
                                .strong(),
                        );
                        ui.add_space(12.0);

                        // Green decrease pill
                        egui::Frame::NONE
                            .fill(egui::Color32::from_rgb(28, 45, 35))
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("-4.2% vs last month")
                                        .color(egui::Color32::from_rgb(60, 180, 100))
                                        .size(11.0)
                                        .strong(),
                                );
                            });
                    });

                    // Card 3: Active Sessions
                    card_frame.show(&mut columns[2], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new("Active Sessions")
                                .color(egui::Color32::from_gray(150))
                                .size(12.0),
                        );
                        ui.add_space(8.0);
                        let active_agents_count = _session.agents.len().max(1);
                        ui.label(
                            egui::RichText::new(active_agents_count.to_string())
                                .color(egui::Color32::WHITE)
                                .size(26.0)
                                .strong(),
                        );
                        ui.add_space(12.0);

                        // Background sessions pill
                        egui::Frame::NONE
                            .fill(egui::Color32::from_rgb(45, 45, 45))
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(8, 4))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new("2 in background")
                                        .color(egui::Color32::from_gray(180))
                                        .size(11.0)
                                        .strong(),
                                );
                            });
                    });
                });

                ui.add_space(24.0);

                crate::app::components::network_graph::render(ui, _session, _theme);

                ui.add_space(24.0);

                // ── MIDDLE SECTION: TOKEN USAGE TREND CHART ──────────────────────
                let chart_frame = egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(30, 30, 30))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(20))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(50)));

                chart_frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new("Token Usage Trend (Daily)")
                            .color(egui::Color32::WHITE)
                            .size(16.0)
                            .strong(),
                    );
                    ui.add_space(20.0);

                    ui.horizontal(|ui| {
                        let bar_values = [42, 65, 88, 55, 92, 110, 75];
                        let weekdays = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

                        for i in 0..7 {
                            ui.vertical(|ui| {
                                ui.set_width(70.0);

                                let (rect, _resp) = ui.allocate_exact_size(
                                    egui::vec2(24.0, 120.0),
                                    egui::Sense::hover(),
                                );

                                ui.painter().rect_filled(
                                    rect,
                                    egui::CornerRadius::same(4),
                                    egui::Color32::from_rgb(40, 40, 40),
                                );

                                let filled_h = (bar_values[i] as f32 / 120.0) * 120.0;
                                let filled_rect = egui::Rect::from_min_max(
                                    egui::pos2(rect.min.x, rect.max.y - filled_h),
                                    egui::pos2(rect.max.x, rect.max.y),
                                );
                                ui.painter().rect_filled(
                                    filled_rect,
                                    egui::CornerRadius::same(4),
                                    egui::Color32::from_rgb(230, 80, 80),
                                );

                                ui.add_space(6.0);
                                ui.label(
                                    egui::RichText::new(weekdays[i])
                                        .color(egui::Color32::from_gray(160))
                                        .size(11.0),
                                );
                            });
                            ui.add_space(16.0);
                        }
                    });
                });

                ui.add_space(24.0);

                // ── BOTTOM ROW: TWO COLUMNS ───────────────────────────────────────
                ui.columns(2, |cols| {
                    // Left Column: Recent Tool Executions
                    let bottom_card_frame = egui::Frame::NONE
                        .fill(egui::Color32::from_rgb(30, 30, 30))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::same(16))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(50)));

                    bottom_card_frame.show(&mut cols[0], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new("Recent Tool Executions")
                                .color(egui::Color32::WHITE)
                                .size(16.0)
                                .strong(),
                        );
                        ui.add_space(16.0);

                        let tools = if !_session.tools.is_empty() {
                            _session
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
                                        egui::Color32::from_rgb(200, 150, 40),
                                        egui::Color32::from_rgb(50, 45, 30),
                                    )
                                } else {
                                    (
                                        "SUCCESS",
                                        egui::Color32::from_rgb(60, 180, 100),
                                        egui::Color32::from_rgb(30, 45, 35),
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
                                        .color(egui::Color32::from_gray(210))
                                        .size(13.0),
                                );
                            });
                            ui.add_space(10.0);
                        }
                    });

                    // Right Column: Memory Blocks
                    bottom_card_frame.show(&mut cols[1], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            egui::RichText::new("Memory Blocks")
                                .color(egui::Color32::WHITE)
                                .size(16.0)
                                .strong(),
                        );
                        ui.add_space(16.0);

                        let blocks = if !_session.memory_blocks.is_empty() {
                            _session
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
                                    .fill(egui::Color32::from_rgb(45, 45, 45))
                                    .corner_radius(egui::CornerRadius::same(3))
                                    .inner_margin(egui::Margin::symmetric(8, 4))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(format!("📌 {}", label))
                                                .color(egui::Color32::from_gray(200))
                                                .size(11.0)
                                                .strong(),
                                        );
                                    });

                                ui.add_space(8.0);
                                let preview: String = val.chars().take(22).collect();
                                let suffix = if val.chars().count() > 22 { "..." } else { "" };
                                ui.label(
                                    egui::RichText::new(format!("{}{}", preview, suffix))
                                        .color(egui::Color32::from_gray(160))
                                        .size(12.0),
                                );
                            });
                            ui.add_space(8.0);
                        }
                    });
                });
            });
        });
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overview_layout_spec_matches_reference_structure() {
        let spec = reference_layout_spec();
        assert_eq!(spec.title, "CADE Command Center");
        assert_eq!(spec.columns, ["metrics", "network", "operations"]);
        assert_eq!(spec.metric_cards, ["Agents", "Tools", "Memory", "Tokens"]);
        assert_eq!(spec.center_sections, ["Activity", "Network Node Graph"]);
        assert_eq!(spec.operation_panels, ["Session", "MCP Servers", "Recent Tools"]);
    }
}

