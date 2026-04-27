//! Standalone view functions: welcome screen and timeline message renderer.

use crate::theme::EguiThemeExt;
use eframe::egui;

use super::AppAction;

pub fn render_welcome(
    ui: &mut egui::Ui,
    md_cache: &mut egui_commonmark::CommonMarkCache,
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
        - Use `/` or `Ctrl+P` to open the command palette",
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
    let tool_icon =
        |name: &str| -> &'static str {
            match name {
                // -- Shell / process
                "bash" | "shell" | "run_command" | "execute_command" | "start_process"
                | "RunShellCommand" => "\u{f120}", //

                // -- File read
                "read_file" | "ReadFileGemini" | "read_multiple_files" => "\u{f15c}", //

                // -- File write / edit
                "write_file" | "edit_file" | "create_file" | "edit_block" | "replace_in_file" => {
                    "\u{f0f6}"
                } //

                // -- Patch / diff
                "apply_patch" | "ide_apply_patch" => "\u{f440}", //

                // -- Search / grep
                "grep" | "grep_search" | "GlobGemini" | "SearchFileContent" | "start_search"
                | "find_references" | "symbol_search" => "\u{f002}", //

                // -- Directory / glob
                "list_directory" | "glob" | "get_file_info" => "\u{f07b}", //

                // -- Git
                "commit" | "push" | "pull" | "branch" | "merge" | "rebase_op" | "stash_op"
                | "log" | "diff" | "status" | "add" | "reset" | "restore" | "fetch" | "remote"
                | "tag" | "show" | "blame" | "cherry_pick" | "clean" | "revert" | "config"
                | "repository" => "\u{e725}", //

                // -- GitHub
                "create_pull_request"
                | "create_issue"
                | "list_issues"
                | "search_issues"
                | "search_code"
                | "get_issue"
                | "add_issue_comment"
                | "list_commits"
                | "get_file_contents"
                | "get_repository"
                | "create_branch"
                | "search_repositories"
                | "update_issue"
                | "get_user" => "\u{f09b}", //

                // -- Memory / knowledge
                "update_memory"
                | "memory_apply_patch"
                | "search_memory"
                | "conversation_search"
                | "archival_memory_insert"
                | "archival_memory_search"
                | "update_memory_typed"
                | "link_memory_evidence"
                | "reflect" => "\u{f0eb}", //

                // -- Skills
                "load_skill" | "install_skill" | "run_skill_script" | "load_skill_ref"
                | "unload_skill" => "\u{f085}", //

                // -- Subagents
                "run_subagent" | "list_agents" | "message_agent" => "\u{f0c0}", //

                // -- Plan / task
                "EnterPlanMode" | "ExitPlanMode" | "TodoWrite" | "UpdatePlan" | "WriteTodos"
                | "set_plan" | "workflow" => "\u{f0ae}", //

                // -- Checkpoints / artifacts
                "create_checkpoint" | "restore_checkpoint" | "list_checkpoints"
                | "store_artifact" => "\u{f0c7}", //

                // -- Web / network
                "web_search" | "fetch_doc" | "browser_screenshot" | "http_request"
                | "get-library-docs" | "resolve-library-id" => "\u{f0ac}", //

                // -- Desktop
                "screen_capture"
                | "desktop_screenshot"
                | "list_windows"
                | "desktop_list_windows"
                | "desktop_control"
                | "image_processor" => "\u{f108}", //

                // -- Default
                _ => "\u{f0ad}", //  (wrench — generic tool)
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
            // Width-constrained markdown rendering
            let max_w = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::vec2(max_w, 0.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_max_width(max_w);
                    egui_commonmark::CommonMarkViewer::new()
                        .default_width(Some(max_w as usize))
                        .max_image_width(Some((max_w * 0.9) as usize))
                        .syntax_theme_dark("base16-ocean.dark")
                        .syntax_theme_light("base16-ocean.light")
                        .show(ui, md_cache, &text);
                },
            );
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
            let name = msg
                .content
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let args_raw = msg
                .content
                .get("arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let args: serde_json::Value = serde_json::from_str(args_raw).unwrap_or_default();

            ui.add_space(1.0);
            let icon = tool_icon(name);

            match name {
                // ── Edit / replace: file badge + inline diff ──────
                "edit_file"
                | "replace_in_file"
                | "Replace"
                | "desktop-commander__edit_block"
                | "cade-nvim__ide_propose_edit" => {
                    let path = args["path"].as_str().unwrap_or("unknown");
                    let old = args["old_string"]
                        .as_str()
                        .or_else(|| args["old_str"].as_str())
                        .unwrap_or("");
                    let new = args["new_string"]
                        .as_str()
                        .or_else(|| args["new_str"].as_str())
                        .unwrap_or("");

                    let header = format!("{icon} edit  {}", short_path(path));
                    egui::CollapsingHeader::new(
                        egui::RichText::new(&header)
                            .color(theme.primary())
                            .strong()
                            .monospace()
                            .size(12.0),
                    )
                    .id_salt(format!("tc_{}", msg.id))
                    .default_open(true)
                    .show(ui, |ui| {
                        render_inline_diff(ui, old, new, theme);
                    });
                }

                // ── Write file: file badge + content preview ──────
                "write_file"
                | "WriteFileGemini"
                | "create_file"
                | "desktop-commander__write_file"
                | "developer__write_file" => {
                    let path = args["path"]
                        .as_str()
                        .or_else(|| args["file_path"].as_str())
                        .unwrap_or("unknown");
                    let content = args["content"]
                        .as_str()
                        .or_else(|| args["file_text"].as_str())
                        .unwrap_or("");
                    let line_count = content.lines().count();

                    let header =
                        format!("{icon} write  {} ({} lines)", short_path(path), line_count);
                    egui::CollapsingHeader::new(
                        egui::RichText::new(&header)
                            .color(theme.success())
                            .strong()
                            .monospace()
                            .size(12.0),
                    )
                    .id_salt(format!("tc_{}", msg.id))
                    .default_open(false)
                    .show(ui, |ui| {
                        render_code_preview(ui, content, 30, theme);
                    });
                }

                // ── Apply patch: rendered diff ────────────────────
                "apply_patch" | "cade-nvim__ide_apply_patch" => {
                    let patch = args["patch"].as_str().unwrap_or("");
                    let file_count = patch.matches("--- a/").count().max(1);

                    let header = format!(
                        "{icon} patch  ({} file{})",
                        file_count,
                        if file_count != 1 { "s" } else { "" }
                    );
                    egui::CollapsingHeader::new(
                        egui::RichText::new(&header)
                            .color(theme.warning())
                            .strong()
                            .monospace()
                            .size(12.0),
                    )
                    .id_salt(format!("tc_{}", msg.id))
                    .default_open(true)
                    .show(ui, |ui| {
                        render_patch(ui, patch, theme);
                    });
                }

                // ── Read file: file badge + range ─────────────────
                "read_file"
                | "ReadFileGemini"
                | "developer__read_file"
                | "desktop-commander__read_file" => {
                    let path = args["path"].as_str().unwrap_or("unknown");
                    let offset = args["offset"]
                        .as_u64()
                        .or_else(|| args["start_line"].as_u64());
                    let limit = args["limit"].as_u64().or_else(|| args["end_line"].as_u64());

                    let range_str = match (offset, limit) {
                        (Some(o), Some(l)) => format!(" L{}–{}", o, l),
                        (Some(o), None) => format!(" L{}+", o),
                        _ => String::new(),
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{icon} read  {}{}",
                                short_path(path),
                                range_str
                            ))
                            .color(theme.text_primary())
                            .monospace()
                            .size(12.0),
                        );
                    });
                }

                // ── Bash / shell: command badge ───────────────────
                "bash"
                | "shell"
                | "run_command"
                | "execute_command"
                | "RunShellCommand"
                | "developer__shell"
                | "desktop-commander__start_process"
                | "developer__start_process" => {
                    let cmd = args["command"].as_str().unwrap_or("…");
                    let cmd_preview: String = cmd.chars().take(120).collect();
                    let suffix = if cmd.len() > 120 { "…" } else { "" };

                    egui::CollapsingHeader::new(
                        egui::RichText::new(format!("{icon} $ {cmd_preview}{suffix}"))
                            .color(theme.teal())
                            .strong()
                            .monospace()
                            .size(12.0),
                    )
                    .id_salt(format!("tc_{}", msg.id))
                    .default_open(false)
                    .show(ui, |ui| {
                        if cmd.len() > 120 {
                            render_code_preview(ui, cmd, 10, theme);
                        }
                    });
                }

                // ── Glob / grep: search badge ─────────────────────
                "glob" | "GlobGemini" | "developer__glob" => {
                    let pattern = args["pattern"].as_str().unwrap_or("*");
                    let path = args["path"].as_str().unwrap_or(".");
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{icon} glob  {} in {}",
                                pattern,
                                short_path(path)
                            ))
                            .color(theme.text_primary())
                            .monospace()
                            .size(12.0),
                        );
                    });
                }

                "grep" | "SearchFileContent" | "developer__grep_search" => {
                    let pattern = args["pattern"].as_str().unwrap_or("…");
                    let path = args["path"].as_str().unwrap_or(".");
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{icon} grep  /{}/  in {}",
                                pattern,
                                short_path(path)
                            ))
                            .color(theme.text_primary())
                            .monospace()
                            .size(12.0),
                        );
                    });
                }

                // ── Git commands: operation badge ─────────────────
                "git__commit" | "git__add" | "git__push" | "git__pull" | "git__branch"
                | "git__diff" | "git__status" | "git__log" | "git__stash_op" | "git__merge"
                | "git__rebase_op" | "git__reset" | "git__restore" | "git__tag" | "git__show"
                | "git__fetch" | "git__blame" | "git__cherry_pick" | "git__clean"
                | "git__revert" | "git__config" | "git__remote" | "git__repository" => {
                    let op = name.strip_prefix("git__").unwrap_or(name);
                    let summary = match op {
                        "commit" => args["message"]
                            .as_str()
                            .map(|m| {
                                let s: String = m.chars().take(72).collect();
                                format!("\"{}\"", s)
                            })
                            .unwrap_or_default(),
                        "add" => args["files"]
                            .as_array()
                            .map(|f| format!("{} files", f.len()))
                            .unwrap_or_else(|| "all".into()),
                        "branch" => args["name"].as_str().unwrap_or("").to_string(),
                        "diff" => {
                            let staged = args["staged"].as_bool().unwrap_or(false);
                            if staged {
                                "staged".into()
                            } else {
                                "working tree".into()
                            }
                        }
                        _ => String::new(),
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{icon} git {op}  {summary}"))
                                .color(theme.primary())
                                .monospace()
                                .size(12.0),
                        );
                    });
                }

                // ── Default: fallback to compact JSON ─────────────
                _ => {
                    let preview: String = args_raw.replace('\n', " ").chars().take(60).collect();
                    let suffix = if args_raw.len() > 60 { "…" } else { "" };

                    egui::CollapsingHeader::new(
                        egui::RichText::new(format!("{icon} {}({preview}{suffix})", name))
                            .color(theme.primary())
                            .strong()
                            .monospace()
                            .size(12.0),
                    )
                    .id_salt(format!("tc_{}", msg.id))
                    .default_open(false)
                    .show(ui, |ui| {
                        let pretty = serde_json::to_string_pretty(&args)
                            .unwrap_or_else(|_| args_raw.to_string());
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
                }
            }
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
                    if i == 0 {
                        continue;
                    } // already shown in header
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
                    ui.label(egui::RichText::new(ln).color(theme.text_muted()).size(12.0));
                });
            }
            None
        }

        // ── Fallback ─────────────────────────────────────────────────
        role => {
            let text = msg
                .content
                .as_str()
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
    let status_color = if block.done {
        theme.text_muted()
    } else {
        theme.warning()
    };

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
        0 => theme.primary(),      // system
        1 => theme.teal(),         // tools
        2 => theme.purple(),       // mcp
        3 => theme.warning(),      // memory
        4 => theme.success(),      // skills
        5 => theme.text_primary(), // messages
        6 => theme.text_dim(),     // free
        7 => theme.text_muted(),   // buffer
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
            let seg_rect =
                egui::Rect::from_min_size(egui::pos2(x, r.min.y), egui::vec2(seg_w, bar_h));
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

// ── Rich tool call helpers ────────────────────────────────────────────────────

/// Shorten a file path for display: show last 2 components.
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 3 {
        path.to_string()
    } else {
        format!("…/{}", parts[parts.len() - 2..].join("/"))
    }
}

