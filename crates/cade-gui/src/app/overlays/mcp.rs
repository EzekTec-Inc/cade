//! MCP servers overlay — shows all MCP servers configured on the server,
//! their tool lists, and connected/disabled status.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::super::AppAction;

/// Render the MCP servers overlay.
///
/// Returns an `AppAction` if the user interacts (close).
pub fn render_mcp_overlay(
    ctx: &egui::Context,
    servers: &[crate::api::McpServerInfo],
    loading: bool,
    error: Option<&str>,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result: Option<AppAction> = None;

    let screen = ctx.content_rect();
    let w = 640.0_f32.min(screen.width() - 40.0);
    let h = 520.0_f32.min(screen.height() - 60.0);
    let x = (screen.width() - w) / 2.0 + screen.left();
    let y = (screen.height() - h) / 2.0 + screen.top();

    // Dim backdrop
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("mcp_overlay_backdrop"),
    ));
    painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(140));

    // ESC closes
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        return Some(AppAction::CloseMcpOverlay);
    }

    egui::Area::new(egui::Id::new("mcp_overlay_area"))
        .fixed_pos(egui::pos2(x, y))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme.bg_surface0())
                .stroke(egui::Stroke::new(1.0, theme.border_base()))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_min_size(egui::vec2(w - 32.0, h - 32.0));
                    ui.set_max_size(egui::vec2(w - 32.0, h - 32.0));

                    // Title bar
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("MCP Servers")
                                .color(theme.text_primary())
                                .strong()
                                .size(16.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(egui::RichText::new("✕").color(theme.text_dim()))
                                .clicked()
                            {
                                result = Some(AppAction::CloseMcpOverlay);
                            }
                        });
                    });

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // Error banner
                    if let Some(err) = error {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("⚠ {err}"))
                                    .color(theme.error())
                                    .size(12.0),
                            );
                        });
                        ui.add_space(8.0);
                    }

                    // Loading spinner
                    if loading {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                egui::RichText::new("Loading MCP servers…")
                                    .color(theme.text_dim())
                                    .size(13.0),
                            );
                        });
                        return;
                    }

                    // Empty state
                    if servers.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(
                                egui::RichText::new("No MCP servers configured")
                                    .color(theme.text_dim())
                                    .size(14.0),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(
                                    "Add MCP servers to ~/.config/cade/settings.toml\n\
                                     or a project-level cade.toml",
                                )
                                .color(theme.text_muted())
                                .size(12.0),
                            );
                        });
                        return;
                    }

                    // Summary line
                    let enabled_count = servers.iter().filter(|s| !s.disabled).count();
                    let total_tools: usize = servers.iter().map(|s| s.tools.len()).sum();
                    ui.label(
                        egui::RichText::new(format!(
                            "{} server{} active · {} tool{}",
                            enabled_count,
                            if enabled_count == 1 { "" } else { "s" },
                            total_tools,
                            if total_tools == 1 { "" } else { "s" },
                        ))
                        .color(theme.text_dim())
                        .size(11.0),
                    );
                    ui.add_space(8.0);

                    // Scrollable server list
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for server in servers {
                                let (status_color, status_label) = if server.disabled {
                                    (theme.text_muted(), "DISABLED")
                                } else {
                                    (theme.success(), "ACTIVE")
                                };

                                // Server card
                                egui::Frame::new()
                                    .fill(theme.bg_surface1())
                                    .stroke(egui::Stroke::new(1.0, theme.border_base()))
                                    .corner_radius(egui::CornerRadius::same(6))
                                    .inner_margin(egui::Margin::same(10))
                                    .show(ui, |ui| {
                                        // Server header row
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(&server.key)
                                                    .color(theme.text_primary())
                                                    .strong()
                                                    .size(13.0),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        egui::RichText::new(status_label)
                                                            .color(status_color)
                                                            .strong()
                                                            .size(10.0),
                                                    );
                                                    ui.label(
                                                        egui::RichText::new(format!(
                                                            "{} tool{}",
                                                            server.tools.len(),
                                                            if server.tools.len() == 1 {
                                                                ""
                                                            } else {
                                                                "s"
                                                            }
                                                        ))
                                                        .color(theme.text_dim())
                                                        .size(11.0),
                                                    );
                                                },
                                            );
                                        });

                                        // Command line
                                        ui.label(
                                            egui::RichText::new(&server.command)
                                                .color(theme.text_muted())
                                                .monospace()
                                                .size(11.0),
                                        );

                                        // Tool list (collapsible if many)
                                        if !server.tools.is_empty() {
                                            ui.add_space(6.0);
                                            egui::CollapsingHeader::new(
                                                egui::RichText::new("Tools")
                                                    .color(theme.text_dim())
                                                    .size(11.0),
                                            )
                                            .id_salt(format!("mcp_tools_{}", server.key))
                                            .default_open(server.tools.len() <= 8)
                                            .show(
                                                ui,
                                                |ui| {
                                                    for tool in &server.tools {
                                                        // Strip the "server__" prefix for display
                                                        let display = tool
                                                            .split_once("__")
                                                            .map(|(_, t)| t)
                                                            .unwrap_or(tool);
                                                        ui.horizontal(|ui| {
                                                            ui.label(
                                                                egui::RichText::new("·  ")
                                                                    .color(theme.border_base())
                                                                    .monospace()
                                                                    .size(11.0),
                                                            );
                                                            ui.label(
                                                                egui::RichText::new(display)
                                                                    .color(theme.text_dim())
                                                                    .monospace()
                                                                    .size(11.0),
                                                            );
                                                        });
                                                    }
                                                },
                                            );
                                        }
                                    });

                                ui.add_space(6.0);
                            }
                        });
                });
        });

    result
}
