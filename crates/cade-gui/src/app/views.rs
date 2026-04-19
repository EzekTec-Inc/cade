//! Standalone view functions: welcome screen and timeline message renderer.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::AppAction;

pub fn render_welcome(ui: &mut egui::Ui, md_cache: &mut egui_commonmark::CommonMarkCache,
    theme: &crate::theme::ThemeColors, 
) {
    ui.add_space(24.0);
    // dim horizontal rule
    ui.add(egui::Separator::default().horizontal().spacing(0.0));
    ui.add_space(16.0);

    ui.label(
        egui::RichText::new("CADE")
            .color(theme.primary())
            .strong()
            .size(15.0),
    );
    ui.add_space(6.0);
    egui_commonmark::CommonMarkViewer::new().show(
        ui,
        md_cache,
        "Connected and ready. Select an agent from the sidebar to begin.\n\n\
        - **Chat** with any configured agent\n\
        - View *streaming* responses in real time\n\
        - Inspect tool calls, reasoning, and results\n\
        - Use `/` or `Ctrl+K` to open the command palette",
    );
    ui.add_space(16.0);
    ui.add(egui::Separator::default().horizontal().spacing(0.0));
}

/// Render one timeline message.
/// Returns an `AppAction` only when a user interaction requires it.
///
/// Visual language mirrors the TUI — all items left-aligned, no bubbles:
/// User: "You" bold + plain text; Assistant: "▍ CADE" primary bold + markdown;
/// tool_call: "⚙ name(args…)" collapsible; tool: "│ OK/ERR content";
/// reasoning: "╭ THINKING N words" collapsible; system: " INFO  text".
pub fn render_timeline_message(
    ui: &mut egui::Ui,
    md_cache: &mut egui_commonmark::CommonMarkCache,
    msg: &cade_api_types::ChatMessage,
    theme: &crate::theme::ThemeColors, 
) -> Option<AppAction> {
    match msg.role.as_str() {

        // ── User ─────────────────────────────────────────────────────
        "user" => {
            let text = msg.content.as_str().unwrap_or("").trim().to_string();
            // "You" label
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("You")
                    .color(theme.text_primary())
                    .strong()
                    .size(13.0),
            );
            // Plain text — no markdown rendering for user messages (mirrors TUI)
            ui.label(
                egui::RichText::new(&text)
                    .color(theme.text_primary())
                    .size(13.0),
            );
            None
        }

        // ── Assistant ─────────────────────────────────────────────────
        "assistant" => {
            let text = msg.content.as_str().unwrap_or("").trim().to_string();
            ui.add_space(4.0);
            // "▍ CADE" header
            ui.label(
                egui::RichText::new("▍ CADE")
                    .color(theme.primary())
                    .strong()
                    .size(13.0),
            );
            // Markdown-rendered content
            egui_commonmark::CommonMarkViewer::new().show(ui, md_cache, &text);
            None
        }

        // ── Reasoning ─────────────────────────────────────────────────
        "reasoning" => {
            let text = msg.content.as_str().unwrap_or("");
            let word_count = text.split_whitespace().count();
            let is_streaming = msg.id.is_empty();
            let header = if is_streaming {
                format!("╭ THINKING  {} words", word_count)
            } else {
                format!("╭ THINKING  {} words · collapsed", word_count)
            };

            ui.add_space(4.0);
            egui::CollapsingHeader::new(
                egui::RichText::new(header)
                    .color(theme.text_muted())
                    .italics()
                    .size(12.0),
            )
            .id_salt(format!("reasoning_{}", msg.id))
            .default_open(false)
            .show(ui, |ui| {
                for ln in text.lines() {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("│ ")
                                .color(theme.border_base())
                                .size(12.0),
                        );
                        ui.label(
                            egui::RichText::new(ln)
                                .color(theme.text_muted())
                                .italics()
                                .size(12.0),
                        );
                    });
                }
            });
            None
        }

        // ── Tool call ─────────────────────────────────────────────────
        "tool_call" => {
            let name = msg.content.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
            let args_raw = msg.content.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");

            // One-line preview: truncate args to ~60 chars
            let preview: String = args_raw
                .replace('\n', " ")
                .chars()
                .take(60)
                .collect();
            let preview_suffix = if args_raw.len() > 60 { "…" } else { "" };

            ui.add_space(2.0);
            // Single-line invocation row: "⚙ name(args…)"
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("⚙ {}({preview}{preview_suffix})", name))
                    .color(theme.primary())
                    .strong()
                    .monospace()
                    .size(12.0),
            )
            .id_salt(format!("tc_{}", msg.id))
            .default_open(false)
            .show(ui, |ui| {
                // Pretty-print full args
                let pretty = serde_json::from_str::<serde_json::Value>(args_raw)
                    .ok()
                    .and_then(|v| serde_json::to_string_pretty(&v).ok())
                    .unwrap_or_else(|| args_raw.to_string());
                for ln in pretty.lines() {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("│ ")
                                .color(theme.border_base())
                                .size(11.0),
                        );
                        ui.label(
                            egui::RichText::new(ln)
                                .color(theme.text_dim())
                                .monospace()
                                .size(11.0),
                        );
                    });
                }
            });
            None
        }

        // ── Tool result ───────────────────────────────────────────────
        "tool" => {
            let content = msg.content.as_str().unwrap_or("").trim();

            // Detect error by checking if content starts with known error prefixes
            let is_error = content.starts_with("Error")
                || content.starts_with("error")
                || content.starts_with("ERR");

            let (status_label, status_color) = if is_error {
                (" ERR ", theme.error())
            } else {
                (" OK ", theme.success())
            };

            let lines: Vec<&str> = content.lines().collect();
            let show_limit = 3usize;

            ui.add_space(2.0);
            egui::CollapsingHeader::new({
                // Header: "│ ✓ OK  <first line>"
                let first = lines.first().copied().unwrap_or("(no output)");
                let first_trunc: String = first.chars().take(72).collect();
                let suffix = if first.len() > 72 { "…" } else { "" };
                egui::RichText::new(format!("│ {status_label}  {first_trunc}{suffix}"))
                    .color(status_color)
                    .monospace()
                    .size(12.0)
            })
            .id_salt(format!("tr_{}", msg.id))
            .default_open(false)
            .show(ui, |ui| {
                let to_show = lines.len();
                for (i, ln) in lines.iter().take(to_show).enumerate() {
                    if i == 0 { continue; } // already shown in header
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("│      ")
                                .color(theme.border_base())
                                .monospace()
                                .size(11.0),
                        );
                        ui.label(
                            egui::RichText::new(*ln)
                                .color(status_color)
                                .monospace()
                                .size(11.0),
                        );
                    });
                }
                let _ = show_limit; // suppress unused warning
            });
            None
        }

        // ── System ────────────────────────────────────────────────────
        "system" => {
            let text = msg.content.as_str().unwrap_or("");
            ui.add_space(2.0);
            for (i, ln) in text.lines().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(if i == 0 { " INFO " } else { "      " })
                            .color(theme.primary())
                            .background_color(theme.bg_base())
                            .strong()
                            .size(11.0),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(ln)
                            .color(theme.text_muted())
                            .size(12.0),
                    );
                });
            }
            None
        }

        // ── Fallback ─────────────────────────────────────────────────
        role => {
            let text = msg.content.as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| msg.content.to_string());
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(" {role} "))
                        .color(theme.text_dim())
                        .monospace()
                        .size(11.0),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(&text)
                        .color(theme.text_muted())
                        .size(12.0),
                );
            });
            None
        }
    }
}