/// Render an inline diff view: `- old` lines in red, `+ new` lines in green.
fn render_inline_diff(ui: &mut egui::Ui, old: &str, new: &str, theme: &crate::theme::ThemeColors) {
    use crate::theme::EguiThemeExt;

    let del_color = theme.error();
    let add_color = theme.success();
    let ctx_color = theme.text_dim();
    let bg_del = theme.tinted_bg(theme.diff_removed(), 15);
    let bg_add = theme.tinted_bg(theme.diff_added(), 15);

    // Show removed lines
    if !old.is_empty() {
        for ln in old.lines() {
            let row_rect = ui.available_rect_before_wrap();
            let row_rect =
                egui::Rect::from_min_size(row_rect.min, egui::vec2(row_rect.width(), 16.0));
            ui.painter().rect_filled(row_rect, 0.0, bg_del);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("- ")
                        .color(del_color)
                        .monospace()
                        .size(11.0),
                );
                ui.label(
                    egui::RichText::new(ln)
                        .color(del_color)
                        .monospace()
                        .size(11.0),
                );
            });
        }
    }

    // Show added lines
    if !new.is_empty() {
        for ln in new.lines() {
            let row_rect = ui.available_rect_before_wrap();
            let row_rect =
                egui::Rect::from_min_size(row_rect.min, egui::vec2(row_rect.width(), 16.0));
            ui.painter().rect_filled(row_rect, 0.0, bg_add);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("+ ")
                        .color(add_color)
                        .monospace()
                        .size(11.0),
                );
                ui.label(
                    egui::RichText::new(ln)
                        .color(add_color)
                        .monospace()
                        .size(11.0),
                );
            });
        }
    }

    if old.is_empty() && new.is_empty() {
        ui.label(
            egui::RichText::new("(empty edit)")
                .color(ctx_color)
                .monospace()
                .italics()
                .size(11.0),
        );
    }
}

