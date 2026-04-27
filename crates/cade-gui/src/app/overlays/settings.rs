//! Settings/config overlays dispatched from palette commands.
//!
//! Overlays: providers, permissions, hooks, theme, pricing, toolset, backend.
//! Each is a simple `egui::Window` rendering server-fetched or local config data.

use super::super::AppAction;
use crate::theme::EguiThemeExt;
use eframe::egui;

/// Thin separator helper.
fn sep(ui: &mut egui::Ui, theme: &crate::theme::ThemeColors) {
    let r = ui.available_rect_before_wrap();
    let r = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), 1.0));
    ui.painter().rect_filled(r, 0.0, theme.border_base());
    ui.advance_cursor_after_rect(r);
}

/// Generic overlay frame builder — centered, TUI-style.
fn overlay_frame(
    ctx: &egui::Context,
    title: &str,
    w: f32,
    h: f32,
    theme: &crate::theme::ThemeColors,
    content: impl FnOnce(&mut egui::Ui),
) -> bool {
    let screen = ctx.content_rect();
    let pos = egui::pos2((screen.width() - w) / 2.0, (screen.height() - h) / 2.0);

    let mut open = true;
    egui::Window::new(title)
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .open(&mut open)
        .fixed_pos(pos)
        .fixed_size([w, h])
        .frame(
            egui::Frame::new()
                .fill(theme.bg_surface1())
                .stroke(egui::Stroke::new(1.0, theme.border_base()))
                .corner_radius(egui::CornerRadius::ZERO)
                .inner_margin(egui::Margin::symmetric(10, 8)),
        )
        .show(ctx, |ui| {
            // Header
            ui.label(
                egui::RichText::new(format!(" {} ", title))
                    .color(theme.primary())
                    .monospace()
                    .strong()
                    .size(13.0),
            );
            ui.add_space(2.0);
            sep(ui, theme);
            ui.add_space(4.0);
            content(ui);
        });
    open
}

// ── Providers overlay ───────────────────────────────────────────────────

pub fn render_providers_overlay(
    ctx: &egui::Context,
    providers: &[crate::api::ProviderInfo],
    loading: bool,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result = None;
    let open = overlay_frame(ctx, "AI Providers", 480.0, 320.0, theme, |ui| {
        if loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new("Loading…")
                        .color(theme.text_dim())
                        .monospace()
                        .size(11.0),
                );
            });
            return;
        }
        if providers.is_empty() {
            ui.label(
                egui::RichText::new("No providers configured.")
                    .color(theme.text_muted())
                    .monospace()
                    .size(11.0),
            );
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for p in providers {
                ui.horizontal(|ui| {
                    let status_color = if p.is_connected {
                        theme.success()
                    } else {
                        theme.error()
                    };
                    ui.label(egui::RichText::new("●").color(status_color).size(10.0));
                    ui.label(
                        egui::RichText::new(&p.name)
                            .color(theme.text_primary())
                            .monospace()
                            .strong()
                            .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(format!("({} models)", p.model_count))
                            .color(theme.text_dim())
                            .monospace()
                            .size(10.0),
                    );
                });
            }
        });
    });
    if !open {
        result = Some(AppAction::CloseProvidersOverlay);
    }
    result
}

// ── Permissions overlay ─────────────────────────────────────────────────

