use crate::app::*;
use crate::colors::ThemeColorsExt;
use unicode_width::UnicodeWidthStr;

// -- Line renderers

pub(crate) fn render_separator_item(
    width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    out.push(Line::from(Span::styled(
        "─".repeat(width),
        colors.border_muted(),
    )));
}

use super::tool_presentation::{TreeBranch, render_tool_activity_pill, resolve_tool_presentation};

pub(crate) fn render_blank_item(out: &mut Vec<Line<'static>>) {
    out.push(Line::from(""));
}

/// Render the context-window usage bar chart.
///
/// Emits:
///   Line 0: header  — "  ◆ Context  <model>  ·  <used>/<window>  (<pct>%)"
///   Line 1: bar     — proportional segments using per-category glyphs
///   Line 2+: legend — one row per non-zero category
///   Last:   blank
pub(crate) fn render_context_bar_item(
    model: &str,
    window: u64,
    pct: u8,
    category_tokens: &[u64],
    width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    // Per-category metadata: (glyph, label)
    const CAT_META: &[(char, &str)] = &[
        ('█', "System prompt"),
        ('▓', "Native tools"),
        ('▒', "MCP tools"),
        ('░', "Memory"),
        ('▪', "Skills"),
        ('■', "Messages"),
        ('·', "Free"),
        ('⎹', "Buffer (autocompact)"),
    ];

    let cat_colors: [RC; 8] = [
        colors.c_ctx_bar_system(),
        colors.c_ctx_bar_native_tools(),
        colors.c_ctx_bar_mcp_tools(),
        colors.c_ctx_bar_memory(),
        colors.c_ctx_bar_skills(),
        colors.c_ctx_bar_messages(),
        colors.c_ctx_bar_free(),
        colors.c_ctx_bar_buffer(),
    ];

    let fmt_tok = |n: u64| -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}k", n as f64 / 1_000.0)
        } else {
            n.to_string()
        }
    };

    let total_used: u64 = category_tokens
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 6 && *i != 7) // exclude free + buffer
        .map(|(_, &t)| t)
        .sum();

    // -- Header line
    out.push(Line::from(vec![
        Span::styled(
            "  ◆ Context  ",
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            model.to_string(),
            Style::default()
                .fg(colors.c_text_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", colors.text_muted()),
        Span::styled(
            format!("{}/{} tokens", fmt_tok(total_used), fmt_tok(window)),
            colors.text_muted(),
        ),
        Span::styled(
            format!("  ({}%)", pct),
            Style::default().fg(if pct >= 90 {
                colors.c_error()
            } else if pct >= 75 {
                colors.c_warning()
            } else {
                colors.c_success()
            }),
        ),
    ]));

    // -- Bar line
    // Reserve 2 chars indent + 2 chars margin = 4; fit bar in remaining width (min 20)
    let bar_width = width.saturating_sub(4).max(20).min(120);
    let mut bar_spans: Vec<Span<'static>> = vec![Span::raw("  ")];

    if window == 0 {
        bar_spans.push(Span::styled("?".repeat(bar_width), colors.text_muted()));
    } else {
        let mut filled = 0usize;
        for (i, &tok) in category_tokens.iter().enumerate() {
            if tok == 0 {
                continue;
            }
            let cells = ((tok as f64 / window as f64) * bar_width as f64).round() as usize;
            if cells == 0 {
                continue;
            }
            let (glyph, _) = CAT_META.get(i).copied().unwrap_or(('?', ""));
            let color = cat_colors.get(i).copied().unwrap_or(colors.c_text_dim());
            let s: String = std::iter::repeat_n(glyph, cells).collect();
            bar_spans.push(Span::styled(s, Style::default().fg(color)));
            filled += cells;
        }
        // Pad remainder to full bar width
        if filled < bar_width {
            let pad: String = std::iter::repeat_n('·', bar_width - filled).collect();
            bar_spans.push(Span::styled(pad, Style::default().fg(colors.c_text_dim())));
        }
    }
    out.push(Line::from(bar_spans));
    out.push(Line::from("")); // spacer

    // -- Legend lines (skip categories with 0 tokens, except Free)
    for (i, &tok) in category_tokens.iter().enumerate() {
        if i == 7 && tok == 0 {
            continue; // skip empty buffer row
        }
        let (glyph, label) = CAT_META.get(i).copied().unwrap_or(('?', "?"));
        let color = cat_colors.get(i).copied().unwrap_or(colors.c_text_dim());
        let pct_cat = if window > 0 {
            format!("{:.1}%", 100.0 * tok as f64 / window as f64)
        } else {
            "  ?%".to_string()
        };
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(glyph.to_string(), Style::default().fg(color)),
            Span::styled(format!("  {:<18}", label), colors.text_muted()),
            Span::styled(
                format!("{:>7}  {:>6}", fmt_tok(tok), pct_cat),
                colors.text_muted(),
            ),
        ]));
    }
    out.push(Line::from(""));
}

