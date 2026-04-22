//! Standalone view functions: welcome screen and timeline message renderer.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::AppAction;

pub fn render_welcome(ui: &mut egui::Ui, md_cache: &mut egui_commonmark::CommonMarkCache,
    theme: &crate::theme::ThemeColors, 
) {
    ui.add_space(8.0);
    // dim horizontal rule
    ui.add(egui::Separator::default().horizontal().spacing(0.0));
    ui.add_space(4.0);

    ui.label(
        egui::RichText::new("CADE")
            .color(theme.primary())
            .strong()
            .monospace()
            .size(13.0),
    );
    ui.add_space(2.0);
    egui_commonmark::CommonMarkViewer::new().show(
        ui,
        md_cache,
        "Connected and ready. Select an agent from the sidebar to begin.\n\n\
        - **Chat** with any configured agent\n\
        - View *streaming* responses in real time\n\
        - Inspect tool calls, reasoning, and results\n\
        - Use `/` or `Ctrl+K` to open the command palette",
    );
    ui.add_space(4.0);
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
    let tool_icon = |name: &str| -> &'static str {
        match name {
            // -- Shell / process
            "bash" | "shell" | "run_command" | "execute_command"
            | "start_process" | "RunShellCommand" => "\u{f120}",  // 
            
            // -- File read
            "read_file" | "ReadFileGemini" | "read_multiple_files" => "\u{f15c}",  // 
            
            // -- File write / edit
            "write_file" | "edit_file" | "create_file" | "edit_block"
            | "replace_in_file" => "\u{f0f6}",  // 
            
            // -- Patch / diff
            "apply_patch" | "ide_apply_patch" => "\u{f440}",  // 
            
            // -- Search / grep
            "grep" | "grep_search" | "GlobGemini" | "SearchFileContent"
            | "start_search" | "find_references" | "symbol_search" => "\u{f002}",  // 
            
            // -- Directory / glob
            "list_directory" | "glob" | "get_file_info" => "\u{f07b}",  // 
            
            // -- Git
            "commit" | "push" | "pull" | "branch" | "merge" | "rebase_op"
            | "stash_op" | "log" | "diff" | "status" | "add" | "reset"
            | "restore" | "fetch" | "remote" | "tag" | "show" | "blame"
            | "cherry_pick" | "clean" | "revert" | "config"
            | "repository" => "\u{e725}",  // 
            
            // -- GitHub
            "create_pull_request" | "create_issue" | "list_issues"
            | "search_issues" | "search_code" | "get_issue"
            | "add_issue_comment" | "list_commits" | "get_file_contents"
            | "get_repository" | "create_branch" | "search_repositories"
            | "update_issue" | "get_user" => "\u{f09b}",  // 
            
            // -- Memory / knowledge
            "update_memory" | "memory_apply_patch" | "search_memory"
            | "conversation_search" | "archival_memory_insert"
            | "archival_memory_search" | "update_memory_typed"
            | "link_memory_evidence" | "reflect" => "\u{f0eb}",  // 
            
            // -- Skills
            "load_skill" | "install_skill" | "run_skill_script"
            | "load_skill_ref" => "\u{f085}",  // 
            
            // -- Subagents
            "run_subagent" | "list_agents" | "message_agent" => "\u{f0c0}",  // 
            
            // -- Plan / task
            "EnterPlanMode" | "ExitPlanMode" | "TodoWrite"
            | "UpdatePlan" | "WriteTodos" | "set_plan" | "workflow" => "\u{f0ae}",  // 
            
            // -- Checkpoints / artifacts
            "create_checkpoint" | "restore_checkpoint"
            | "list_checkpoints" | "store_artifact" => "\u{f0c7}",  // 
            
            // -- Web / network
            "web_search" | "fetch_doc" | "browser_screenshot"
            | "http_request" | "get-library-docs"
            | "resolve-library-id" => "\u{f0ac}",  // 
            
            // -- Desktop
            "screen_capture" | "desktop_screenshot" | "list_windows"
            | "desktop_list_windows" | "desktop_control"
            | "image_processor" => "\u{f108}",  // 
            
            // -- Default
            _ => "\u{f0ad}",  //  (wrench — generic tool)
        }
    };

    match msg.role.as_str() {

        // ── User ─────────────────────────────────────────────────────
        "user" => {
            let text = msg.content.as_str().unwrap_or("").trim().to_string();
            // "You" label
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("You")
                    .color(theme.text_primary())
                    .strong()
                    .monospace()
                    .size(12.0),
            );
            // Plain text — no markdown rendering for user messages (mirrors TUI)
            ui.label(
                egui::RichText::new(&text)
                    .color(theme.text_primary())
                    .size(12.0),
            );
            None
        }

        // ── Assistant ─────────────────────────────────────────────────
        "assistant" => {
            let text = msg.content.as_str().unwrap_or("").trim().to_string();
            ui.add_space(2.0);
            // "▍ CADE" header
            ui.label(
                egui::RichText::new("▍ CADE")
                    .color(theme.primary())
                    .strong()
                    .monospace()
                    .size(12.0),
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

            ui.add_space(1.0);
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

            ui.add_space(1.0);
            // Single-line invocation row: "⚙ name(args…)"
            let icon = tool_icon(name);
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("{icon} {}({preview}{preview_suffix})", name))
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
                ("\u{f057}", theme.error()) // 
            } else {
                ("\u{f058}", theme.success()) // 
            };

            let lines: Vec<&str> = content.lines().collect();
            let show_limit = 3usize;

            ui.add_space(1.0);
            egui::CollapsingHeader::new({
                // Header: "│ ✓ <first line>"
                let first = lines.first().copied().unwrap_or("(no output)");
                let first_trunc: String = first.chars().take(72).collect();
                let suffix = if first.len() > 72 { "…" } else { "" };
                egui::RichText::new(format!("│ {status_label} {first_trunc}{suffix}"))
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
            ui.add_space(1.0);
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

/// Render a live-output block in the timeline.
///
/// Mirrors the TUI's `LiveOutput` render: a fixed-height scrollable area
/// showing streaming output lines from a long-running tool execution.
pub fn render_live_output(
    ui: &mut egui::Ui,
    block: &crate::session::LiveOutputBlock,
    theme: &crate::theme::ThemeColors,
) {
    let status = if block.done { "done" } else { "running…" };
    let status_color = if block.done { theme.text_muted() } else { theme.warning() };

    ui.add_space(1.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("┃ {} · {}", block.tool_name, status))
                .color(status_color)
                .monospace()
                .size(11.0),
        );
        if !block.done {
            ui.spinner();
        }
    });

    // Scrollable output area — show last N lines
    let max_h = (block.max_visible.max(4) as f32) * 16.0;
    egui::ScrollArea::vertical()
        .id_salt(format!("live_{}", block.call_id))
        .max_height(max_h)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for ln in &block.lines {
                ui.label(
                    egui::RichText::new(ln.as_str())
                        .color(theme.text_dim())
                        .monospace()
                        .size(11.0),
                );
            }
        });
}

