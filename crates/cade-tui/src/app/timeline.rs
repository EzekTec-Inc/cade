use super::*;

// -- Timeline adapter

/// Transitional rendering adapter for the conversation viewport.
///
/// Today the TUI still stores committed content as [`RenderLine`] values and
/// streams assistant text separately.  `TimelineItem` introduces the first
/// structural layer above that flat representation so rendering, row
/// measurement, and future per-item behavior can move away from the monolithic
/// `RenderLine -> Paragraph` path incrementally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TimelineItemKind {
    Separator,
    Blank,
    ContextBar,
    User,
    Assistant,
    ToolCall,
    ToolResult,
    LiveOutput,
    Reasoning,
    System,
    Success,
    InfoHeader,
    Dim,
    Pair,
    Error,
    QuestionResult,
    Table,
    HeuristicSummary,
    StreamingAssistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TimelineKey {
    pub(crate) index: usize,
    pub(crate) kind: TimelineItemKind,
    pub(crate) streaming: bool,
}

pub(crate) struct TimelineEntry<'a> {
    pub(crate) key: TimelineKey,
    pub(crate) item: TimelineItem<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardStyle {
    None,
    User,
    Assistant,
}

pub(crate) struct PreparedTimelineEntry {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) rows: u16,
    pub(crate) card_style: CardStyle,
}

