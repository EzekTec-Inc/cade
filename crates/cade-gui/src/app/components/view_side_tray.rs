use eframe::egui;
use cade_api_types::AgentInfo;
use crate::api::AgentMetrics;
use crate::api::ConversationInfo;
use crate::app::AppAction;

pub fn render(
    ctx: &egui::Context,
    agents: &[AgentInfo],
    selected_agent: Option<usize>,
    has_agent: bool,
    conversations: &[ConversationInfo],
    selected_conversation: Option<usize>,
    agent_metrics: &Option<AgentMetrics>,
    action: &mut AppAction,
) {
    egui::SidePanel::left("agent_sidebar")
        .default_size(180.0)
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading("Agents");
            ui.separator();
            if agents.is_empty() {
                ui.label("No agents configured.");
            } else {
                for (i, agent) in agents.iter().enumerate() {
                    let is_selected = selected_agent == Some(i);
                    let label = format!("🤖 {}", agent.name);
                    if ui.selectable_label(is_selected, label).clicked() && !is_selected {
                        *action = AppAction::SelectAgent(i);
                    }
                }
            }
            ui.separator();

            if has_agent {
                if let Some(idx) = selected_agent {
                    if let Some(agent) = agents.get(idx) {
                        ui.add_space(2.0);
                        egui::Frame::new()
                            .fill(crate::theme::BG_SURFACE0)
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    if let Some(model) = &agent.model {
                                        ui.label(
                                            egui::RichText::new(format!("model: {model}"))
                                                .monospace()
                                                .color(crate::theme::PRIMARY)
                                                .size(11.0),
                                        );
                                    }
                                    if let Some(provider) = &agent.provider {
                                        ui.label(
                                            egui::RichText::new(format!("provider: {provider}"))
                                                .monospace()
                                                .color(crate::theme::TEXT_MUTED)
                                                .size(11.0),
                                        );
                                    }
                                    let short_id = if agent.id.len() > 12 {
                                        format!("{}…", &agent.id[..12])
                                    } else {
                                        agent.id.clone()
                                    };
                                    ui.label(
                                        egui::RichText::new(format!("id: {short_id}"))
                                            .monospace()
                                            .color(crate::theme::TEXT_DIM)
                                            .size(10.0),
                                    );

                                    if let Some(m) = agent_metrics {
                                        ui.add_space(4.0);
                                        ui.separator();
                                        for (label, val) in [
                                            ("consolidations", m.consolidation_runs),
                                            ("compacted", m.tool_outputs_compacted),
                                            ("guard hits", m.inflation_guard_hits),
                                        ] {
                                            ui.label(
                                                egui::RichText::new(format!("{label}: {val}"))
                                                    .color(crate::theme::TEXT_DIM)
                                                    .size(10.0),
                                            );
                                        }
                                    }
                                });
                            });
                    }
                }

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Conversations").strong().size(13.0));
                    if ui.small_button("➕ New").clicked() {
                        *action = AppAction::NewConversation;
                    }
                });
                ui.add_space(2.0);

                if conversations.is_empty() {
                    ui.label(egui::RichText::new("No conversations yet.").weak().size(11.0));
                } else {
                    for (ci, conv) in conversations.iter().enumerate() {
                        let is_sel = selected_conversation == Some(ci);
                        let title = if conv.title.is_empty() {
                            "Untitled"
                        } else {
                            &conv.title
                        };
                        ui.horizontal(|ui| {
                            let label = format!("💬 {} ({})", title, conv.message_count);
                            if ui.selectable_label(is_sel, label).clicked() && !is_sel {
                                *action = AppAction::SelectConversation(ci);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let del_btn = egui::Button::new(
                                        egui::RichText::new("🗑")
                                            .color(crate::theme::TEXT_DIM)
                                            .size(11.0),
                                    )
                                    .fill(egui::Color32::TRANSPARENT)
                                    .stroke(egui::Stroke::NONE)
                                    .min_size(egui::vec2(18.0, 18.0));
                                    if ui.add(del_btn).on_hover_text("Delete conversation").clicked() {
                                        *action = AppAction::DeleteConversation(ci);
                                    }
                                },
                            );
                        });
                    }
                }
                ui.separator();
            }

            ui.add_space(4.0);
            if ui.button("🚪 Logout").clicked() {
                *action = AppAction::Logout;
            }
        });
}