pub fn render_permissions_overlay(
    ctx: &egui::Context,
    current_mode: &str,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result = None;
    let modes = ["default", "acceptEdits", "plan", "bypassPermissions"];
    let open = overlay_frame(ctx, "Permission Mode", 400.0, 240.0, theme, |ui| {
        ui.label(
            egui::RichText::new("Select tool approval mode:")
                .color(theme.text_muted())
                .monospace()
                .size(11.0),
        );
        ui.add_space(4.0);
        for mode in &modes {
            let is_active = current_mode == *mode;
            let prefix = if is_active { "● " } else { "○ " };
            let color = if is_active {
                theme.primary()
            } else {
                theme.text_primary()
            };
            let desc = match *mode {
                "default" => "Ask before each tool execution",
                "acceptEdits" => "Auto-approve edits, ask for others",
                "plan" => "Read-only mode, no tool execution",
                "bypassPermissions" => "Auto-approve all tools (YOLO)",
                _ => "",
            };
            if ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(format!("{prefix}{mode}"))
                            .color(color)
                            .monospace()
                            .strong()
                            .size(12.0),
                    )
                    .sense(egui::Sense::click()),
                )
                .clicked()
                && !is_active
            {
                result = Some(AppAction::SetPermissionMode(mode.to_string()));
            }
            ui.label(
                egui::RichText::new(format!("  {desc}"))
                    .color(theme.text_dim())
                    .monospace()
                    .size(10.0),
            );
        }
    });
    if !open && result.is_none() {
        result = Some(AppAction::ClosePermissionsOverlay);
    }
    result
}

// ── Theme overlay ───────────────────────────────────────────────────────

pub fn render_theme_overlay(
    ctx: &egui::Context,
    available_themes: &[String],
    current_theme: &str,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result = None;
    let open = overlay_frame(ctx, "Themes", 400.0, 300.0, theme, |ui| {
        if available_themes.is_empty() {
            ui.label(
                egui::RichText::new("No themes found.")
                    .color(theme.text_muted())
                    .monospace()
                    .size(11.0),
            );
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for name in available_themes {
                let is_active = name == current_theme;
                let prefix = if is_active { "● " } else { "○ " };
                let color = if is_active {
                    theme.primary()
                } else {
                    theme.text_primary()
                };
                if ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(format!("{prefix}{name}"))
                                .color(color)
                                .monospace()
                                .strong()
                                .size(12.0),
                        )
                        .sense(egui::Sense::click()),
                    )
                    .clicked()
                    && !is_active
                {
                    result = Some(AppAction::SetTheme(name.clone()));
                }
            }
        });
    });
    if !open && result.is_none() {
        result = Some(AppAction::CloseThemeOverlay);
    }
    result
}

// ── Hooks overlay ───────────────────────────────────────────────────────

pub fn render_hooks_overlay(
    ctx: &egui::Context,
    hooks: &[crate::api::HookInfo],
    loading: bool,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result = None;
    let open = overlay_frame(ctx, "Session Hooks", 480.0, 280.0, theme, |ui| {
        if loading {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new("Loading…")
                        .color(theme.text_dim())
                        .monospace()
                        .size(11.0),
                );
            });
            return;
        }
        if hooks.is_empty() {
            ui.label(
                egui::RichText::new("No hooks configured.")
                    .color(theme.text_muted())
                    .monospace()
                    .size(11.0),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Add hooks in .cade/settings.json → \"hooks\" section.")
                    .color(theme.text_dim())
                    .monospace()
                    .size(10.0),
            );
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for hook in hooks {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&hook.event)
                            .color(theme.primary())
                            .monospace()
                            .strong()
                            .size(11.0),
                    );
                    ui.label(egui::RichText::new("→").color(theme.text_dim()).size(11.0));
                    ui.label(
                        egui::RichText::new(&hook.command)
                            .color(theme.text_primary())
                            .monospace()
                            .size(11.0),
                    );
                });
            }
        });
    });
    if !open && result.is_none() {
        result = Some(AppAction::CloseHooksOverlay);
    }
    result
}

// ── Toolset overlay ─────────────────────────────────────────────────────