pub(crate) enum TimelineItem<'a> {
    Separator,
    Blank,
    ContextBar {
        model: &'a str,
        window: u64,
        pct: u8,
        category_tokens: &'a [u64],
    },
    User(&'a str),
    Assistant(&'a str),
    ToolCall {
        name: &'a str,
        preview: &'a str,
    },
    ToolResult {
        is_error: bool,
        content: &'a str,
    },
    LiveOutput {
        lines: &'a [String],
        max_visible: usize,
        done: bool,
    },
    Reasoning {
        words: usize,
        content: &'a str,
    },
    System(&'a str),
    Success(&'a str),
    InfoHeader(&'a str),
    Dim(&'a str),
    Pair {
        label: &'a str,
        value: &'a str,
    },
    Error(&'a str),
    QuestionResult {
        header: &'a str,
        answer: &'a str,
    },
    Table {
        headers: &'a [String],
        rows: &'a [Vec<String>],
    },
    HeuristicSummary {
        intent: &'a str,
        safety: &'a str,
        directives: &'a str,
    },
    StreamingAssistant(&'a str),
}

impl<'a> TimelineItem<'a> {
    pub(crate) fn kind(&self) -> TimelineItemKind {
        match self {
            Self::Separator => TimelineItemKind::Separator,
            Self::Blank => TimelineItemKind::Blank,
            Self::ContextBar { .. } => TimelineItemKind::ContextBar,
            Self::User(_) => TimelineItemKind::User,
            Self::Assistant(_) => TimelineItemKind::Assistant,
            Self::ToolCall { .. } => TimelineItemKind::ToolCall,
            Self::ToolResult { .. } => TimelineItemKind::ToolResult,
            Self::LiveOutput { .. } => TimelineItemKind::LiveOutput,
            Self::Reasoning { .. } => TimelineItemKind::Reasoning,
            Self::System(_) => TimelineItemKind::System,
            Self::Success(_) => TimelineItemKind::Success,
            Self::InfoHeader(_) => TimelineItemKind::InfoHeader,
            Self::Dim(_) => TimelineItemKind::Dim,
            Self::Pair { .. } => TimelineItemKind::Pair,
            Self::Error(_) => TimelineItemKind::Error,
            Self::QuestionResult { .. } => TimelineItemKind::QuestionResult,
            Self::Table { .. } => TimelineItemKind::Table,
            Self::HeuristicSummary { .. } => TimelineItemKind::HeuristicSummary,
            Self::StreamingAssistant(_) => TimelineItemKind::StreamingAssistant,
        }
    }

    pub(crate) fn from_render_line(line: &'a RenderLine) -> Self {
        match line {
            RenderLine::Separator => Self::Separator,
            RenderLine::Blank => Self::Blank,
            RenderLine::ContextBar {
                model,
                window,
                pct,
                category_tokens,
            } => Self::ContextBar {
                model,
                window: *window,
                pct: *pct,
                category_tokens,
            },
            RenderLine::UserMessage(text) => Self::User(text),
            RenderLine::AssistantText(text) => Self::Assistant(text),
            RenderLine::ToolCall { name, preview } => Self::ToolCall { name, preview },
            RenderLine::ToolResult { is_error, content } => Self::ToolResult {
                is_error: *is_error,
                content,
            },
            RenderLine::LiveOutput {
                lines,
                max_visible,
                done,
            } => Self::LiveOutput {
                lines,
                max_visible: *max_visible,
                done: *done,
            },
            RenderLine::Reasoning { words, content } => Self::Reasoning {
                words: *words,
                content,
            },
            RenderLine::SystemMsg(text) => Self::System(text),
            RenderLine::SuccessMsg(text) => Self::Success(text),
            RenderLine::InfoHeader(text) => Self::InfoHeader(text),
            RenderLine::DimMsg(text) => Self::Dim(text),
            RenderLine::Pair { label, value } => Self::Pair { label, value },
            RenderLine::ErrorMsg(text) => Self::Error(text),
            RenderLine::QuestionResult { header, answer } => {
                Self::QuestionResult { header, answer }
            }
            RenderLine::Table { headers, rows } => Self::Table { headers, rows },
            RenderLine::HeuristicSummary { intent, safety, directives } => Self::HeuristicSummary { intent, safety, directives },
        }
    }

    pub(crate) fn render_into(
        &self,
        width: usize,
        expand_all: bool,
        out: &mut Vec<Line<'static>>,
        colors: &ThemeColors,
    ) {
        match self {
            Self::Separator => render_separator_item(width, out),
            Self::Blank => render_blank_item(out),
            Self::ContextBar {
                model,
                window,
                pct,
                category_tokens,
            } => render_context_bar_item(model, *window, *pct, category_tokens, width, out, colors),
            Self::User(text) => render_user_message_item(text, width, out, colors),
            Self::Assistant(text) => render_assistant_item(text, out, colors),
            Self::ToolCall { name, preview } => {
                render_tool_call_item(name, preview, width, expand_all, out, colors)
            }
            Self::ToolResult { is_error, content } => {
                render_tool_result_item(*is_error, content, width, expand_all, out, colors)
            }
            Self::LiveOutput {
                lines,
                max_visible,
                done,
            } => {
                render_live_output_item(lines, *max_visible, *done, width, expand_all, out, colors)
            }
            Self::Reasoning { words, content } => {
                render_reasoning_item(*words, content, width, expand_all, out, colors)
            }
            Self::System(text) => render_system_item(text, out, colors),
            Self::Success(text) => render_success_item(text, out, colors),
            Self::InfoHeader(text) => render_info_header_item(text, out, colors),
            Self::Dim(text) => render_dim_item(text, out, colors),
            Self::Pair { label, value } => render_pair_item(label, value, out, colors),
            Self::Error(text) => render_error_item(text, out, colors),
            Self::QuestionResult { header, answer } => {
                render_question_result_item(header, answer, out, colors)
            }
            Self::Table { headers, rows } => render_table_item(headers, rows, out, colors),
            Self::HeuristicSummary { intent, safety, directives } => {
                render_heuristic_summary_item(intent, safety, directives, width, out, colors)
            }
            Self::StreamingAssistant(text) => render_streaming_assistant_item(text, out, colors),
        }
    }

    pub(crate) fn visual_rows(
        &self,
        content_w: u16,
        expand_all: bool,
        colors: &ThemeColors,
    ) -> u16 {
        let mut lines = Vec::new();
        self.render_into(content_w as usize, expand_all, &mut lines, colors);
        lines.iter().map(|l| count_wrapped_rows(l, content_w)).sum()
    }
}

impl<'a> TimelineEntry<'a> {
    pub(crate) fn from_render_line(index: usize, line: &'a RenderLine) -> Self {
        let item = TimelineItem::from_render_line(line);
        Self {
            key: TimelineKey {
                index,
                kind: item.kind(),
                streaming: false,
            },
            item,
        }
    }

    pub(crate) fn streaming(index: usize, text: &'a str) -> Self {
        let item = TimelineItem::StreamingAssistant(text);
        Self {
            key: TimelineKey {
                index,
                kind: item.kind(),
                streaming: true,
            },
            item,
        }
    }

    pub(crate) fn is_expanded(
        &self,
        expand_all: bool,
        expanded_items: &std::collections::HashSet<TimelineKey>,
    ) -> bool {
        timeline_key_expanded(expand_all, expanded_items, &self.key)
    }

    pub(crate) fn render_into(
        &self,
        width: usize,
        expand_all: bool,
        out: &mut Vec<Line<'static>>,
        colors: &ThemeColors,
    ) {
        self.item.render_into(width, expand_all, out, colors)
    }

    pub(crate) fn render_with_state(
        &self,
        width: usize,
        expand_all: bool,
        expanded_items: &std::collections::HashSet<TimelineKey>,
        out: &mut Vec<Line<'static>>,
        colors: &ThemeColors,
    ) {
        self.item.render_into(
            width,
            self.is_expanded(expand_all, expanded_items),
            out,
            colors,
        );
    }

    pub(crate) fn visual_rows_with_state(
        &self,
        content_w: u16,
        expand_all: bool,
        expanded_items: &std::collections::HashSet<TimelineKey>,
        colors: &ThemeColors,
    ) -> u16 {
        let card_style = match self.key.kind {
            TimelineItemKind::User => CardStyle::User,
            TimelineItemKind::Assistant | TimelineItemKind::StreamingAssistant => {
                CardStyle::Assistant
            }
            _ => CardStyle::None,
        };
        let effective_width = match card_style {
            CardStyle::None => content_w,
            _ => content_w.saturating_sub(2), // 1 for border, 1 for padding
        };

        self.item.visual_rows(
            effective_width,
            self.is_expanded(expand_all, expanded_items),
            colors,
        )
    }

    pub(crate) fn is_tool_call(&self) -> bool {
        self.key.kind == TimelineItemKind::ToolCall
    }
}

pub(crate) fn build_timeline_entries<'a>(lines: &'a [RenderLine]) -> Vec<TimelineEntry<'a>> {
    lines
        .iter()
        .enumerate()
        .map(|(idx, line)| TimelineEntry::from_render_line(idx, line))
        .collect()
}

pub(crate) fn prepare_timeline_entries(
    entries: &[TimelineEntry<'_>],
    width: usize,
    expand_all: bool,
    expanded_items: &std::collections::HashSet<TimelineKey>,
    colors: &ThemeColors,
) -> Vec<PreparedTimelineEntry> {
    entries
        .iter()
        .map(|entry| {
            let card_style = match entry.key.kind {
                TimelineItemKind::User => CardStyle::User,
                TimelineItemKind::Assistant | TimelineItemKind::StreamingAssistant => {
                    CardStyle::Assistant
                }
                _ => CardStyle::None,
            };
            let effective_width = match card_style {
                CardStyle::None => width,
                _ => width.saturating_sub(2), // 1 for border, 1 for padding
            };
            let mut lines = Vec::new();
            entry.render_with_state(
                effective_width,
                expand_all,
                expanded_items,
                &mut lines,
                colors,
            );
            let rows = lines
                .iter()
                .map(|l| count_wrapped_rows(l, effective_width as u16))
                .sum();
            PreparedTimelineEntry {
                lines,
                rows,
                card_style,
            }
        })
        .collect()
}

pub(crate) fn render_timeline_viewport(
    frame: &mut Frame,
    area: Rect,
    prepared: &[PreparedTimelineEntry],
    scroll: usize,
    colors: &ThemeColors,
) -> u16 {
    // Clear the full messages area so no stale content leaks between frames.
    frame.render_widget(ratatui::widgets::Clear, area);

    let total_visual: u16 = prepared
        .iter()
        .map(|p| p.rows as u32)
        .sum::<u32>()
        .min(u16::MAX as u32) as u16;
    let visible = area
        .height
        .saturating_sub(CONTENT_PAD_TOP + CONTENT_PAD_BOT);
    let max_skip = total_visual.saturating_sub(visible);
    let effective_up = (scroll as u16).min(max_skip);
    let visible_start = max_skip.saturating_sub(effective_up);
    let visible_end = visible_start.saturating_add(visible);

    let inner = Rect {
        x: area.x + 2,
        y: area.y + CONTENT_PAD_TOP,
        width: area.width.saturating_sub(4),
        height: area
            .height
            .saturating_sub(CONTENT_PAD_TOP + CONTENT_PAD_BOT),
    };

    let mut item_start: u16 = 0;
    for item in prepared {
        let item_end = item_start.saturating_add(item.rows);
        if item_end <= visible_start {
            item_start = item_end;
            continue;
        }
        if item_start >= visible_end {
            break;
        }

        let clip_top = visible_start.saturating_sub(item_start);
        let render_start = item_start.max(visible_start);
        let render_end = item_end.min(visible_end);
        let render_height = render_end.saturating_sub(render_start);
        if render_height > 0 {
            let rect = Rect {
                x: inner.x,
                y: inner.y + render_start.saturating_sub(visible_start),
                width: inner.width,
                height: render_height,
            };
            let mut block = ratatui::widgets::Block::default();
            match item.card_style {
                CardStyle::User => {
                    block = block
                        .borders(ratatui::widgets::Borders::LEFT)
                        .border_style(Style::default().fg(colors.dim))
                        .padding(ratatui::widgets::Padding::left(1));
                }
                CardStyle::Assistant => {
                    block = block
                        .borders(ratatui::widgets::Borders::LEFT)
                        .border_style(Style::default().fg(colors.assistant_accent))
                        .padding(ratatui::widgets::Padding::left(1));
                }
                CardStyle::None => {}
            }
            frame.render_widget(
                Paragraph::new(item.lines.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((clip_top, 0))
                    .block(block),
                rect,
            );
        }

        item_start = item_end;
    }

    max_skip
}

pub(crate) fn timeline_key_expanded(
    expand_all: bool,
    expanded_items: &std::collections::HashSet<TimelineKey>,
    key: &TimelineKey,
) -> bool {
    expand_all || expanded_items.contains(key)
}



// -- Line renderers

fn render_separator_item(width: usize, out: &mut Vec<Line<'static>>) {
    out.push(Line::from(Span::styled(
        "─".repeat(width),
        Style::default().fg(RC::DarkGray),
    )));
}

fn render_blank_item(out: &mut Vec<Line<'static>>) {
    out.push(Line::from(""));
}

/// Render the context-window usage bar chart.
///
/// Emits:
///   Line 0: header  — "  ◆ Context  <model>  ·  <used>/<window>  (<pct>%)"
///   Line 1: bar     — proportional segments using per-category glyphs
///   Line 2+: legend — one row per non-zero category
///   Last:   blank
fn render_context_bar_item(
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
                .fg(colors.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            model.to_string(),
            Style::default().fg(RC::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(colors.muted)),
        Span::styled(
            format!("{}/{} tokens", fmt_tok(total_used), fmt_tok(window)),
            Style::default().fg(colors.muted),
        ),
        Span::styled(
            format!("  ({}%)", pct),
            Style::default().fg(if pct >= 90 {
                RC::Rgb(239, 68, 68)
            } else if pct >= 75 {
                RC::Rgb(245, 158, 11)
            } else {
                colors.success
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
            Style::default().fg(colors.muted),
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
                Style::default().fg(colors.muted),
            ),
            Span::styled(
                format!("{:>7}  {:>6}", fmt_tok(tok), pct_cat),
                Style::default().fg(colors.muted),
            ),
        ]));
    }
    out.push(Line::from(""));
}

fn render_user_message_item(
    text: &str,
    _width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    out.push(Line::from(vec![Span::styled(
        "You",
        Style::default()
            .fg(colors.text)
            .add_modifier(Modifier::BOLD),
    )]));
    out.extend(crate::markdown::parse_markdown_lines_with_theme(
        text, colors,
    ));
}

fn render_assistant_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    out.push(Line::from(vec![Span::styled(
        "▍ CADE",
        Style::default()
            .fg(colors.assistant_accent)
            .add_modifier(Modifier::BOLD),
    )]));
    let md_lines = crate::markdown::parse_markdown_lines_with_theme(text, colors);
    out.extend(md_lines);
}

fn render_streaming_assistant_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    render_assistant_item(text, out, colors);
}

fn render_tool_call_item(
    name: &str,
    preview: &str,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    out.push(Line::from(""));
    let display = display_tool_name(name);
    let name_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(colors.tool_title);
    let budget = width.saturating_sub(display.len() + 14);
    let args_span = if preview.is_empty() {
        Span::styled(")", Style::default().fg(colors.dim))
    } else if expand_all || preview.len() < budget {
        Span::styled(format!("{})", preview), Style::default().fg(colors.dim))
    } else {
        let truncated = truncate_str(preview, budget.saturating_sub(1));
        Span::styled(format!("{truncated}…)"), Style::default().fg(colors.dim))
    };
    let spans: Vec<Span<'static>> = vec![
        Span::styled(
            "▶ TOOL ",
            Style::default()
                .fg(colors.assistant_accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{display}"), name_style.add_modifier(Modifier::BOLD)),
        Span::styled("(", Style::default().fg(colors.dim)),
        args_span,
    ];
    out.push(Line::from(spans));
}

fn render_tool_result_item(
    is_error: bool,
    content: &str,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    let color = if is_error {
        colors.diff_removed
    } else {
        colors.diff_added
    };
    let inner_w = width.saturating_sub(11);
    let lns: Vec<&str> = content.lines().collect();
    if lns.is_empty() {
        out.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(colors.border)),
            Span::styled(
                if is_error { "✗ ERR " } else { "✓ OK " },
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
                spans.push(Span::styled("│ ", Style::default().fg(colors.border)));
                spans.push(Span::styled(
                    if is_error { "✗ ERR " } else { "✓ OK " },
                    Style::default()
                        .fg(color)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
            } else {
                spans.push(Span::styled("│      ", Style::default().fg(colors.border)));
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
                Span::styled("       ", Style::default().fg(colors.border)),
                Span::styled(
                    hint,
                    Style::default()
                        .fg(colors.dim)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }
}

fn render_reasoning_item(
    words: usize,
    content: &str,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    out.push(Line::from(""));
    out.push(Line::from(vec![
        Span::styled("╭ ", Style::default().fg(colors.border_muted)),
        Span::styled(
            " THINKING ",
            Style::default()
                .fg(colors.badge_fg)
                .bg(colors.reasoning_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(format!("{words} words"), Style::default().fg(colors.muted)),
        Span::styled(
            if expand_all {
                " · expanded"
            } else {
                " · ctrl+o to expand"
            },
            Style::default().fg(colors.dim),
        ),
    ]));
    if expand_all {
        let inner_w = width.saturating_sub(4);
        for ln in content.lines() {
            out.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(colors.border_muted)),
                Span::styled(
                    truncate_str(ln, inner_w),
                    Style::default()
                        .fg(colors.thinking_text)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        }
    }
}

fn render_live_output_item(
    lines: &[String],
    max_visible: usize,
    _done: bool,
    width: usize,
    expand_all: bool,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    let inner_w = width.saturating_sub(11);
    let color = colors.diff_added;

    if lines.is_empty() {
        out.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(colors.border)),
            Span::styled(
                " LIVE ",
                Style::default()
                    .fg(color)
                    .bg(colors.tool_pending_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                "(starting…)",
                Style::default()
                    .fg(colors.dim)
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
            Span::styled("│ ", Style::default().fg(colors.border)),
            Span::styled(
                hint,
                Style::default()
                    .fg(colors.dim)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    let start = lines.len() - visible;
    for (i, ln) in lines[start..].iter().enumerate() {
        let mut spans = Vec::new();
        if i == 0 && hidden == 0 {
            spans.push(Span::styled("│ ", Style::default().fg(colors.border)));
            spans.push(Span::styled(
                " LIVE ",
                Style::default()
                    .fg(color)
                    .bg(colors.tool_pending_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
        } else {
            spans.push(Span::styled("│      ", Style::default().fg(colors.border)));
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

fn render_system_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for (i, ln) in text.lines().enumerate() {
        out.push(Line::from(vec![
            Span::styled(
                if i == 0 { " INFO " } else { "      " },
                Style::default()
                    .fg(colors.overlay_title)
                    .bg(colors.custom_message_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(ln.to_string(), Style::default().fg(colors.muted)),
        ]));
    }
}

fn render_success_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for (i, ln) in text.lines().enumerate() {
        out.push(Line::from(vec![
            Span::styled(
                if i == 0 { " OK " } else { "    " },
                Style::default()
                    .fg(colors.success)
                    .bg(colors.tool_success_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(ln.to_string(), Style::default().fg(colors.success)),
        ]));
    }
}

fn render_info_header_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for ln in text.lines() {
        out.push(Line::from(Span::styled(
            ln.to_string(),
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )));
    }
}

fn render_dim_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for ln in text.lines() {
        out.push(Line::from(Span::styled(
            ln.to_string(),
            Style::default().fg(colors.dim).add_modifier(Modifier::DIM),
        )));
    }
}

fn render_pair_item(label: &str, value: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    out.push(Line::from(vec![
        Span::styled(format!("  {label:<24}"), Style::default().fg(colors.dim)),
        Span::styled(value.to_string(), Style::default().fg(colors.text)),
    ]));
}

fn render_error_item(text: &str, out: &mut Vec<Line<'static>>, colors: &ThemeColors) {
    for (i, ln) in text.lines().enumerate() {
        out.push(Line::from(vec![
            Span::styled(
                if i == 0 { " ERR " } else { "     " },
                Style::default()
                    .fg(colors.error)
                    .bg(colors.tool_error_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(ln.to_string(), Style::default().fg(colors.error)),
        ]));
    }
}

fn render_question_result_item(
    header: &str,
    answer: &str,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    out.push(Line::from(vec![
        Span::styled(
            " DONE ",
            Style::default()
                .fg(colors.success)
                .bg(colors.tool_success_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{header}: "),
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(answer.to_string(), Style::default().fg(colors.text)),
    ]));
}

fn render_heuristic_summary_item(
    intent: &str,
    safety: &str,
    directives: &str,
    width: usize,
    out: &mut Vec<Line<'static>>,
    colors: &ThemeColors,
) {
    let w = width.max(40).saturating_sub(4);
    let top = format!("╭── ⚡ Context & Memory Synchronized {}╮", "─".repeat(w.saturating_sub(35)));
    out.push(Line::from(Span::styled(top, Style::default().fg(colors.dim))));

    let mut render_row = |label: &str, value: &str, val_color: ratatui::style::Color| {
        let label_pad = format!("│  {label:<10} │ ");
        let val_w = w.saturating_sub(15);
        let val_str = crate::truncate_str(value, val_w);
        let pad = " ".repeat(val_w.saturating_sub(val_str.width()));
        out.push(Line::from(vec![
            Span::styled(label_pad, Style::default().fg(colors.dim)),
            Span::styled(val_str, Style::default().fg(val_color)),
            Span::styled(format!("{pad} │"), Style::default().fg(colors.dim)),
        ]));
    };

    render_row("Intent", intent, colors.text);
    render_row("Safety", safety, colors.success);
    render_row("Directives", directives, colors.text);

    let bot = format!("╰{}╯", "─".repeat(w));
    out.push(Line::from(Span::styled(bot, Style::default().fg(colors.dim))));
}

fn render_table_item(
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
                .fg(colors.overlay_title)
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
                    Style::default().fg(colors.text),
                ));
            }
        }
        out.push(Line::from(row_spans));
    }
    out.push(Line::from(""));
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    mode: PermissionMode,
    input_mode: InputMode,
    agent_name: &str,
    model: &str,
    reasoning_effort: Option<&str>,
    cwd: &str,
    context_pct: Option<u8>,
    queued_count: usize,
    thinking_text: Option<&str>,
    thinking_elapsed: Option<std::time::Duration>,
    active_plan: Option<&PlanState>,
    copy_mode: bool,
    colors: &ThemeColors,
) {
    let inner = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(colors.border))
        .padding(Padding::new(1, 1, 0, 0))
        .inner(area);
    frame.render_widget(
        Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(colors.border)),
        area,
    );

    let (input_badge, _) = input_mode_badge(input_mode, colors);
    let mode_name = format!("{mode}");
    let context_text = context_pct
        .map(|p| format!("{p}%"))
        .unwrap_or_else(|| "—".to_string());
    let think_text = if let Some(elapsed) = thinking_elapsed {
        let secs = elapsed.as_secs();
        format!(
            "{} · {}s",
            thinking_text.unwrap_or("thinking…"),
            secs.max(1)
        )
    } else if queued_count > 0 {
        format!("idle · {queued_count} queued")
    } else {
        "idle".to_string()
    };
    let plan_summary = if let Some(plan) = active_plan {
        let done = plan.steps.iter().filter(|s| s.is_done).count();
        let total = plan.steps.len();
        if total > 0 {
            format!("{done}/{total} complete")
        } else {
            "none".to_string()
        }
    } else {
        "none".to_string()
    };

    let lines = vec![
        Line::from(Span::styled(
            " Session ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" agent   ", Style::default().fg(colors.muted)),
            Span::styled(
                truncate_str(agent_name, 28),
                Style::default().fg(colors.text),
            ),
        ]),
        Line::from(vec![
            Span::styled(" model   ", Style::default().fg(colors.muted)),
            Span::styled(truncate_str(model, 28), Style::default().fg(colors.text)),
        ]),
        Line::from(vec![
            Span::styled(" cwd     ", Style::default().fg(colors.muted)),
            Span::styled(truncate_str(cwd, 28), Style::default().fg(colors.text)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Status ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" mode    ", Style::default().fg(colors.muted)),
            Span::styled(mode_name, Style::default().fg(mode_sep_color(mode, colors))),
        ]),
        Line::from(vec![
            Span::styled(" input   ", Style::default().fg(colors.muted)),
            Span::styled(
                input_badge,
                Style::default().fg(colors.badge_fg).bg(colors.badge_bg),
            ),
        ]),
        Line::from(vec![
            Span::styled(" context ", Style::default().fg(colors.muted)),
            Span::styled(
                context_text,
                Style::default().fg(context_severity_color(context_pct, colors)),
            ),
        ]),
        Line::from(vec![
            Span::styled(" queue   ", Style::default().fg(colors.muted)),
            Span::styled(queued_count.to_string(), Style::default().fg(colors.text)),
        ]),
        Line::from(vec![
            Span::styled(" copy    ", Style::default().fg(colors.muted)),
            Span::styled(
                if copy_mode { "ON" } else { "OFF" },
                Style::default().fg(if copy_mode {
                    colors.success
                } else {
                    colors.dim
                }),
            ),
        ]),
        if let Some(reason) = reasoning_effort {
            Line::from(vec![
                Span::styled(" reason  ", Style::default().fg(colors.muted)),
                Span::styled(reason.to_string(), Style::default().fg(colors.warning)),
            ])
        } else {
            Line::from(vec![
                Span::styled(" reason  ", Style::default().fg(colors.muted)),
                Span::styled("default", Style::default().fg(colors.warning)),
            ])
        },
        Line::from(""),
        Line::from(Span::styled(
            " Activity ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            truncate_str(&think_text, 36),
            Style::default().fg(colors.thinking_text),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Plan ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(" todos   ", Style::default().fg(colors.muted)),
            Span::styled(plan_summary, Style::default().fg(colors.text)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            " Keys ",
            Style::default()
                .fg(colors.overlay_title)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " Ctrl+C abort / clear",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " Ctrl+O expand/collapse all",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " Tab cycle permissions",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " ↑/↓ command history",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " @ file picker",
            Style::default().fg(colors.muted),
        )),
        Line::from(Span::styled(
            " / commands menu",
            Style::default().fg(colors.muted),
        )),
    ];

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub(crate) fn render_toast(
    frame: &mut Frame,
    main_area: Rect,
    toast: &Toast,
    colors: &ThemeColors,
) {
    let width = (toast.message.chars().count() as u16 + 6)
        .clamp(20, main_area.width.saturating_sub(2).max(20));
    let rect = Rect {
        x: main_area.x + main_area.width.saturating_sub(width),
        y: main_area.y,
        width,
        height: 3,
    };
    let (fg, border) = match toast.level {
        ToastLevel::Info => (colors.text, colors.accent),
        ToastLevel::Success => (colors.text, colors.success),
        ToastLevel::Warning => (colors.text, colors.warning),
        ToastLevel::Error => (colors.text, colors.error),
    };
    frame.render_widget(ratatui::widgets::Clear, rect);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                truncate_str(&toast.message, rect.width.saturating_sub(4) as usize),
                Style::default().fg(fg),
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .style(Style::default().bg(colors.overlay_bg)),
        ),
        rect,
    );
}

fn context_severity_color(context_pct: Option<u8>, colors: &ThemeColors) -> RC {
    match context_pct {
        Some(p) if p >= 90 => colors.error,
        Some(p) if p >= 80 => colors.warning,
        Some(_) => colors.muted,
        None => colors.dim,
    }
}

// -- Input helpers (ported from input.rs)

pub(crate) fn input_mode_badge(mode: InputMode, colors: &ThemeColors) -> (&'static str, RC) {
    match mode {
        InputMode::Regular => (" CHAT ", colors.badge_bg),
        InputMode::BashCommand { silent: false } => (" SHELL ", colors.warning),
        InputMode::BashCommand { silent: true } => (" LOCAL ", colors.border_muted),
        InputMode::SlashCommand => (" COMMAND ", colors.assistant_accent),
    }
}

pub(crate) fn calc_input_rows(buf: &str, available_width: u16, prefix_width: u16) -> u16 {
    let w = available_width.max(1) as usize;
    let first_row_capacity = w.saturating_sub(prefix_width as usize).max(1);
    if buf.is_empty() {
        return 1;
    }
    let mut total: u16 = 0;
    for seg in buf.split('\n') {
        let chars = seg.chars().count();
        let rows = if chars == 0 {
            1
        } else if chars <= first_row_capacity {
            1
        } else {
            1 + (chars - first_row_capacity).div_ceil(w) as u16
        };
        total += rows;
    }
    total.clamp(1, MAX_INPUT_ROWS)
}

pub(crate) fn calc_visual_cursor(
    before_cursor: &str,
    available_width: u16,
    prefix_width: u16,
) -> (u16, u16) {
    // Mirror exactly how render_frame builds the Paragraph:
    //   • Each logical line (split on '\n') is its own ratatui Line.
    //   • The first visual row starts after the input-mode badge + "> " prefix.
    //   • The paragraph uses Wrap { trim: false }, meaning it wraps exactly
    //     at the available_width boundary. Wrapped lines do NOT get the prefix
    //     so they start at column 0.
    let w = available_width.max(1) as usize;

    let mut vis_row: u16 = 0;
    let mut vis_col: u16 = prefix_width;

    for (li, seg) in before_cursor.split('\n').enumerate() {
        if li > 0 {
            // Crossed a \n: start a new logical line → new visual row, prefix col
            vis_row += 1;
            vis_col = prefix_width;
        }
        // Walk through the segment, wrapping when we exceed available width
        let mut chars_on_row = vis_col as usize;
        for _ch in seg.chars() {
            chars_on_row += 1;
            if chars_on_row > w {
                // Wrap to next visual row within this logical line
                vis_row += 1;
                chars_on_row = 1;
                vis_col = 1; // 0-indexed column is 0, so 1st char is length 1
            } else {
                vis_col = chars_on_row as u16;
            }
        }
        // After processing all chars of this segment, vis_col is already set
        // correctly for the end of the segment.  If the segment was empty
        // (bare \n), vis_col stays at prefix_width (just the prefix).
    }

    (vis_row, vis_col)
}

/// Given the full input `buf`, the visual text-column width `text_w`
/// (= available_width - 2, matching `calc_visual_cursor`), and a target
/// `(row, col)` in visual space, return the **byte offset** in `buf` of the
/// character at that visual position.
/// Used by the Up/Down cursor-movement logic.
pub(crate) fn find_cursor_at_visual_row_col(
    buf: &str,
    available_width: u16,
    prefix_width: u16,
    target_row: u16,
    target_col: u16,
) -> usize {
    let text_w = available_width.max(1) as usize;
    let mut vis_row: u16 = 0;
    let mut chars_on_row: usize = prefix_width as usize;
    let mut byte_offset: usize = 0;

    for (li, seg) in buf.split('\n').enumerate() {
        if li > 0 {
            vis_row += 1;
            chars_on_row = prefix_width as usize;
            byte_offset += 1; // the '\n' byte
        }
        if vis_row > target_row {
            break;
        }
        let seg_start = byte_offset;
        for ch in seg.chars() {
            chars_on_row += 1;
            if chars_on_row > text_w {
                // visual wrap
                vis_row += 1;
                chars_on_row = 1;
            }
            if vis_row == target_row {
                // We're on the target row — check column
                // target_col is raw screen column; chars_on_row matches raw length
                let content_col = target_col as usize;
                if chars_on_row > content_col {
                    return byte_offset;
                }
            }
            if vis_row > target_row {
                // Overshot — return last valid position on target row
                return byte_offset;
            }
            byte_offset += ch.len_utf8();
        }
        // If we passed through the whole segment without overshooting, the
        // cursor target is at the end of the segment (or beyond — clamp to end).
        if vis_row == target_row {
            // Return end of this segment (before the next \n or end of string)
            return byte_offset;
        }
        let _ = seg_start; // suppress unused warning
    }
    // Clamp to end of buffer
    buf.len()
}
