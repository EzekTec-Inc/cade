use crate::colors::{ThemeColorsExt,};
use crate::app::*;
use unicode_width::UnicodeWidthStr;


// -- Line renderers

pub(crate) fn render_separator_item(width: usize, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    out.push(Line::from(Span::styled(
        "─".repeat(width),
        colors.border_muted(),
    )));
}

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
            Style::default().fg(colors.c_text_primary()).add_modifier(Modifier::BOLD),
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
        bar_spans.push(Span::styled(
            "?".repeat(bar_width),
            colors.text_muted(),
        ));
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
            Span::styled(
                format!("  {:<18}", label),
                colors.text_muted(),
            ),
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
) {
    out.push(Line::from(vec![Span::styled(
        "You",
        Style::default()
            .fg(colors.c_text_primary())
            .add_modifier(Modifier::BOLD),
    )]));
    out.extend(crate::markdown::parse_markdown_lines_with_theme(
        text, colors, width,
    ));
}

pub(crate) fn render_assistant_item(text: &str, width: usize, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    out.push(Line::from(vec![Span::styled(
        "▍ CADE",
        Style::default()
            .fg(colors.c_primary())
            .add_modifier(Modifier::BOLD),
    )]));
    let md_lines = crate::markdown::parse_markdown_lines_with_theme(text, colors, width);
    out.extend(md_lines);
}

pub(crate) fn render_streaming_assistant_item(text: &str, width: usize, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    render_assistant_item(text, width, out, colors);
}

pub(crate) fn render_tool_call_item(
    name: &str,
    preview: &str,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
    nerd: bool,
) {
    out.push(Line::from(""));
    let display = display_tool_name(name);
    let icon = crate::icons::tool_icon(&display, nerd);
    let name_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(colors.c_primary());
    let budget = width.saturating_sub(display.len() + icon.len() + 15);
    let args_span = if preview.is_empty() {
        Span::styled(")", colors.text_dim())
    } else if expand_all || preview.len() < budget {
        Span::styled(format!("{})", preview), colors.text_dim())
    } else {
        let truncated = truncate_str(preview, budget.saturating_sub(1));
        Span::styled(format!("{truncated}…)"), colors.text_dim())
    };
    let spans: Vec<Span<'static>> = vec![
        Span::styled(
            format!("{icon} "),
            Style::default()
                .fg(colors.c_primary())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(display.to_string(), name_style.add_modifier(Modifier::BOLD)),
        Span::styled("(", colors.text_dim()),
        args_span,
    ];
    out.push(Line::from(spans));
}

pub(crate) fn render_tool_result_item(
    is_error: bool,
    content: &str,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
    nerd: bool,
) {
    let color = if is_error {
        colors.c_diff_removed()
    } else {
        colors.c_diff_added()
    };
    let status_bg = if is_error {
        colors.c_tool_error_bg()
    } else {
        colors.c_tool_success_bg()
    };
    let status_label = if is_error {
        format!("{} ERR ", crate::icons::error_icon(nerd))
    } else {
        format!("{} OK ", crate::icons::success_icon(nerd))
    };
    let inner_w = width.saturating_sub(11);
    let lns: Vec<&str> = content.lines().collect();
    if lns.is_empty() {
        out.push(Line::from(vec![
            Span::styled("│ ", colors.border_muted()),
            Span::styled(
                status_label,
                Style::default()
                    .fg(color)
                    .bg(status_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "(no output)",
                Style::default().fg(color).add_modifier(Modifier::ITALIC),
            ),
        ]));
    } else {
        use ansi_to_tui::IntoText;
        let show_limit = if expand_all { 20 } else { 3 };
        let show = lns.len().min(show_limit);

        for (i, ln) in lns.iter().take(show).enumerate() {
            let mut spans = Vec::new();
            if i == 0 {
                spans.push(Span::styled("│ ", colors.border_muted()));
                spans.push(Span::styled(
                    status_label.clone(),
                    Style::default()
                        .fg(color)
                        .bg(status_bg)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
            } else {
                spans.push(Span::styled("│      ", colors.border_muted()));
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
                let mut style = Style::default().fg(color);
                if i == 0 {
                    style = style.add_modifier(Modifier::BOLD);
                }
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
                format!("… +{remaining} lines")
            } else {
                format!("… +{remaining} lines (ctrl+o to expand)")
            };
            out.push(Line::from(vec![
                Span::styled("       ", colors.border_muted()),
                Span::styled(
                    hint,
                    Style::default()
                        .fg(colors.c_text_dim())
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }
}

pub(crate) fn render_reasoning_item(
    words: usize,
    content: &str,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    out.push(Line::from(""));
    out.push(Line::from(vec![
        Span::styled("╭ ", colors.border_muted()),
        Span::styled(
            " THINKING ",
            Style::default()
                .fg(colors.c_text_primary())
                .bg(colors.c_bg_surface1())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(format!("{words} words"), colors.text_muted()),
        Span::styled(
            if expand_all {
                " · expanded"
            } else {
                " · ctrl+o to expand"
            },
            colors.text_dim(),
        ),
    ]));
    if expand_all {
        let inner_w = width.saturating_sub(4);
        for ln in content.lines() {
            out.push(Line::from(vec![
                Span::styled("│ ", colors.border_muted()),
                Span::styled(
                    truncate_str(ln, inner_w),
                    Style::default()
                        .fg(colors.c_text_muted())
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }
}

pub(crate) fn render_live_output_item(
    lines: &[String],
    max_visible: usize,
    _done: bool,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    let inner_w = width.saturating_sub(11);
    let color = colors.c_diff_added();

    if lines.is_empty() {
        out.push(Line::from(vec![
            Span::styled("│ ", colors.border_muted()),
            Span::styled(
                " LIVE ",
                Style::default()
                    .fg(color)
                    .bg(colors.c_tool_pending_bg())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "(starting…)",
                Style::default()
                    .fg(colors.c_text_dim())
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
        return;
    }

    use ansi_to_tui::IntoText;
    let visible = if expand_all {
        lines.len()
    } else {
        lines.len().min(max_visible)
    };
    let hidden = lines.len().saturating_sub(visible);

    if hidden > 0 {
        let hint = format!("… {hidden} earlier lines (ctrl+o to expand)");
        out.push(Line::from(vec![
            Span::styled("│ ", colors.border_muted()),
            Span::styled(
                hint,
                Style::default()
                    .fg(colors.c_text_dim())
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    let start = lines.len() - visible;
    for (i, ln) in lines[start..].iter().enumerate() {
        let mut spans = Vec::new();
        if i == 0 && hidden == 0 {
            spans.push(Span::styled("│ ", colors.border_muted()));
            spans.push(Span::styled(
                " LIVE ",
                Style::default()
                    .fg(color)
                    .bg(colors.c_tool_pending_bg())
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
        } else {
            spans.push(Span::styled("│      ", colors.border_muted()));
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
            let mut style = Style::default().fg(color);
            if i == 0 && hidden == 0 {
                style = style.add_modifier(Modifier::BOLD);
            }
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

pub(crate) fn render_info_header_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
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

pub(crate) fn render_pair_item(label: &str, value: &str, width: usize, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
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
    let top = format!("╭── ⚡ Context & Memory Synchronized {}╮", "─".repeat(w.saturating_sub(35)));
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