pub fn render_toolset_overlay(
    ctx: &egui::Context,
    current_toolset: &str,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result = None;
    let toolsets = ["default", "codex", "gemini"];
    let open = overlay_frame(ctx, "Active Toolset", 360.0, 200.0, theme, |ui| {
        for ts in &toolsets {
            let is_active = current_toolset == *ts;
            let prefix = if is_active { "● " } else { "○ " };
            let color = if is_active {
                theme.primary()
            } else {
                theme.text_primary()
            };
            if ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(format!("{prefix}{ts}"))
                            .color(color)
                            .monospace()
                            .strong()
                            .size(12.0),
                    )
                    .sense(egui::Sense::click()),
                )
                .clicked()
                && !is_active
            {
                result = Some(AppAction::SetToolset(ts.to_string()));
            }
        }
    });
    if !open && result.is_none() {
        result = Some(AppAction::CloseToolsetOverlay);
    }
    result
}

// ── Pricing overlay ─────────────────────────────────────────────────────

pub fn render_pricing_overlay(
    ctx: &egui::Context,
    pricing_info: &str,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result = None;
    let open = overlay_frame(ctx, "Token Pricing", 460.0, 280.0, theme, |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for line in pricing_info.lines() {
                ui.label(
                    egui::RichText::new(line)
                        .color(theme.text_primary())
                        .monospace()
                        .size(11.0),
                );
            }
        });
    });
    if !open && result.is_none() {
        result = Some(AppAction::ClosePricingOverlay);
    }
    result
}

// ── Backend overlay ─────────────────────────────────────────────────────

pub fn render_backend_overlay(
    ctx: &egui::Context,
    current_backend: &str,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result = None;
    let backends = ["local", "docker", "ssh"];
    let open = overlay_frame(ctx, "Execution Backend", 360.0, 200.0, theme, |ui| {
        for be in &backends {
            let is_active = current_backend == *be;
            let prefix = if is_active { "● " } else { "○ " };
            let color = if is_active {
                theme.primary()
            } else {
                theme.text_primary()
            };
            if ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(format!("{prefix}{be}"))
                            .color(color)
                            .monospace()
                            .strong()
                            .size(12.0),
                    )
                    .sense(egui::Sense::click()),
                )
                .clicked()
                && !is_active
            {
                result = Some(AppAction::SetBackend(be.to_string()));
            }
        }
    });
    if !open && result.is_none() {
        result = Some(AppAction::CloseBackendOverlay);
    }
    result
}

// ── Reasoning overlay ───────────────────────────────────────────────────

pub fn render_reasoning_overlay(
    ctx: &egui::Context,
    current_effort: &str,
    theme: &crate::theme::ThemeColors,
) -> Option<AppAction> {
    let mut result = None;
    let tiers = [
        ("none", "No explicit reasoning budget (default)"),
        ("low", "Low reasoning effort (~25% budget)"),
        ("medium", "Medium reasoning effort (~50% budget)"),
        ("high", "High reasoning effort (~75% budget)"),
        ("xhigh", "Maximum reasoning effort (~100% budget)"),
    ];
    let open = overlay_frame(ctx, "Reasoning Effort", 440.0, 280.0, theme, |ui| {
        ui.label(
            egui::RichText::new(
                "Controls extended thinking / reasoning budget for supported models:",
            )
            .color(theme.text_muted())
            .monospace()
            .size(10.0),
        );
        ui.add_space(4.0);
        for (level, desc) in &tiers {
            let is_active = current_effort == *level;
            let prefix = if is_active { "● " } else { "○ " };
            let color = if is_active {
                theme.primary()
            } else {
                theme.text_primary()
            };
            if ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(format!("{prefix}{level}"))
                            .color(color)
                            .monospace()
                            .strong()
                            .size(12.0),
                    )
                    .sense(egui::Sense::click()),
                )
                .clicked()
                && !is_active
            {
                result = Some(AppAction::SetReasoning(level.to_string()));
            }
            ui.label(
                egui::RichText::new(format!("  {desc}"))
                    .color(theme.text_dim())
                    .monospace()
                    .size(10.0),
            );
        }
    });
    if !open && result.is_none() {
        result = Some(AppAction::CloseReasoningOverlay);
    }
    result
}