// ── Context bar (per-category breakdown) ──────────────────────────────

/// Category glyphs matching TUI's ContextBar (indices 0–7).
const CATEGORY_GLYPHS: [&str; 8] = ["█", "▓", "▒", "░", "▪", "■", "·", "⎹"];

/// Category colors — returns a color for each category index.
fn category_color(idx: usize, theme: &crate::theme::ThemeColors) -> egui::Color32 {
    match idx {
        0 => theme.primary(),     // system
        1 => theme.teal(),        // tools
        2 => theme.purple(),      // mcp
        3 => theme.warning(),     // memory
        4 => theme.success(),     // skills
        5 => theme.text_primary(),// messages
        6 => theme.text_dim(),    // free
        7 => theme.text_muted(),  // buffer
        _ => theme.text_dim(),
    }
}

/// Render a per-category context-window bar chart, mirroring TUI's `ContextBar`.
///
/// Layout:
///   Header: model · pct% (used/window tokens)
///   Bar:    proportional colored segments
///   Legend: one row per non-zero category
pub fn render_context_bar(
    ui: &mut egui::Ui,
    breakdown: &crate::api::ContextBreakdown,
    theme: &crate::theme::ThemeColors,
) {
    let pct_color = if breakdown.pct >= 90 {
        theme.error()
    } else if breakdown.pct >= 60 {
        theme.warning()
    } else {
        theme.text_primary()
    };

    let total_used: u64 = breakdown
        .categories
        .iter()
        .filter(|c| c.name != "free" && c.name != "buffer")
        .map(|c| c.tokens)
        .sum();

    // Header line
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} · {}% ({}/{})",
                breakdown.model,
                breakdown.pct,
                format_tok_compact(total_used),
                format_tok_compact(breakdown.window_tokens),
            ))
            .color(pct_color)
            .monospace()
            .strong()
            .size(11.0),
        );
    });

    ui.add_space(2.0);

    // Proportional bar — painted as adjacent rects
    if breakdown.window_tokens > 0 {
        let bar_w = ui.available_width();
        let bar_h = 12.0;
        let (resp, painter) = ui.allocate_painter(egui::vec2(bar_w, bar_h), egui::Sense::hover());
        let r = resp.rect;

        // Background
        painter.rect_filled(r, 0.0, theme.bg_surface0());

        // Segments
        let mut x = r.min.x;
        for (idx, cat) in breakdown.categories.iter().enumerate() {
            if cat.tokens == 0 {
                continue;
            }
            let frac = cat.tokens as f32 / breakdown.window_tokens as f32;
            let seg_w = (frac * bar_w).max(1.0);
            let seg_rect = egui::Rect::from_min_size(
                egui::pos2(x, r.min.y),
                egui::vec2(seg_w, bar_h),
            );
            painter.rect_filled(seg_rect, 0.0, category_color(idx, theme));
            x += seg_w;
        }
    }

    ui.add_space(2.0);

    // Legend rows (non-zero only)
    for (idx, cat) in breakdown.categories.iter().enumerate() {
        if cat.tokens == 0 {
            continue;
        }
        let pct_cat = if breakdown.window_tokens > 0 {
            (cat.tokens as f64 / breakdown.window_tokens as f64 * 100.0) as u8
        } else {
            0
        };
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(*CATEGORY_GLYPHS.get(idx).unwrap_or(&"?"))
                    .color(category_color(idx, theme))
                    .monospace()
                    .size(10.0),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{:<8} {:>6}  {}%",
                    cat.name,
                    format_tok_compact(cat.tokens),
                    pct_cat,
                ))
                .color(theme.text_muted())
                .monospace()
                .size(10.0),
            );
        });
    }
}

fn format_tok_compact(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{}k", n / 1_000)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

