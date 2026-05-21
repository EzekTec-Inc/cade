use eframe::egui;
use crate::theme::EguiThemeExt;

pub fn render(ui: &mut egui::Ui, session: &crate::session::ConnectedSession, theme: &crate::theme::ThemeColors) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(20.0);
        
        ui.heading(egui::RichText::new("Dashboard Overview").color(theme.text_primary()).size(24.0));
        ui.add_space(20.0);

        ui.columns(2, |columns| {
            // Left Column
            columns[0].vertical(|ui| {
                // What's New Section
                egui::Frame::NONE.inner_margin(8.0).fill(theme.bg_surface0()).corner_radius(8.0).show(ui, |ui| {
                    ui.heading(egui::RichText::new("What's New").color(theme.primary()));
                    ui.add_space(8.0);
                    ui.label("CADE has been updated to support hybrid tab-based dashboard views.");
                });
                
                ui.add_space(20.0);

                // Current Configuration
                egui::Frame::NONE.inner_margin(8.0).fill(theme.bg_surface0()).corner_radius(8.0).show(ui, |ui| {
                    ui.heading(egui::RichText::new("Current Configuration").color(theme.primary()));
                    ui.add_space(8.0);
                    
                    egui::Grid::new("config_grid").num_columns(2).spacing([40.0, 8.0]).show(ui, |ui| {
                        ui.label(egui::RichText::new("Model:").color(theme.text_muted()));
                        let model = session.context_stats.as_ref().and_then(|c| c.model.clone()).unwrap_or_else(|| "Auto-detect".into());
                        ui.label(model);
                        ui.end_row();
                        
                        ui.label(egui::RichText::new("Available Tools:").color(theme.text_muted()));
                        ui.label(session.tools.len().to_string());
                        ui.end_row();
                        
                        ui.label(egui::RichText::new("MCP Servers:").color(theme.text_muted()));
                        ui.label(session.mcp_servers.len().to_string());
                        ui.end_row();
                    });
                });

                ui.add_space(20.0);

                // Token Usage Summary
                egui::Frame::NONE.inner_margin(8.0).fill(theme.bg_surface0()).corner_radius(8.0).show(ui, |ui| {
                    ui.heading(egui::RichText::new("Session Usage").color(theme.primary()));
                    ui.add_space(8.0);

                    let total_input = session.total_input_tokens;
                    let total_output = session.total_output_tokens;
                    let total = total_input + total_output;

                    egui::Grid::new("usage_grid").num_columns(2).spacing([40.0, 8.0]).show(ui, |ui| {
                        ui.label(egui::RichText::new("Total Input Tokens:").color(theme.text_muted()));
                        ui.label(total_input.to_string());
                        ui.end_row();
                        
                        ui.label(egui::RichText::new("Total Output Tokens:").color(theme.text_muted()));
                        ui.label(total_output.to_string());
                        ui.end_row();
                        
                        ui.label(egui::RichText::new("Combined Total:").color(theme.text_muted()));
                        ui.label(total.to_string());
                        ui.end_row();
                    });

                    ui.add_space(12.0);
                    ui.label("Context Window Fill:");
                    const DEFAULT_WINDOW: u64 = 128_000;
                    let frac = crate::theme::context_fill_fraction(total, DEFAULT_WINDOW);
                    let bar_color = crate::theme::context_fill_color(frac, theme);
                    
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_height(14.0)
                            .fill(bar_color)
                            .text(format!("{:.1}%", frac * 100.0)),
                    );
                });
            });

            // Right Column
            columns[1].vertical(|ui| {
                // Agent Metrics
                egui::Frame::NONE.inner_margin(8.0).fill(theme.bg_surface0()).corner_radius(8.0).show(ui, |ui| {
                    ui.heading(egui::RichText::new("Agent Metrics").color(theme.primary()));
                    ui.add_space(8.0);
                    
                    let metrics = &session.agent_metrics;
                    let compacted = metrics.as_ref().map(|m| m.tool_outputs_compacted as f64).unwrap_or(0.0);
                    let runs = metrics.as_ref().map(|m| m.consolidation_runs as f64).unwrap_or(0.0);
                    let guard_hits = metrics.as_ref().map(|m| m.inflation_guard_hits as f64).unwrap_or(0.0);

                    egui::Grid::new("metrics_grid").num_columns(2).spacing([40.0, 8.0]).show(ui, |ui| {
                        ui.label(egui::RichText::new("Compacted Data:").color(theme.text_muted()));
                        ui.label(compacted.to_string());
                        ui.end_row();
                        
                        ui.label(egui::RichText::new("Consolidations:").color(theme.text_muted()));
                        ui.label(runs.to_string());
                        ui.end_row();
                        
                        ui.label(egui::RichText::new("Guard Hits:").color(theme.text_muted()));
                        ui.label(guard_hits.to_string());
                        ui.end_row();
                    });
                });

                ui.add_space(20.0);
                
                // Last Execution
                egui::Frame::NONE.inner_margin(8.0).fill(theme.bg_surface0()).corner_radius(8.0).show(ui, |ui| {
                    ui.heading(egui::RichText::new("Last Execution").color(theme.primary()));
                    ui.add_space(8.0);
                    
                    egui::Grid::new("last_exec_grid").num_columns(2).spacing([40.0, 8.0]).show(ui, |ui| {
                        ui.label(egui::RichText::new("Status:").color(theme.text_muted()));
                        if let Some(reason) = &session.last_finish_reason {
                            ui.label(reason);
                        } else {
                            ui.label("N/A");
                        }
                        ui.end_row();
                        
                        ui.label(egui::RichText::new("Turn Tokens:").color(theme.text_muted()));
                        if let Some((input, output, _)) = &session.last_usage {
                            ui.label(format!("{} in, {} out", input, output));
                        } else {
                            ui.label("N/A");
                        }
                        ui.end_row();
                    });
                });

                ui.add_space(20.0);

                // Execution Queue
                egui::Frame::NONE.inner_margin(8.0).fill(theme.bg_surface0()).corner_radius(8.0).show(ui, |ui| {
                    ui.heading(egui::RichText::new("Executions Queue").color(theme.primary()));
                    ui.add_space(8.0);
                    
                    let cards = &session.subagent_cards;
                    if cards.is_empty() {
                        ui.label(egui::RichText::new("No active background executions.").color(theme.text_muted()).italics());
                    } else {
                        for card in cards {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("•").color(theme.primary()));
                                ui.label(format!("{} - ", card.task));
                                ui.label(egui::RichText::new(&card.status).color(theme.text_muted()));
                            });
                        }
                    }
                });
            });
        });
    });
}
