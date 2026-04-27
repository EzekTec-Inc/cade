use crate::app::AppAction;
use crate::theme::EguiThemeExt;
use eframe::egui;

/// Helper: render a section header in TUI style.
fn section_header(ui: &mut egui::Ui, label: &str, theme: &crate::theme::ThemeColors) {
    ui.label(
        egui::RichText::new(format!(" {label} "))
            .color(theme.primary())
            .monospace()
            .strong()
            .size(11.0),
    );
}

/// Helper: render a key-value pair row.
fn kv_row(
    ui: &mut egui::Ui,
    key: &str,
    val: &str,
    val_color: egui::Color32,
    theme: &crate::theme::ThemeColors,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(" {key:<8} "))
                .color(theme.text_muted())
                .monospace()
                .size(10.0),
        );
        ui.label(
            egui::RichText::new(val)
                .color(val_color)
                .monospace()
                .size(10.0),
        );
    });
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    ui: &mut egui::Ui,
    agents: &[cade_api_types::AgentInfo],
    selected_agent: &Option<usize>,
    has_agent: bool,
    agent_metrics: Option<&crate::api::AgentMetrics>,
    conversations: &[crate::api::ConversationInfo],
    selected_conversation: &Option<usize>,
    is_streaming: bool,
    active_plan: Option<&crate::session::PlanState>,
    total_tokens: (u64, u64),
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut action: Option<AppAction> = None;

    egui::Panel::left("agent_sidebar")
        .default_size(200.0)
        .resizable(true)
        .show_inside(ui, |ui| {
            // ── Agent selection ────────────────────────
            ui.label(
                egui::RichText::new("Agents")
                    .strong()
                    .monospace()
                    .size(12.0),
            );
            ui.add_space(2.0);
            if agents.is_empty() {
                ui.label(
                    egui::RichText::new("No agents.")
                        .color(theme.text_dim())
                        .monospace()
                        .size(10.0),
                );
            } else {
                for (i, agent) in agents.iter().enumerate() {
                    let is_selected = *selected_agent == Some(i);
                    let label = format!("🤖 {}", agent.name);
                    if ui.selectable_label(is_selected, label).clicked() && !is_selected {
                        action = Some(AppAction::SelectAgent(i));
                    }
                }
            }

            // Thin separator
            ui.add_space(4.0);
            let sep = ui.available_rect_before_wrap();
            let sep = egui::Rect::from_min_size(sep.min, egui::vec2(sep.width(), 1.0));
            ui.painter().rect_filled(sep, 0.0, theme.border_base());
            ui.advance_cursor_after_rect(sep);
            ui.add_space(4.0);

            if has_agent {
                if let Some(idx) = *selected_agent {
                    if let Some(agent) = agents.get(idx) {
                        // ── Session section (mirrors TUI) ─────────────
                        section_header(ui, "Session", theme);
                        kv_row(ui, "agent", &agent.name, theme.text_primary(), theme);
                        if let Some(model) = &agent.model {
                            kv_row(ui, "model", model, theme.text_primary(), theme);
                        }
                        if let Some(provider) = &agent.provider {
                            kv_row(ui, "provider", provider, theme.text_primary(), theme);
                        }
                        ui.add_space(4.0);

                        // ── Status section ────────────────────────────
                        section_header(ui, "Status", theme);

                        // Context %
                        let total = total_tokens.0 + total_tokens.1;
                        let ctx_pct = if total > 0 {
                            let pct = ((total as f64 / 128_000.0) * 100.0).min(100.0);
                            format!("{:.0}%", pct)
                        } else {
                            "—".to_string()
                        };
                        let ctx_color = {
                            let pct = (total as f64 / 128_000.0 * 100.0) as u8;
                            if pct >= 90 {
                                theme.error()
                            } else if pct >= 80 {
                                theme.warning()
                            } else {
                                theme.text_muted()
                            }
                        };
                        kv_row(ui, "context", &ctx_pct, ctx_color, theme);

                        // Tokens
                        kv_row(
                            ui,
                            "tokens",
                            &format!("↑{} ↓{}", total_tokens.0, total_tokens.1),
                            theme.text_primary(),
                            theme,
                        );

                        // Metrics
                        if let Some(m) = agent_metrics {
                            kv_row(
                                ui,
                                "consol.",
                                &m.consolidation_runs.to_string(),
                                theme.text_dim(),
                                theme,
                            );
                            kv_row(
                                ui,
                                "compact",
                                &m.tool_outputs_compacted.to_string(),
                                theme.text_dim(),
                                theme,
                            );
                        }
                        ui.add_space(4.0);

                        // ── Activity section ──────────────────────────
                        section_header(ui, "Activity", theme);
                        let activity = if is_streaming { "streaming…" } else { "idle" };
                        let act_color = if is_streaming {
                            theme.warning()
                        } else {
                            theme.text_muted()
                        };
                        ui.label(
                            egui::RichText::new(format!(" {activity}"))
                                .color(act_color)
                                .monospace()
                                .size(10.0),
                        );
                        ui.add_space(4.0);

                        // ── Plan section ──────────────────────────────
                        section_header(ui, "Plan", theme);
                        let plan_summary = match active_plan {
                            Some(plan) => {
                                let done = plan.steps.iter().filter(|s| s.is_done).count();
                                let total = plan.steps.len();
                                if total > 0 {
                                    format!("{done}/{total} complete")
                                } else {
                                    "none".into()
                                }
                            }
                            None => "none".into(),
                        };
                        kv_row(ui, "todos", &plan_summary, theme.text_primary(), theme);
                        ui.add_space(4.0);

                        // ── Keys section ──────────────────────────────
                        section_header(ui, "Keys", theme);
                        for hint in [
                            "Ctrl+P  command palette",
                            "/       slash commands",
                            "Esc     close overlay",
                        ] {
                            ui.label(
                                egui::RichText::new(format!(" {hint}"))
                                    .color(theme.text_muted())
                                    .monospace()
                                    .size(9.0),
                            );
                        }
                    }
                }

                // Thin separator before conversations
                ui.add_space(4.0);
                let sep = ui.available_rect_before_wrap();
                let sep = egui::Rect::from_min_size(sep.min, egui::vec2(sep.width(), 1.0));
                ui.painter().rect_filled(sep, 0.0, theme.border_base());
                ui.advance_cursor_after_rect(sep);
                ui.add_space(4.0);

                // ── Conversations ─────────────────────────
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Conversations")
                            .strong()
                            .monospace()
                            .size(11.0),
                    );
                    if ui.small_button("+ New").clicked() {
                        action = Some(AppAction::NewConversation);
                    }
                });
                ui.add_space(2.0);

                if conversations.is_empty() {
                    ui.label(
                        egui::RichText::new("No conversations yet.")
                            .color(theme.text_dim())
                            .monospace()
                            .size(10.0),
                    );
                } else {
                    for (ci, conv) in conversations.iter().enumerate() {
                        let is_sel = *selected_conversation == Some(ci);
                        let title = if conv.title.is_empty() {
                            "Untitled"
                        } else {
                            &conv.title
                        };
                        ui.horizontal(|ui| {
                            let label = format!("💬 {} ({})", title, conv.message_count);
                            if ui.selectable_label(is_sel, label).clicked() && !is_sel {
                                action = Some(AppAction::SelectConversation(ci));
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let del_btn = egui::Button::new(
                                        egui::RichText::new("🗑").color(theme.text_dim()).size(10.0),
                                    )
                                    .fill(egui::Color32::TRANSPARENT)
                                    .stroke(egui::Stroke::NONE)
                                    .min_size(egui::vec2(16.0, 16.0));
                                    if ui.add(del_btn).on_hover_text("Delete").clicked() {
                                        action = Some(AppAction::DeleteConversation(ci));
                                    }
                                },
                            );
                        });
                    }
                }

                // Thin separator
                ui.add_space(4.0);
                let sep = ui.available_rect_before_wrap();
                let sep = egui::Rect::from_min_size(sep.min, egui::vec2(sep.width(), 1.0));
                ui.painter().rect_filled(sep, 0.0, theme.border_base());
                ui.advance_cursor_after_rect(sep);
                ui.add_space(4.0);
            }

            if ui.button("🚪 Logout").clicked() {
                action = Some(AppAction::Logout);
            }
        });

    action
}
