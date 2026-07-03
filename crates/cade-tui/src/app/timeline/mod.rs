use crate::app::timeline::render_item::*;
use crate::colors::ThemeColorsExt;
pub mod render_item;
use super::*;

// -- Timeline adapter

/// Transitional rendering adapter for the conversation viewport.
///
/// Today the TUI still stores committed content as [`RenderLine`] values and
/// streams assistant text separately.  `TimelineItem` introduces the first
/// structural layer above that flat representation so rendering, row
/// measurement, and future per-item behavior can move away from the monolithic
/// `RenderLine -> Paragraph` path incrementally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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

#[derive(Clone)]
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
            RenderLine::HeuristicSummary {
                intent,
                safety,
                directives,
            } => Self::HeuristicSummary {
                intent,
                safety,
                directives,
            },
        }
    }

    pub(crate) fn render_into(
        &self,
        width: usize,
        expand_all: bool,
        out: &mut Vec<Line<'static>>,
        colors: &ThemeColors,
        nerd: bool,
    ) {
        match self {
            Self::Separator => render_separator_item(width, out, colors),
            Self::Blank => render_blank_item(out),
            Self::ContextBar {
                model,
                window,
                pct,
                category_tokens,
            } => render_context_bar_item(model, *window, *pct, category_tokens, width, out, colors),
            Self::User(text) => render_user_message_item(text, width, out, colors),
            Self::Assistant(text) => render_assistant_item(text, width, expand_all, out, colors),
            Self::ToolCall { name, preview } => {
                render_tool_call_item(name, preview, width, expand_all, out, colors, nerd)
            }
            Self::ToolResult { is_error, content } => {
                render_tool_result_item(*is_error, content, width, expand_all, out, colors, nerd)
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
            Self::Pair { label, value } => render_pair_item(label, value, width, out, colors),
            Self::Error(text) => render_error_item(text, out, colors),
            Self::QuestionResult { header, answer } => {
                render_question_result_item(header, answer, out, colors)
            }
            Self::Table { headers, rows } => render_table_item(headers, rows, width, out, colors),
            Self::HeuristicSummary {
                intent,
                safety,
                directives,
            } => render_heuristic_summary_item(intent, safety, directives, width, out, colors),
            Self::StreamingAssistant(text) => {
                render_streaming_assistant_item(text, width, expand_all, out, colors)
            }
        }
    }

    pub(crate) fn visual_rows(
        &self,
        content_w: u16,
        expand_all: bool,
        colors: &ThemeColors,
        nerd: bool,
    ) -> u16 {
        let mut lines = Vec::new();
        self.render_into(content_w as usize, expand_all, &mut lines, colors, nerd);
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
        nerd: bool,
    ) {
        self.item.render_into(width, expand_all, out, colors, nerd)
    }

    pub(crate) fn render_with_state(
        &self,
        width: usize,
        expand_all: bool,
        expanded_items: &std::collections::HashSet<TimelineKey>,
        out: &mut Vec<Line<'static>>,
        colors: &ThemeColors,
        nerd: bool,
    ) {
        self.item.render_into(
            width,
            self.is_expanded(expand_all, expanded_items),
            out,
            colors,
            nerd,
        );
    }

    pub(crate) fn visual_rows_with_state(
        &self,
        content_w: u16,
        expand_all: bool,
        expanded_items: &std::collections::HashSet<TimelineKey>,
        colors: &ThemeColors,
        nerd: bool,
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
            nerd,
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
    nerd: bool,
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
                nerd,
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
    copy_highlight: Option<(usize, std::time::Instant)>,
    mouse_selection: Option<usize>,
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
    for (entry_idx, item) in prepared.iter().enumerate() {
        let item_end = item_start.saturating_add(item.rows);
        if item_end <= visible_start {
            item_start = item_end;
            continue;
        }
        if item_start >= visible_end {
            break;
        }

        // Determine if this entry should get the highlight background.
        // Highlighted during copy confirmation flash OR while mouse button is held.
        let is_highlighted =
            copy_highlight.is_some_and(|(idx, _)| idx == entry_idx)
                || mouse_selection.is_some_and(|idx| idx == entry_idx);

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
                    let mut style = colors.text_primary();
                    if is_highlighted {
                        style = style.bg(colors.c_bg_surface2());
                    }
                    block = block
                        .borders(ratatui::widgets::Borders::LEFT)
                        .border_style(colors.text_dim())
                        .style(style)
                        .padding(ratatui::widgets::Padding::left(1));
                }
                CardStyle::Assistant => {
                    let mut style = colors.text_primary();
                    if is_highlighted {
                        style = style.bg(colors.c_bg_surface2());
                    }
                    block = block
                        .borders(ratatui::widgets::Borders::LEFT)
                        .border_style(colors.primary())
                        .style(style)
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

    // Render high-fidelity Scrollbar (Option 1)
    if total_visual > visible {
        use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
        let scrollbar = Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .thumb_symbol("█")
            .track_symbol(Some("░"))
            .style(colors.border_muted());

        let mut scrollbar_state = ScrollbarState::new(total_visual as usize)
            .position(total_visual.saturating_sub(effective_up) as usize);

        let scrollbar_area = Rect {
            x: area.x + area.width.saturating_sub(1),
            y: area.y + CONTENT_PAD_TOP,
            width: 1,
            height: area
                .height
                .saturating_sub(CONTENT_PAD_TOP + CONTENT_PAD_BOT),
        };
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
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