/// Render a code preview (first N lines, monospace, with line numbers).
fn render_code_preview(
    ui: &mut egui::Ui,
    content: &str,
    max_lines: usize,
    theme: &crate::theme::ThemeColors,
) {
    use crate::theme::EguiThemeExt;

    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let showing = total.min(max_lines);
    let gutter_w = format!("{}", showing).len();

    egui::ScrollArea::vertical()
        .id_salt(ui.next_auto_id())
        .max_height(300.0)
        .show(ui, |ui| {
            for (i, ln) in lines.iter().take(showing).enumerate() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{:>w$} ", i + 1, w = gutter_w))
                            .color(theme.text_dim())
                            .monospace()
                            .size(10.0),
                    );
                    ui.label(
                        egui::RichText::new(*ln)
                            .color(theme.text_primary())
                            .monospace()
                            .size(11.0),
                    );
                });
            }
            if total > showing {
                ui.label(
                    egui::RichText::new(format!("  … {} more lines", total - showing))
                        .color(theme.text_dim())
                        .monospace()
                        .italics()
                        .size(10.0),
                );
            }
        });
}

/// Render a unified diff / patch with colored `+`/`-`/`@@` lines.
fn render_patch(ui: &mut egui::Ui, patch: &str, theme: &crate::theme::ThemeColors) {
    use crate::theme::EguiThemeExt;

    let del_color = theme.error();
    let add_color = theme.success();
    let hunk_color = theme.purple();
    let file_color = theme.primary();
    let ctx_color = theme.text_dim();
    let bg_del = theme.tinted_bg(theme.diff_removed(), 15);
    let bg_add = theme.tinted_bg(theme.diff_added(), 15);

    egui::ScrollArea::vertical()
        .id_salt(ui.next_auto_id())
        .max_height(400.0)
        .show(ui, |ui| {
            for ln in patch.lines() {
                let (color, bg) = if ln.starts_with("---") || ln.starts_with("+++") {
                    (file_color, egui::Color32::TRANSPARENT)
                } else if ln.starts_with("@@") {
                    (hunk_color, egui::Color32::TRANSPARENT)
                } else if ln.starts_with('+') {
                    (add_color, bg_add)
                } else if ln.starts_with('-') {
                    (del_color, bg_del)
                } else {
                    (ctx_color, egui::Color32::TRANSPARENT)
                };

                if bg != egui::Color32::TRANSPARENT {
                    let row_rect = ui.available_rect_before_wrap();
                    let row_rect =
                        egui::Rect::from_min_size(row_rect.min, egui::vec2(row_rect.width(), 16.0));
                    ui.painter().rect_filled(row_rect, 0.0, bg);
                }
                ui.label(egui::RichText::new(ln).color(color).monospace().size(11.0));
            }
        });
}

