use eframe::egui;
use crate::theme::EguiThemeExt;

pub fn render(ui: &mut egui::Ui, session: &crate::session::ConnectedSession, theme: &crate::theme::ThemeColors) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(20.0);
        
        ui.heading(egui::RichText::new("Overview").color(theme.text_primary()));
        ui.add_space(20.0);

        ui.columns(2, |columns| {
            // Left Column
            columns[0].vertical(|ui| {
                // What's New Section
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("What's New").color(theme.primary()));
                    ui.label("CADE has been updated to support hybrid tab-based dashboard views.");
                });
                
                ui.add_space(20.0);

                // Current Configuration
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("Current Configuration").color(theme.primary()));
                    
                    let model = session.context_stats.as_ref().and_then(|c| c.model.clone()).unwrap_or_else(|| "Auto-detect".into());
                    ui.label(format!("Model: {}", model));
                    ui.label(format!("Available Tools: {}", session.tools.len()));
                    ui.label(format!("MCP Servers: {}", session.mcp_servers.len()));
                });
            });

            // Right Column
            columns[1].vertical(|ui| {
                // Tool Usage Chart
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("Agent Metrics").color(theme.primary()));
                    
                    let metrics = &session.agent_metrics;
                    let compacted = metrics.as_ref().map(|m| m.tool_outputs_compacted as f64).unwrap_or(0.0);
                    let runs = metrics.as_ref().map(|m| m.consolidation_runs as f64).unwrap_or(0.0);
                    let guard_hits = metrics.as_ref().map(|m| m.inflation_guard_hits as f64).unwrap_or(0.0);

                    ui.label(format!("Compacted: {}", compacted));
                    ui.label(format!("Consolidations: {}", runs));
                    ui.label(format!("Guard Hits: {}", guard_hits));
                });

                ui.add_space(20.0);

                // Execution Queue
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("Executions Queue").color(theme.primary()));
                    let cards = &session.subagent_cards;
                    if cards.is_empty() {
                        ui.label("No active background executions.");
                    } else {
                        for card in cards {
                            ui.label(format!("Task: {} [{}]", card.task, card.status));
                        }
                    }
                });
                
                ui.add_space(20.0);
                
                // Last Execution
                ui.group(|ui| {
                    ui.heading(egui::RichText::new("Last Execution").color(theme.primary()));
                    if let Some(reason) = &session.last_finish_reason {
                        ui.label(format!("Status: {}", reason));
                    } else {
                        ui.label("Status: N/A");
                    }
                    
                    if let Some((input, output, _)) = &session.last_usage {
                        ui.label(format!("Tokens: {} in, {} out", input, output));
                    } else {
                        ui.label("Tokens: N/A");
                    }
                });
            });
        });
    });
}
