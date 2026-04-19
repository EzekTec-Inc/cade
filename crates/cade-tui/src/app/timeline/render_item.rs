use crate::colors::{ThemeColorsExt, ColorDefExt};
use crate::app::*;


// -- Line renderers

pub(crate) fn render_separator_item(width: usize, out: &mut Vec<Line<'static>>) {
    out.push(Line::from(Span::styled(
        "─".repeat(width),
        Style::default().fg(RC::DarkGray),
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
    // Per-category metadata: (glyph, color, label)
    const CATS: &[(char, RC, &str)] = &[
        ('█', RC::Rgb(120, 120, 120), "System prompt"),
        ('▓', RC::Rgb(8, 145, 178), "Native tools"),
        ('▒', RC::Rgb(0, 188, 212), "MCP tools"),
        ('░', RC::Rgb(215, 119, 87), "Memory"),
        ('▪', RC::Rgb(255, 193, 7), "Skills"),
        ('■', RC::Rgb(147, 51, 234), "Messages"),
        ('·', RC::Rgb(50, 50, 50), "Free"),
        ('⎹', RC::Rgb(80, 80, 80), "Buffer (autocompact)"),
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
                .fg(colors.primary.to_ratatui())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            model.to_string(),
            Style::default().fg(RC::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", colors.text_muted()),
        Span::styled(
            format!("{}/{} tokens", fmt_tok(total_used), fmt_tok(window)),
            colors.text_muted(),
        ),
        Span::styled(
            format!("  ({}%)", pct),
            Style::default().fg(if pct >= 90 {
                RC::Rgb(239, 68, 68)
            } else if pct >= 75 {
                RC::Rgb(245, 158, 11)
            } else {
                colors.success.to_ratatui()
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
            let (glyph, color, _) = CATS.get(i).copied().unwrap_or(('?', RC::DarkGray, ""));
            let s: String = std::iter::repeat_n(glyph, cells).collect();
            bar_spans.push(Span::styled(s, Style::default().fg(color)));
            filled += cells;
        }
        // Pad remainder to full bar width
        if filled < bar_width {
            let pad: String = std::iter::repeat_n('·', bar_width - filled).collect();
            bar_spans.push(Span::styled(pad, Style::default().fg(RC::Rgb(40, 40, 40))));
        }
    }
    out.push(Line::from(bar_spans));
    out.push(Line::from("")); // spacer

    // -- Legend lines (skip categories with 0 tokens, except Free)
    for (i, &tok) in category_tokens.iter().enumerate() {
        if i == 7 && tok == 0 {
            continue; // skip empty buffer row
        }
        let (glyph, color, label) = CATS.get(i).copied().unwrap_or(('?', RC::DarkGray, "?"));
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
    _width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    out.push(Line::from(vec![Span::styled(
        "You",
        Style::default()
            .fg(colors.text_primary.to_ratatui())
            .add_modifier(Modifier::BOLD),
    )]));
    out.extend(crate::markdown::parse_markdown_lines_with_theme(
        text, colors,
    ));
}

pub(crate) fn render_assistant_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    out.push(Line::from(vec![Span::styled(
        "▍ CADE",
        Style::default()
            .fg(colors.primary.to_ratatui())
            .add_modifier(Modifier::BOLD),
    )]));
    let md_lines = crate::markdown::parse_markdown_lines_with_theme(text, colors);
    out.extend(md_lines);
}

pub(crate) fn render_streaming_assistant_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    render_assistant_item(text, out, colors);
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
        .fg(colors.primary.to_ratatui());
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
                .fg(colors.primary.to_ratatui())
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
        colors.diff_removed.to_ratatui()
    } else {
        colors.diff_added.to_ratatui()
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
            Span::styled("│ ", colors.border_base()),
            Span::styled(
                status_label,
                Style::default()
                    .fg(color)
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
                spans.push(Span::styled("│ ", colors.border_base()));
                spans.push(Span::styled(
                    status_label.clone(),
                    Style::default()
                        .fg(color)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
            } else {
                spans.push(Span::styled("│      ", colors.border_base()));
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
                Span::styled("       ", colors.border_base()),
                Span::styled(
                    hint,
                    Style::default()
                        .fg(colors.text_dim.to_ratatui())
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
        Span::styled("╭ ", colors.border_base()),
        Span::styled(
            " THINKING ",
            Style::default()
                .fg(colors.text_primary.to_ratatui())
                .bg(colors.bg_surface1.to_ratatui())
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
                Span::styled("│ ", colors.border_base()),
                Span::styled(
                    truncate_str(ln, inner_w),
                    Style::default()
                        .fg(colors.text_muted.to_ratatui())
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
    let color = colors.diff_added.to_ratatui();

    if lines.is_empty() {
        out.push(Line::from(vec![
            Span::styled("│ ", colors.border_base()),
            Span::styled(
                " LIVE ",
                Style::default()
                    .fg(color)
                    .bg(colors.bg_surface1.to_ratatui())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "(starting…)",
                Style::default()
                    .fg(colors.text_dim.to_ratatui())
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
            Span::styled("│ ", colors.border_base()),
            Span::styled(
                hint,
                Style::default()
                    .fg(colors.text_dim.to_ratatui())
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    let start = lines.len() - visible;
    for (i, ln) in lines[start..].iter().enumerate() {
        let mut spans = Vec::new();
        if i == 0 && hidden == 0 {
            spans.push(Span::styled("│ ", colors.border_base()));
            spans.push(Span::styled(
                " LIVE ",
                Style::default()
                    .fg(color)
                    .bg(colors.bg_surface1.to_ratatui())
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
        } else {
            spans.push(Span::styled("│      ", colors.border_base()));
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
                    .fg(colors.primary.to_ratatui())
                    .bg(colors.bg_base.to_ratatui())
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
                    .fg(colors.success.to_ratatui())
                    .bg(colors.bg_surface1.to_ratatui())
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
                .fg(colors.primary.to_ratatui())
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

pub(crate) fn render_pair_item(label: &str, value: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    out.push(Line::from(vec![
        Span::styled(format!("  {label:<24}"), colors.text_dim()),
        Span::styled(value.to_string(), colors.text_primary()),
    ]));
}

pub(crate) fn render_error_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for (i, ln) in text.lines().enumerate() {
        out.push(Line::from(vec![
            Span::styled(
                if i == 0 { " ERR " } else { "     " },
                Style::default()
                    .fg(colors.error.to_ratatui())
                    .bg(colors.bg_surface1.to_ratatui())
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
                .fg(colors.success.to_ratatui())
                .bg(colors.bg_surface1.to_ratatui())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{header}: "),
            Style::default()
                .fg(colors.primary.to_ratatui())
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

    render_row("Intent", intent, colors.text_primary.to_ratatui());
    render_row("Safety", safety, colors.success.to_ratatui());
    render_row("Directives", directives, colors.text_primary.to_ratatui());

    let bot = format!("╰{}╯", "─".repeat(w));
    out.push(Line::from(Span::styled(bot, colors.text_dim())));
}

pub(crate) fn render_table_item(
    headers: &[String],
    rows: &[Vec<String>],
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    if rows.is_empty() {
        return;
    }
    let n_cols = headers.len();
    let mut widths = vec![0; n_cols];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = h.len();
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < n_cols {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let mut header_spans = Vec::new();
    for (i, h) in headers.iter().enumerate() {
        header_spans.push(Span::styled(
            format!("  {:<width$}  ", h, width = widths[i]),
            Style::default()
                .fg(colors.primary.to_ratatui())
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ));
    }
    out.push(Line::from(header_spans));

    for row in rows {
        let mut row_spans = Vec::new();
        for (i, cell) in row.iter().enumerate() {
            if i < n_cols {
                row_spans.push(Span::styled(
                    format!("  {:<width$}  ", cell, width = widths[i]),
                    colors.text_primary(),
                ));
            }
        }
        out.push(Line::from(row_spans));
    }
    out.push(Line::from(""));
}