// ── Subagent progress card ────────────────────────────────────────────────────

/// State for a single subagent tracked in the timeline.
#[derive(Debug, Clone)]
pub struct SubagentCard {
    pub subagent_id: String,
    pub task: String,
    pub mode: String,
    pub model: String,
    pub status: SubagentStatus,
    pub elapsed_secs: u32,
    pub tool_calls: u32,
    pub output_lines: u32,
    pub result_preview: String,
    #[allow(dead_code)]
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SubagentStatus {
    Running,
    Complete,
    Error,
}

/// Render a subagent progress card in the timeline.
pub fn render_subagent_card(
    ui: &mut egui::Ui,
    card: &SubagentCard,
    theme: &crate::theme::ThemeColors,
) {
    use crate::theme::EguiThemeExt;

    let (status_icon, status_color, border_color) = match card.status {
        SubagentStatus::Running => ("⟳", theme.warning(), theme.warning()),
        SubagentStatus::Complete => ("✓", theme.success(), theme.success()),
        SubagentStatus::Error => ("✗", theme.error(), theme.error()),
    };

    let bg = theme.tinted_bg(border_color, 10);

    let frame = egui::Frame::new()
        .fill(bg)
        .stroke(egui::Stroke::new(1.0, border_color))
        .corner_radius(egui::CornerRadius::ZERO)
        .inner_margin(egui::Margin::symmetric(8, 6));

    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{status_icon} 🤖 Subagent [{}]", card.mode))
                    .color(status_color)
                    .monospace()
                    .strong()
                    .size(12.0),
            );
            ui.label(
                egui::RichText::new(format!("· {}", card.model))
                    .color(theme.text_dim())
                    .monospace()
                    .size(10.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{}s", card.elapsed_secs))
                        .color(theme.text_dim())
                        .monospace()
                        .size(10.0),
                );
            });
        });

        ui.label(
            egui::RichText::new(format!("  \"{}\"", card.task))
                .color(theme.text_primary())
                .monospace()
                .size(11.0),
        );

        match card.status {
            SubagentStatus::Running => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(format!(
                            "  {} tool calls · {} output lines",
                            card.tool_calls, card.output_lines,
                        ))
                        .color(theme.text_dim())
                        .monospace()
                        .size(10.0),
                    );
                });
            }
            SubagentStatus::Complete | SubagentStatus::Error => {
                if !card.result_preview.is_empty() {
                    egui::CollapsingHeader::new(
                        egui::RichText::new("  result")
                            .color(theme.text_muted())
                            .monospace()
                            .size(10.0),
                    )
                    .id_salt(format!("sa_{}", card.subagent_id))
                    .default_open(false)
                    .show(ui, |ui| {
                        for ln in card.result_preview.lines().take(10) {
                            ui.label(
                                egui::RichText::new(format!("  {ln}"))
                                    .color(theme.text_dim())
                                    .monospace()
                                    .size(10.0),
                            );
                        }
                    });
                }
            }
        }
    });
}