pub(crate) fn render_user_message_item(
    text: &str,
    width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
    nerd: bool,
) {
    let icon = crate::icons::user_icon(nerd);
    let rail = Span::styled("▎ ", Style::default().fg(colors.c_primary()));
    out.push(Line::from(vec![
        rail.clone(),
        Span::styled(
            format!("{icon} "),
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "You",
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    out.push(Line::from(vec![rail.clone()]));
    let parsed = crate::markdown::parse_markdown_lines_with_theme(
        text,
        colors,
        width.saturating_sub(4),
        true,
    );
    for line in parsed {
        let mut spans = vec![rail.clone()];
        spans.extend(line.spans);
        out.push(Line::from(spans));
    }
    out.push(Line::from(""));
}

pub(crate) fn render_assistant_item(
    text: &str,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
    nerd: bool,
) {
    let icon = crate::icons::assistant_icon(nerd);
    out.push(Line::from(vec![
        Span::styled(
            format!("{icon} "),
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "CADE",
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    out.push(Line::from(""));

    // Strip any historical-scratchpad (internal processing state) emitted by
    // the model without rendering it — it is never shown in the viewport.
    let body = {
        let mut body = text.to_string();
        if let Some(start) = body.find("<historical_scratchpad>") {
            let end = body
                .find("</historical_scratchpad>")
                .map(|e| e + "</historical_scratchpad>".len())
                .unwrap_or(body.len());
            body.replace_range(start..end, "");
        }
        body
    };

    let md_lines =
        crate::markdown::parse_markdown_lines_with_theme(&body, colors, width, expand_all);
    out.extend(md_lines);
}

pub(crate) fn render_streaming_assistant_item(
    text: &str,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
    nerd: bool,
) {
    render_assistant_item(text, width, expand_all, out, colors, nerd);
}

/// Live "thinking" block shown while the model is reasoning.  Only the most
/// recent lines are displayed so the viewport keeps flowing; the full text
/// Renders the live/streaming reasoning accordion.
/// Uses a compact windowed tail (max 3 lines) enclosed in a matching framed box
/// so that transitioning to the committed collapsed header is smooth and visually stable.
pub(crate) fn render_live_reasoning_item(
    text: &str,
    width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    const MAX_LIVE_LINES: usize = 3;
    let card_w = width.max(24);

    let words = text.split_whitespace().count();
    let header_badge = " THINKING ";
    let words_str = format!("({words} words)");
    let hint = "streaming…";

    let left_len = 3 + header_badge.len() + 1 + words_str.len() + 1;
    let right_len = hint.len() + 6;
    let dashes = card_w.saturating_sub(left_len + right_len).max(2);

    out.push(Line::from(""));
    out.push(Line::from(vec![
        Span::styled("╭─", colors.border_accent()),
        Span::styled(
            header_badge,
            Style::default()
                .fg(colors.c_primary())
                .bg(colors.c_bg_surface1())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(words_str, colors.text_muted()),
        Span::styled(format!(" {} ", "─".repeat(dashes)), colors.border_muted()),
        Span::styled(
            format!("[{hint}]"),
            colors.text_dim().add_modifier(Modifier::ITALIC),
        ),
        Span::styled(" ─╮", colors.border_accent()),
    ]));

    let all_lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let skip = all_lines.len().saturating_sub(MAX_LIVE_LINES);
    let inner_w = card_w.saturating_sub(6);

    for ln in all_lines.iter().skip(skip).take(MAX_LIVE_LINES) {
        out.push(Line::from(vec![
            Span::styled("│  ", colors.border_muted()),
            Span::styled(
                truncate_str(ln, inner_w),
                Style::default()
                    .fg(colors.c_text_muted())
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    let bot_dashes = card_w.saturating_sub(4).max(4);
    out.push(Line::from(vec![
        Span::styled("╰─", colors.border_accent()),
        Span::styled("─".repeat(bot_dashes), colors.border_muted()),
        Span::styled("─╯", colors.border_accent()),
    ]));
}

/// Ephemeral working/thinking status line (assessing…, tool progress, final
/// status) rendered at the bottom of the live timeline, replacing the removed
/// bottom status bar.  A signature glyph/colour is derived from the status
/// text so each state reads at a glance.
pub(crate) fn render_live_status_item(
    text: &str,
    width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
    nerd: bool,
) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let fg = if trimmed.starts_with('✗') || trimmed.starts_with('⚠') {
        colors.c_error()
    } else if trimmed.starts_with('✓') {
        colors.c_success()
    } else {
        colors.c_primary()
    };
    let budget = width.saturating_sub(12);
    let body = truncate_str(trimmed, budget);

    let (left_cap, right_cap, cap_len) = if nerd {
        ("\u{e0b6}", "\u{e0b4}", 2) //  and  rounded bubble caps
    } else {
        (" [ ", " ] ", 6)
    };

    let pill_text = format!(" {body} ");
    let pill_w = UnicodeWidthStr::width(pill_text.as_str()) + cap_len;
    let pad_total = width.saturating_sub(pill_w);
    let left_dashes = pad_total / 2;
    let right_dashes = pad_total.saturating_sub(left_dashes);

    if nerd {
        out.push(Line::from(vec![
            Span::styled("─".repeat(left_dashes), colors.border_muted()),
            Span::styled(left_cap, Style::default().fg(colors.c_bg_surface1())),
            Span::styled(
                pill_text,
                Style::default()
                    .fg(fg)
                    .bg(colors.c_bg_surface1())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(right_cap, Style::default().fg(colors.c_bg_surface1())),
            Span::styled("─".repeat(right_dashes), colors.border_muted()),
        ]));
    } else {
        out.push(Line::from(vec![
            Span::styled("─".repeat(left_dashes), colors.border_muted()),
            Span::styled(
                left_cap,
                Style::default()
                    .fg(colors.c_border_accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                pill_text,
                Style::default()
                    .fg(fg)
                    .bg(colors.c_bg_surface1())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                right_cap,
                Style::default()
                    .fg(colors.c_border_accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("─".repeat(right_dashes), colors.border_muted()),
        ]));
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_tool_call_item(
    name: &str,
    preview: &str,
    branch: TreeBranch,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
    nerd: bool,
) {
    let presentation = resolve_tool_presentation(name);
    let tree_connector = branch.connector();
    let pill_spans = render_tool_activity_pill(&presentation, colors, nerd);
    let pill_width = UnicodeWidthStr::width(
        pill_spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
            .as_str(),
    );
    let mut left_spans: Vec<Span<'static>> =
        vec![Span::styled(tree_connector, colors.border_muted())];
    left_spans.extend(pill_spans);

    // Clean 2-space margin between pill and argument preview
    let margin_str = if preview.is_empty() { "" } else { "  " };
    left_spans.push(Span::styled(margin_str, colors.text_dim()));

    let prefix_width =
        UnicodeWidthStr::width(tree_connector) + pill_width + UnicodeWidthStr::width(margin_str);
    let budget = width.saturating_sub(prefix_width + 4);
    let args_str = if preview.is_empty() {
        String::new()
    } else if expand_all || UnicodeWidthStr::width(preview) <= budget {
        preview.to_owned()
    } else {
        let truncated = truncate_str(preview, budget.saturating_sub(1));
        format!("{truncated}…")
    };
    left_spans.push(Span::styled(args_str, colors.text_muted()));

    out.push(Line::from(left_spans));
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_tool_result_item(
    is_error: bool,
    content: &str,
    branch: TreeBranch,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
    nerd: bool,
) {
    // Check if content represents a unified diff (e.g. from file edit/patch)
    if !is_error
        && (content.contains("@@ -") || content.starts_with("--- ") || content.starts_with("diff "))
        && let Some(diff_card) =
            crate::app::timeline::DiffViewEngine::parse_unified_diff("edit", "modified", content)
    {
        out.extend(crate::app::timeline::DiffViewEngine::render_diff_card(
            &diff_card,
            width as u16,
            expand_all,
            colors,
        ));
        out.push(Line::from(""));
        return;
    }

    let color = if is_error {
        colors.c_diff_removed()
    } else {
        colors.c_diff_added()
    };
    let marker = if is_error {
        format!("{} ", crate::icons::error_icon(nerd))
    } else {
        format!("{} ", crate::icons::success_icon(nerd))
    };
    let inner_w = width.saturating_sub(12);
    let lns: Vec<&str> = content.lines().collect();
    let guide_prefix = branch.guide_rail();

    if lns.is_empty() {
        out.push(Line::from(vec![
            Span::styled(guide_prefix, colors.border_muted()),
            Span::styled(
                marker,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "(no output)",
                colors.text_dim().add_modifier(Modifier::ITALIC),
            ),
        ]));
    } else if lns.len() == 1 && !is_error {
        // Single-line clean success output (e.g. "OK", "Checkpoint created")
        let single = lns[0].trim();
        let display_txt = truncate_str(single, inner_w);
        out.push(Line::from(vec![
            Span::styled(guide_prefix, colors.border_muted()),
            Span::styled(
                marker,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(display_txt, Style::default().fg(colors.c_text_primary())),
        ]));
    } else if !expand_all && !is_error {
        // Folded Tool Execution Card (Collapsed State)
        // Provides a compact 1-line summary that prevents viewport flooding while agent is working.
        let line_count_str = format!("({} lines)", lns.len());
        let first_preview = lns
            .iter()
            .find(|l| !l.trim().is_empty())
            .unwrap_or(&lns[0])
            .trim();

        let hint = "ctrl+o to expand";
        let fixed_w = 3 + marker.chars().count() + line_count_str.len() + 3 + 4 + hint.len();
        let preview_budget = width.saturating_sub(fixed_w);

        let preview_span = if preview_budget > 8 {
            let truncated = truncate_str(first_preview, preview_budget.saturating_sub(2));
            Span::styled(
                format!("\"{truncated}\""),
                Style::default()
                    .fg(colors.c_text_muted())
                    .add_modifier(Modifier::ITALIC),
            )
        } else {
            Span::raw("")
        };

        out.push(Line::from(vec![
            Span::styled(guide_prefix, colors.border_muted()),
            Span::styled(
                marker,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                line_count_str,
                Style::default()
                    .fg(colors.c_primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", colors.border_muted()),
            preview_span,
            Span::styled(" · ", colors.border_muted()),
            Span::styled(format!("[{hint}]"), colors.text_dim()),
        ]));
    } else {
        // Expanded State (or Error State)
        use ansi_to_tui::IntoText;

        if expand_all && !is_error {
            let header_badge = format!(" Output ({} lines) ", lns.len());
            let hint = "ctrl+o to collapse";
            let left_len = 4 + header_badge.len();
            let right_len = hint.len() + 6;
            let dashes = width.saturating_sub(left_len + right_len).max(2);

            out.push(Line::from(vec![
                Span::styled("╭─", colors.border_accent()),
                Span::styled(
                    header_badge,
                    Style::default()
                        .fg(colors.c_primary())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("─".repeat(dashes), colors.border_muted()),
                Span::styled(format!(" [{hint}] ─╮"), colors.border_accent()),
            ]));
        }

        let show_limit = if expand_all { 50 } else { 4 };
        let show = lns.len().min(show_limit);

        for (i, ln) in lns.iter().take(show).enumerate() {
            let mut spans = Vec::new();
            if i == 0 && (!expand_all || is_error) {
                spans.push(Span::styled(guide_prefix, colors.border_muted()));
                spans.push(Span::styled(
                    marker.clone(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(
                    branch.continuation_rail(),
                    colors.border_muted(),
                ));
            }

            let parsed_text = ln
                .into_text()
                .unwrap_or_else(|_| ratatui::text::Text::raw(ln.to_string()));
            let parsed_spans: Vec<Span> = parsed_text
                .lines
                .into_iter()
                .flat_map(|line| line.spans)
                .collect();

            if parsed_spans.iter().all(|s| s.style == Style::default()) {
                let text_content = parsed_spans
                    .into_iter()
                    .map(|s| s.content)
                    .collect::<String>();
                let style = if is_error {
                    Style::default().fg(color)
                } else {
                    Style::default().fg(colors.c_text_primary())
                };
                spans.push(Span::styled(truncate_str(&text_content, inner_w), style));
            } else {
                let mut remaining = inner_w;
                for mut s in parsed_spans {
                    let len = s.content.chars().count();
                    if len > remaining {
                        let truncated = s.content.chars().take(remaining).collect::<String>();
                        s.content = std::borrow::Cow::Owned(truncated);
                        spans.push(s);
                        break;
                    } else {
                        spans.push(s);
                        remaining -= len;
                    }
                }
            }

            out.push(Line::from(spans));
        }

        let remaining = lns.len().saturating_sub(show);
        if remaining > 0 {
            let hint = if expand_all {
                format!("+{remaining} lines")
            } else {
                format!("+{remaining} lines hidden · ctrl+o to expand")
            };
            out.push(Line::from(vec![
                Span::styled(branch.continuation_rail(), colors.border_muted()),
                Span::styled(
                    format!("[{hint}]"),
                    Style::default()
                        .fg(colors.c_primary())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        if expand_all && !is_error {
            let dashes = width.saturating_sub(4).max(4);
            out.push(Line::from(vec![
                Span::styled("╰─", colors.border_accent()),
                Span::styled("─".repeat(dashes), colors.border_muted()),
                Span::styled("─╯", colors.border_accent()),
            ]));
        }
    }

    // Trailing vertical breathing margin after the completed tool cycle
    out.push(Line::from(""));
}

/// Renders a committed reasoning block as a smooth, framed accordion card.
/// Collapsed state matches the live reasoning framing to avoid sudden viewport jumps.
pub(crate) fn render_reasoning_item(
    words: usize,
    content: &str,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    let card_w = width.max(24);
    let header_badge = " THINKING ";
    let words_str = format!("({words} words)");
    let hint = if expand_all {
        "ctrl+o to collapse"
    } else {
        "ctrl+o to expand"
    };

    let left_len = 3 + header_badge.len() + 1 + words_str.len() + 1;
    let right_len = hint.len() + 6;
    let dashes = card_w.saturating_sub(left_len + right_len).max(2);

    out.push(Line::from(""));
    out.push(Line::from(vec![
        Span::styled("╭─", colors.border_muted()),
        Span::styled(
            header_badge,
            Style::default()
                .fg(colors.c_text_primary())
                .bg(colors.c_bg_surface1())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(words_str, colors.text_muted()),
        Span::styled(format!(" {} ", "─".repeat(dashes)), colors.border_muted()),
        Span::styled(format!("[{hint}]"), colors.text_dim()),
        Span::styled(" ─╮", colors.border_muted()),
    ]));

    if expand_all {
        let inner_w = card_w.saturating_sub(6);
        for ln in content.lines() {
            out.push(Line::from(vec![
                Span::styled("│  ", colors.border_muted()),
                Span::styled(
                    truncate_str(ln, inner_w),
                    Style::default()
                        .fg(colors.c_text_muted())
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }

    let bot_dashes = card_w.saturating_sub(4).max(4);
    out.push(Line::from(vec![
        Span::styled("╰─", colors.border_muted()),
        Span::styled("─".repeat(bot_dashes), colors.border_muted()),
        Span::styled("─╯", colors.border_muted()),
    ]));
}

/// Renders live process and subprocess outputs with a windowed head/tail buffer.
/// Displays the head (command start) and tail (latest progress/result) separated
/// by a hidden line count, preventing long compilation or test logs from displacing
/// the viewport during task execution.
pub(crate) fn render_live_output_item(
    lines: &[String],
    max_visible: usize,
    done: bool,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    let inner_w = width.saturating_sub(11);
    let color = if done {
        colors.c_diff_added()
    } else {
        colors.c_primary()
    };

    let badge_text = if done { " FINISHED " } else { " RUNNING " };

    if lines.is_empty() {
        out.push(Line::from(vec![
            Span::styled("│ ", colors.border_muted()),
            Span::styled(
                badge_text,
                Style::default()
                    .fg(color)
                    .bg(colors.c_tool_pending_bg())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "(starting…)",
                colors.text_dim().add_modifier(Modifier::ITALIC),
            ),
        ]));
        return;
    }

    use ansi_to_tui::IntoText;

    let total = lines.len();
    let limit = if expand_all {
        usize::MAX
    } else {
        max_visible.max(4)
    };

    let format_line_spans = |ln: &str, is_first: bool| -> Line<'static> {
        let mut spans = Vec::new();
        if is_first {
            spans.push(Span::styled("│ ", colors.border_muted()));
            spans.push(Span::styled(
                badge_text,
                Style::default()
                    .fg(color)
                    .bg(colors.c_tool_pending_bg())
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
        } else {
            spans.push(Span::styled("│   ", colors.border_muted()));
        }

        let parsed_text = ln
            .into_text()
            .unwrap_or_else(|_| ratatui::text::Text::raw(ln.to_string()));
        let parsed_spans: Vec<Span> = parsed_text
            .lines
            .into_iter()
            .flat_map(|line| line.spans)
            .collect();

        if parsed_spans.iter().all(|s| s.style == Style::default()) {
            let text_content = parsed_spans
                .into_iter()
                .map(|s| s.content)
                .collect::<String>();
            let style = if is_first && !done {
                Style::default()
                    .fg(colors.c_text_primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.c_text_primary())
            };
            spans.push(Span::styled(truncate_str(&text_content, inner_w), style));
        } else {
            let mut remaining = inner_w;
            for mut s in parsed_spans {
                let len = s.content.chars().count();
                if len > remaining {
                    let truncated = s.content.chars().take(remaining).collect::<String>();
                    s.content = std::borrow::Cow::Owned(truncated);
                    spans.push(s);
                    break;
                } else {
                    spans.push(s);
                    remaining -= len;
                }
            }
        }

        Line::from(spans)
    };

    if total <= limit || expand_all {
        let show_count = total.min(if expand_all { 100 } else { limit });
        for (i, ln) in lines.iter().take(show_count).enumerate() {
            out.push(format_line_spans(ln, i == 0));
        }
        let remaining = total.saturating_sub(show_count);
        if remaining > 0 {
            out.push(Line::from(vec![
                Span::styled("│   ", colors.border_muted()),
                Span::styled(
                    format!("… +{remaining} more lines hidden"),
                    colors.text_dim().add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    } else {
        const HEAD_LINES: usize = 1;
        let tail_lines = limit.saturating_sub(HEAD_LINES).max(2);
        let hidden = total.saturating_sub(HEAD_LINES + tail_lines);

        for i in 0..HEAD_LINES.min(total) {
            out.push(format_line_spans(&lines[i], i == 0));
        }

        if hidden > 0 {
            let hint = format!("… {hidden} intermediate lines (ctrl+o to expand)");
            out.push(Line::from(vec![
                Span::styled("│   ", colors.border_muted()),
                Span::styled(hint, colors.text_dim().add_modifier(Modifier::ITALIC)),
            ]));
        }

        let tail_start = total.saturating_sub(tail_lines);
        for ln in &lines[tail_start..] {
            out.push(format_line_spans(ln, false));
        }
    }
}

pub(crate) fn render_system_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for (i, ln) in text.lines().enumerate() {
        out.push(Line::from(vec![
            Span::styled(
                if i == 0 { " INFO " } else { "      " },
                Style::default()
                    .fg(colors.c_primary())
                    .bg(colors.c_bg_base())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(ln.to_string(), colors.text_muted()),
        ]));
    }
}

pub(crate) fn render_success_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for (i, ln) in text.lines().enumerate() {
        out.push(Line::from(vec![
            Span::styled(
                if i == 0 { " OK " } else { "    " },
                Style::default()
                    .fg(colors.c_success())
                    .bg(colors.c_bg_base())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(ln.to_string(), colors.success()),
        ]));
    }
}

pub(crate) fn render_info_header_item(
    text: &str,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    for ln in text.lines() {
        out.push(Line::from(Span::styled(
            ln.to_string(),
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        )));
    }
}

pub(crate) fn render_dim_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for ln in text.lines() {
        out.push(Line::from(Span::styled(
            ln.to_string(),
            colors.text_dim().add_modifier(Modifier::DIM),
        )));
    }
}

pub(crate) fn render_pair_item(
    label: &str,
    value: &str,
    width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    let val_w = width.saturating_sub(26);
    out.push(Line::from(vec![
        Span::styled(format!("  {label:<24}"), colors.text_dim()),
        Span::styled(truncate_str(value, val_w), colors.text_primary()),
    ]));
}

pub(crate) fn render_error_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for (i, ln) in text.lines().enumerate() {
        out.push(Line::from(vec![
            Span::styled(
                if i == 0 { " ERR " } else { "     " },
                Style::default()
                    .fg(colors.c_error())
                    .bg(colors.c_bg_surface1())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(ln.to_string(), colors.error()),
        ]));
    }
}

pub(crate) fn render_question_result_item(
    header: &str,
    answer: &str,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    out.push(Line::from(vec![
        Span::styled(
            " DONE ",
            Style::default()
                .fg(colors.c_success())
                .bg(colors.c_bg_surface1())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{header}: "),
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(answer.to_string(), colors.text_primary()),
    ]));
}

pub(crate) fn render_heuristic_summary_item(
    intent: &str,
    safety: &str,
    directives: &str,
    width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    let w = width.max(40).saturating_sub(4);
    let top = format!(
        "╭── ⚡ Context & Memory Synchronized {}╮",
        "─".repeat(w.saturating_sub(35))
    );
    out.push(Line::from(Span::styled(top, colors.text_dim())));

    let mut render_row = |label: &str, value: &str, val_color: ratatui::style::Color| {
        let label_pad = format!("│  {label:<10} │ ");
        let val_w = w.saturating_sub(15);
        let val_str = crate::truncate_str(value, val_w);
        let pad = " ".repeat(val_w.saturating_sub(val_str.width()));
        out.push(Line::from(vec![
            Span::styled(label_pad, colors.text_dim()),
            Span::styled(val_str, Style::default().fg(val_color)),
            Span::styled(format!("{pad} │"), colors.text_dim()),
        ]));
    };

    render_row("Intent", intent, colors.c_text_primary());
    render_row("Safety", safety, colors.c_success());
    render_row("Directives", directives, colors.c_text_primary());

    let bot = format!("╰{}╯", "─".repeat(w));
    out.push(Line::from(Span::styled(bot, colors.text_dim())));
}

pub(crate) fn render_table_item(
    headers: &[String],
    rows: &[Vec<String>],
    width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    if rows.is_empty() {
        return;
    }
    let n_cols = headers.len();
    if n_cols == 0 {
        return;
    }

    // Column widths use Unicode display width.
    let mut widths = vec![0usize; n_cols];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = UnicodeWidthStr::width(h.as_str());
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < n_cols {
                widths[i] = widths[i].max(UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }

    // Cap column widths so total fits within viewport.
    // Layout: "│ col0 │ col1 │ … │"
    //   prefix:    "│ "(2)
    //   suffix:    " │"(2)
    //   inter-col: " │ "(3) × (n_cols - 1)
    // Total non-content overhead = 4 + 3*(n_cols-1) — same formula as the
    // markdown table renderer (sans INDENT, since RenderLine::Table is
    // emitted without the body-content indent).
    let row_overhead = 4 + 3 * n_cols.saturating_sub(1);
    if width > 0 {
        let budget = width.saturating_sub(row_overhead);
        let total: usize = widths.iter().sum();
        if total > budget && budget > 0 {
            let min_col = 3usize;
            let min_total = min_col * n_cols;
            let target = budget.max(min_total);
            for w in widths.iter_mut() {
                let share = (*w as f64 / total as f64 * target as f64).floor() as usize;
                *w = share.max(min_col);
            }
        }
    }

    // Truncate `s` to `max` Unicode columns; trailing `…` if cut.
    let truncate = |s: &str, max: usize| -> String {
        let w = UnicodeWidthStr::width(s);
        if w <= max {
            return s.to_string();
        }
        let target = max.saturating_sub(1);
        let mut out_s = String::new();
        let mut acc = 0usize;
        for ch in s.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if acc + cw > target {
                break;
            }
            out_s.push(ch);
            acc += cw;
        }
        out_s.push('…');
        out_s
    };

    // Pad to `width` Unicode cols (left-aligned).
    let pad_right = |s: &str, width: usize| -> String {
        let w = UnicodeWidthStr::width(s);
        let extra = width.saturating_sub(w);
        format!("{s}{}", " ".repeat(extra))
    };

    let border_style = colors.text_dim();

    // ── Top border:  ┌─────┬─────┐ ───────────────────────────────────────
    let mut top_spans = vec![Span::styled("┌─".to_string(), border_style)];
    for (i, w) in widths.iter().enumerate() {
        top_spans.push(Span::styled("─".repeat(*w), border_style));
        if i < n_cols - 1 {
            top_spans.push(Span::styled("─┬─".to_string(), border_style));
        }
    }
    top_spans.push(Span::styled("─┐".to_string(), border_style));
    out.push(Line::from(top_spans));

    // ── Header row + separator ──────────────────────────────────────────
    let header_style = Style::default()
        .fg(colors.c_primary())
        .add_modifier(Modifier::BOLD);
    let mut hdr_spans = vec![Span::styled("│ ".to_string(), border_style)];
    for (i, h) in headers.iter().enumerate() {
        let cell = pad_right(&truncate(h, widths[i]), widths[i]);
        hdr_spans.push(Span::styled(cell, header_style));
        if i < n_cols - 1 {
            hdr_spans.push(Span::styled(" │ ".to_string(), border_style));
        }
    }
    hdr_spans.push(Span::styled(" │".to_string(), border_style));
    out.push(Line::from(hdr_spans));

    let mut sep_spans = vec![Span::styled("├─".to_string(), border_style)];
    for (i, w) in widths.iter().enumerate() {
        sep_spans.push(Span::styled("─".repeat(*w), border_style));
        if i < n_cols - 1 {
            sep_spans.push(Span::styled("─┼─".to_string(), border_style));
        }
    }
    sep_spans.push(Span::styled("─┤".to_string(), border_style));
    out.push(Line::from(sep_spans));

    // ── Body rows ───────────────────────────────────────────────────────
    let body_style = colors.text_primary();
    for row in rows {
        let mut row_spans = vec![Span::styled("│ ".to_string(), border_style)];
        for i in 0..n_cols {
            let cell_text = row.get(i).map(String::as_str).unwrap_or("");
            let cell = pad_right(&truncate(cell_text, widths[i]), widths[i]);
            row_spans.push(Span::styled(cell, body_style));
            if i < n_cols - 1 {
                row_spans.push(Span::styled(" │ ".to_string(), border_style));
            }
        }
        row_spans.push(Span::styled(" │".to_string(), border_style));
        out.push(Line::from(row_spans));
    }

    // ── Bottom border:  └─────┴─────┘ ───────────────────────────────────
    let mut bot_spans = vec![Span::styled("└─".to_string(), border_style)];
    for (i, w) in widths.iter().enumerate() {
        bot_spans.push(Span::styled("─".repeat(*w), border_style));
        if i < n_cols - 1 {
            bot_spans.push(Span::styled("─┴─".to_string(), border_style));
        }
    }
    bot_spans.push(Span::styled("─┘".to_string(), border_style));
    out.push(Line::from(bot_spans));

    out.push(Line::from(""));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_tool_result_item_single_line() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        render_tool_result_item(
            false,
            "OK",
            TreeBranch::Terminal,
            80,
            false,
            &mut out,
            &colors,
            false,
        );
        assert!(!out.is_empty());
        let text = out[0].to_string();
        assert!(text.contains("OK"), "expected 'OK', got {text:?}");
        assert!(text.contains("   "), "terminal result uses indented prefix");
    }

    #[test]
    fn test_render_tool_result_item_continuation_rail() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        render_tool_result_item(
            false,
            "OK",
            TreeBranch::Intermediate,
            80,
            false,
            &mut out,
            &colors,
            false,
        );
        assert!(!out.is_empty());
        let text = out[0].to_string();
        assert!(
            text.contains("│  "),
            "intermediate result uses continuation rail"
        );
    }

    #[test]
    fn test_render_tool_call_uses_tree_connector_and_friendly_pill_label() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        // Intermediate tool call in multi-tool turn
        render_tool_call_item(
            "serena__search_for_pattern",
            "Event::Resize",
            TreeBranch::Intermediate,
            80,
            false,
            &mut out,
            &colors,
            false,
        );
        let text = out[0].to_string();
        assert!(
            text.contains("├─ "),
            "intermediate tool call should use ├─ connector, got {text:?}"
        );
        assert!(text.contains("Search codebase"), "got {text:?}");
        assert!(!text.contains("[search_for_pattern]"), "got {text:?}");
        assert!(text.contains("Event::Resize"), "got {text:?}");

        // Terminal tool call in turn
        let mut out_term = Vec::new();
        render_tool_call_item(
            "serena__search_for_pattern",
            "Event::Resize",
            TreeBranch::Terminal,
            80,
            false,
            &mut out_term,
            &colors,
            false,
        );
        let text_term = out_term[0].to_string();
        assert!(
            text_term.contains("└─ "),
            "terminal tool call should use └─ connector, got {text_term:?}"
        );
    }

    #[test]
    fn test_render_tool_call_preserves_label_at_narrow_width() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        render_tool_call_item(
            "custom_mcp__archive_project",
            "a preview long enough to require truncation in a narrow terminal",
            TreeBranch::Terminal,
            30,
            false,
            &mut out,
            &colors,
            false,
        );
        let text = out[0].to_string();
        assert!(text.contains("Archive project"), "got {text:?}");
        assert!(!text.contains("[archive_project]"), "got {text:?}");
    }

    #[test]
    fn test_render_tool_result_item_collapsed_multiline() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        let content = "Compiling cade-tui v0.2.6\nFinished dev profile\n1 warning emitted";
        render_tool_result_item(
            false,
            content,
            TreeBranch::Terminal,
            80,
            false,
            &mut out,
            &colors,
            false,
        );
        assert!(
            out.len() >= 2,
            "collapsed multiline should be 1 summary line plus trailing spacer"
        );
        let text = out[0].to_string();
        assert!(
            text.contains("(3 lines)"),
            "expected line count badge, got {text:?}"
        );
        assert!(
            text.contains("ctrl+o to expand"),
            "expected expand hint, got {text:?}"
        );
        assert_eq!(
            out.last().unwrap().to_string(),
            "",
            "trailing spacer expected"
        );
    }

    #[test]
    fn test_render_tool_result_item_expanded_multiline() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        let content = "Compiling cade-tui v0.2.6\nFinished dev profile\n1 warning emitted";
        render_tool_result_item(
            false,
            content,
            TreeBranch::Terminal,
            80,
            true,
            &mut out,
            &colors,
            false,
        );
        assert!(out.len() >= 6);
        let header = out[0].to_string();
        assert!(
            header.contains("Output (3 lines)"),
            "expected header frame, got {header:?}"
        );
        assert!(
            header.contains("ctrl+o to collapse"),
            "expected collapse hint, got {header:?}"
        );
        let footer = out[out.len() - 2].to_string();
        assert!(
            footer.contains("╰─"),
            "expected closing border, got {footer:?}"
        );
        assert_eq!(
            out.last().unwrap().to_string(),
            "",
            "trailing spacer expected"
        );
    }

    #[test]
    fn test_render_tool_result_item_error_is_not_folded() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        let content = "error[E0308]: mismatched types\nexpected Color, found Style";
        render_tool_result_item(
            true,
            content,
            TreeBranch::Terminal,
            80,
            false,
            &mut out,
            &colors,
            false,
        );
        assert!(out.len() >= 2, "errors must remain visible, not folded");
    }

    #[test]
    fn test_render_live_reasoning_item_bounded_tail() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        let content =
            "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10";
        render_live_reasoning_item(content, 80, &mut out, &colors);
        // leading newline (1) + header (1) + max 3 lines (3) + footer (1) = 6 lines
        assert_eq!(
            out.len(),
            6,
            "live reasoning must be bounded to 3-line tail to prevent viewport jump"
        );
        let header = out[1].to_string();
        assert!(
            header.contains("THINKING"),
            "header must contain THINKING, got {header:?}"
        );
        assert!(
            header.contains("streaming…"),
            "header must show streaming hint, got {header:?}"
        );
        let footer = out.last().unwrap().to_string();
        assert!(
            footer.contains("╰─"),
            "footer must close the accordion box, got {footer:?}"
        );
    }

    #[test]
    fn test_render_reasoning_item_collapsed() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        let content = "first thought\nsecond thought\nthird thought";
        render_reasoning_item(6, content, 80, false, &mut out, &colors);
        // leading newline (1) + header (1) + footer (1) = 3 lines
        assert_eq!(
            out.len(),
            3,
            "collapsed reasoning must be exactly 3 lines (stable frame)"
        );
        let header = out[1].to_string();
        assert!(
            header.contains("ctrl+o to expand"),
            "header must contain expand hint, got {header:?}"
        );
        let footer = out[2].to_string();
        assert!(
            footer.contains("╰─"),
            "must have closing border, got {footer:?}"
        );
    }

    #[test]
    fn test_render_reasoning_item_expanded() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        let content = "first thought\nsecond thought\nthird thought";
        render_reasoning_item(6, content, 80, true, &mut out, &colors);
        // leading newline (1) + header (1) + 3 content lines (3) + footer (1) = 6 lines
        assert_eq!(
            out.len(),
            6,
            "expanded reasoning should render all lines plus frame"
        );
        let header = out[1].to_string();
        assert!(
            header.contains("ctrl+o to collapse"),
            "header must contain collapse hint, got {header:?}"
        );
    }

    #[test]
    fn test_render_live_output_empty() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        render_live_output_item(&[], 5, false, 80, false, &mut out, &colors);
        assert_eq!(out.len(), 1);
        let text = out[0].to_string();
        assert!(text.contains("(starting…)"));
        assert!(text.contains("RUNNING"));
    }

    #[test]
    fn test_render_live_output_windowed_head_and_tail() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        let lines: Vec<String> = (1..=20).map(|i| format!("command log line {i}")).collect();
        render_live_output_item(&lines, 5, false, 80, false, &mut out, &colors);
        // Head (1) + separator (1) + tail (4) = 6 lines total, instead of 20!
        assert_eq!(
            out.len(),
            6,
            "output must be windowed to head + separator + tail"
        );
        let first = out[0].to_string();
        assert!(
            first.contains("command log line 1"),
            "first line must show command head"
        );
        assert!(
            first.contains("RUNNING"),
            "badge must show RUNNING while active"
        );
        let separator = out[1].to_string();
        assert!(
            separator.contains("intermediate lines"),
            "must show hidden intermediate lines"
        );
        let last = out.last().unwrap().to_string();
        assert!(
            last.contains("command log line 20"),
            "tail must show latest output"
        );
    }

    #[test]
    fn test_render_live_output_expand_all() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        let lines: Vec<String> = (1..=20).map(|i| format!("command log line {i}")).collect();
        render_live_output_item(&lines, 5, false, 80, true, &mut out, &colors);
        assert_eq!(out.len(), 20, "expand_all must display all lines");
    }

    #[test]
    fn test_render_live_output_done_status() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        let lines = vec!["finished successfully".to_string()];
        render_live_output_item(&lines, 5, true, 80, false, &mut out, &colors);
        assert_eq!(out.len(), 1);
        let text = out[0].to_string();
        assert!(
            text.contains("FINISHED"),
            "badge must show FINISHED when done"
        );
    }

    #[test]
    fn test_render_user_message_item_has_accent_rail() {
        let colors = ThemeColors::default();
        let mut out = Vec::new();
        render_user_message_item("Hello world", 80, &mut out, &colors, false);
        assert!(!out.is_empty(), "user message must render lines");
        let header = out[0].to_string();
        assert!(
            header.contains("▎"),
            "header must have vertical accent rail ▎"
        );
        assert!(header.contains("You"), "header must contain You badge");

        let body_line = out.iter().find(|l| l.to_string().contains("Hello world"));
        assert!(body_line.is_some(), "must render body text");
        assert!(
            body_line.unwrap().to_string().contains("▎"),
            "body line must have vertical accent rail ▎"
        );
    }
}
