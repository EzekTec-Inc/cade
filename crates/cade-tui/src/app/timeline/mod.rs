use crate::app::timeline::render_item::*;
use crate::colors::ThemeColorsExt;
pub mod diff_view;
pub mod render_item;
pub(crate) mod tool_presentation;

use super::*;
pub use diff_view::{DiffLayout, DiffViewEngine};

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
    Status,
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
    ToolCall,
    System,
    ActiveTurn,
}

#[derive(Clone)]
pub(crate) struct PreparedTimelineEntry {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) rows: u16,
    pub(crate) card_style: CardStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    #[allow(dead_code)]
    StreamingAssistant(&'a str),
    /// Live thinking block shown while the model is reasoning (not yet
    /// committed as a `Reasoning` item).
    LiveReasoning(&'a str),
    /// Ephemeral working/thinking status line (assessing, tool progress,
    /// final status) rendered at the bottom of the live timeline.
    LiveStatus(&'a str),
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
            Self::LiveReasoning(_) => TimelineItemKind::Reasoning,
            Self::LiveStatus(_) => TimelineItemKind::Status,
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
            Self::User(text) => render_user_message_item(text, width, out, colors, nerd),
            Self::Assistant(text) => {
                render_assistant_item(text, width, expand_all, out, colors, nerd)
            }
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
                render_streaming_assistant_item(text, width, expand_all, out, colors, nerd)
            }
            Self::LiveReasoning(text) => render_live_reasoning_item(text, width, out, colors),
            Self::LiveStatus(text) => render_live_status_item(text, width, out, colors, nerd),
        }
    }

    #[allow(dead_code)]
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

    #[allow(dead_code)]
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

    pub(crate) fn reasoning(index: usize, text: &'a str) -> Self {
        let item = TimelineItem::LiveReasoning(text);
        Self {
            key: TimelineKey {
                index,
                kind: item.kind(),
                streaming: true,
            },
            item,
        }
    }

    pub(crate) fn status(index: usize, text: &'a str) -> Self {
        let item = TimelineItem::LiveStatus(text);
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

    #[allow(dead_code)]
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
            TimelineItemKind::ToolCall | TimelineItemKind::ToolResult => CardStyle::ToolCall,
            TimelineItemKind::Error => CardStyle::System,
            _ => CardStyle::None,
        };
        let effective_width = match card_style {
            CardStyle::None => content_w,
            _ => content_w.saturating_sub(2), // 1 for gutter rail, 1 for padding
        };

        self.item.visual_rows(
            effective_width,
            self.is_expanded(expand_all, expanded_items),
            colors,
            nerd,
        )
    }

    #[allow(dead_code)]
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

pub(crate) fn wrap_line(
    line: ratatui::text::Line<'static>,
    width: u16,
) -> Vec<ratatui::text::Line<'static>> {
    use ratatui::text::{Line, Span};
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    if width == 0 {
        return vec![line];
    }
    let max_w = width as usize;
    let mut wrapped = Vec::new();
    let mut current_line = Line::default();
    let mut current_w = 0;

    for span in line.spans {
        let style = span.style;
        let text = span.content;

        let segments: Vec<&str> = text.split('\n').collect();
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                wrapped.push(std::mem::take(&mut current_line));
                current_w = 0;
            }

            if segment.is_empty() {
                continue;
            }

            for word in segment.split_inclusive([' ', '\t']) {
                let word_w = UnicodeWidthStr::width(word);

                if current_w > 0 && current_w + word_w > max_w {
                    wrapped.push(std::mem::take(&mut current_line));
                    current_w = 0;
                }

                if word_w > max_w {
                    let mut temp_word = word;
                    while !temp_word.is_empty() {
                        let mut take_chars = 0;
                        let mut take_w = 0;
                        for c in temp_word.chars() {
                            let cw = UnicodeWidthChar::width(c).unwrap_or(0);
                            if take_w + cw > max_w {
                                if take_w == 0 {
                                    take_w += cw;
                                    take_chars += c.len_utf8();
                                }
                                break;
                            }
                            take_w += cw;
                            take_chars += c.len_utf8();
                        }

                        if current_w > 0 && current_w + take_w > max_w {
                            wrapped.push(std::mem::take(&mut current_line));
                            current_w = 0;
                        }

                        let chunk = &temp_word[..take_chars];
                        current_line
                            .spans
                            .push(Span::styled(chunk.to_string(), style));
                        current_w += take_w;

                        temp_word = &temp_word[take_chars..];
                    }
                } else {
                    current_line
                        .spans
                        .push(Span::styled(word.to_string(), style));
                    current_w += word_w;
                }
            }
        }
    }

    if !current_line.spans.is_empty() {
        wrapped.push(current_line);
    }

    if wrapped.is_empty() {
        wrapped.push(Line::default());
    }

    wrapped
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_timeline_entries(
    entries: &[TimelineEntry<'_>],
    width: usize,
    expand_all: bool,
    expanded_items: &std::collections::HashSet<TimelineKey>,
    colors: &ThemeColors,
    nerd: bool,
    item_cache: &mut PreparedCache,
    is_processing: bool,
) -> Vec<PreparedTimelineEntry> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let last_user_idx = entries
        .iter()
        .rposition(|e| e.key.kind == TimelineItemKind::User);

    entries
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let is_expanded = expand_all || expanded_items.contains(&entry.key);
            let is_active_turn = is_processing
                && last_user_idx.is_some_and(|u_idx| i > u_idx)
                && matches!(
                    entry.key.kind,
                    TimelineItemKind::ToolCall
                        | TimelineItemKind::ToolResult
                        | TimelineItemKind::LiveOutput
                        | TimelineItemKind::Reasoning
                );

            // Compute a precise content hash of the inner item to enable content-aware caching
            let content_hash = {
                let mut h = DefaultHasher::new();
                entry.item.hash(&mut h);
                h.finish()
            };

            let cache_key = PreparedCacheKey {
                index: entry.key.index,
                kind: entry.key.kind,
                is_expanded,
                content_hash,
                is_active_turn,
            };

            if let Some(cached) = item_cache.get(&cache_key) {
                return cached.clone();
            }

            let card_style = if is_active_turn {
                CardStyle::ActiveTurn
            } else {
                match entry.key.kind {
                    TimelineItemKind::User => CardStyle::User,
                    TimelineItemKind::Assistant | TimelineItemKind::StreamingAssistant => {
                        CardStyle::Assistant
                    }
                    TimelineItemKind::ToolCall | TimelineItemKind::ToolResult => {
                        CardStyle::ToolCall
                    }
                    TimelineItemKind::Error => CardStyle::System,
                    _ => CardStyle::None,
                }
            };
            let effective_width = match card_style {
                CardStyle::None => width,
                _ => width.saturating_sub(2), // 1 for gutter rail, 1 for padding
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

            // Pre-wrap lines so that ratatui Paragraph does not have to dynamically wrap,
            // which breaks scroll alignment for multi-line wrapped lines.
            let mut pre_wrapped_lines = Vec::new();
            for l in lines {
                pre_wrapped_lines.extend(wrap_line(l, effective_width as u16));
            }

            let rows = pre_wrapped_lines.len() as u16;
            let prepared = PreparedTimelineEntry {
                lines: pre_wrapped_lines,
                rows,
                card_style,
            };

            item_cache.insert(cache_key, prepared.clone());
            prepared
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
        let is_highlighted = copy_highlight.is_some_and(|(idx, _)| idx == entry_idx)
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
            const GUTTER_BORDER: ratatui::symbols::border::Set = ratatui::symbols::border::Set {
                vertical_left: "▎",
                vertical_right: " ",
                horizontal_top: " ",
                horizontal_bottom: " ",
                top_left: "▎",
                top_right: " ",
                bottom_left: "▎",
                bottom_right: " ",
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
                        .border_set(GUTTER_BORDER)
                        .border_style(colors.border_accent())
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
                        .border_set(GUTTER_BORDER)
                        .border_style(colors.primary())
                        .style(style)
                        .padding(ratatui::widgets::Padding::left(1));
                }
                CardStyle::ToolCall => {
                    let mut style = colors.text_primary();
                    if is_highlighted {
                        style = style.bg(colors.c_bg_surface2());
                    }
                    block = block
                        .borders(ratatui::widgets::Borders::LEFT)
                        .border_set(GUTTER_BORDER)
                        .border_style(colors.border_muted())
                        .style(style)
                        .padding(ratatui::widgets::Padding::left(1));
                }
                CardStyle::ActiveTurn => {
                    let mut style = colors.text_primary();
                    if is_highlighted {
                        style = style.bg(colors.c_bg_surface2());
                    }
                    block = block
                        .borders(ratatui::widgets::Borders::LEFT)
                        .border_set(GUTTER_BORDER)
                        .border_style(colors.primary().add_modifier(Modifier::BOLD))
                        .style(style)
                        .padding(ratatui::widgets::Padding::left(1));
                }
                CardStyle::System => {
                    let mut style = colors.text_primary();
                    if is_highlighted {
                        style = style.bg(colors.c_bg_surface2());
                    }
                    block = block
                        .borders(ratatui::widgets::Borders::LEFT)
                        .border_set(GUTTER_BORDER)
                        .border_style(colors.error())
                        .style(style)
                        .padding(ratatui::widgets::Padding::left(1));
                }
                CardStyle::None => {}
            }
            frame.render_widget(
                Paragraph::new(item.lines.clone())
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
            .position(visible_start as usize)
            .viewport_content_length(visible as usize);

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

/// A localized cache key for a single timeline entry that ensures robust cache invalidation
/// across width changes, toggles (folded/expanded state), and content modifications.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreparedCacheKey {
    pub(crate) index: usize,
    pub(crate) kind: TimelineItemKind,
    pub(crate) is_expanded: bool,
    pub(crate) content_hash: u64,
    pub(crate) is_active_turn: bool,
}

/// A localized rendering cache in `cade-tui` that stores pre-wrapped visual layout spans
/// and calculated line heights for individual timeline entries. This eliminates redundant,
/// heavy CPU text-wrapping computations during continuous draw cycles, scrolling, or streaming updates.
#[derive(Clone, Default)]
pub(crate) struct PreparedCache {
    pub(crate) cache: std::collections::HashMap<PreparedCacheKey, PreparedTimelineEntry>,
}

impl PreparedCache {
    pub fn new() -> Self {
        Self {
            cache: std::collections::HashMap::new(),
        }
    }

    pub fn get(&self, key: &PreparedCacheKey) -> Option<&PreparedTimelineEntry> {
        self.cache.get(key)
    }

    pub fn insert(&mut self, key: PreparedCacheKey, entry: PreparedTimelineEntry) {
        self.cache.insert(key, entry);
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }
}

/// Scans text for the highest byte offset that represents a safe, completed Markdown block boundary.
/// Returns 0 if no safe boundary is found.
pub(crate) fn find_last_block_boundary(text: &str) -> usize {
    let mut in_code_block = false;
    let mut last_boundary = 0;
    let mut current_offset = 0;
    let mut prev_line_empty = false;

    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_code_block = !in_code_block;
            if !in_code_block {
                // Just closed a code block! The end of this line is a safe boundary.
                last_boundary = current_offset + line.len();
                prev_line_empty = false;
            }
        } else if !in_code_block {
            let is_empty = trimmed.is_empty();
            if is_empty && !prev_line_empty {
                // End of an empty line separating blocks is a safe boundary
                last_boundary = current_offset + line.len();
            }
            prev_line_empty = is_empty;
        }

        current_offset += line.len();
    }

    last_boundary
}

/// Cache for incremental Markdown parsing during live assistant streaming.
/// Freezes parsed and pre-wrapped blocks at paragraph / code block boundaries,
/// avoiding O(N²) re-parsing on every streaming token.
#[derive(Clone, Default, Debug)]
pub(crate) struct StreamingMarkdownCache {
    /// The portion of clean body text that has already been parsed and frozen into lines.
    pub committed_raw_text: String,
    /// Pre-rendered and pre-wrapped ratatui lines for all committed blocks.
    pub committed_lines: Vec<Line<'static>>,
    /// Width used for wrapping and layout.
    pub cached_width: usize,
    /// Nerd font preference used for header icon.
    pub cached_nerd: bool,
}

impl StreamingMarkdownCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.committed_raw_text.clear();
        self.committed_lines.clear();
        self.cached_width = 0;
    }

    pub fn render(
        &mut self,
        text: &str,
        width: usize,
        colors: &ThemeColors,
        nerd: bool,
    ) -> Vec<Line<'static>> {
        // Invalidate if width or nerd font settings change
        if self.cached_width != width || self.cached_nerd != nerd {
            self.clear();
            self.cached_width = width;
            self.cached_nerd = nerd;
        }

        // Clean historical-scratchpad if present
        let clean_body = if text.contains("<historical_scratchpad>") {
            let mut b = text.to_string();
            if let Some(start) = b.find("<historical_scratchpad>") {
                let end = b
                    .find("</historical_scratchpad>")
                    .map(|e| e + "</historical_scratchpad>".len())
                    .unwrap_or(b.len());
                b.replace_range(start..end, "");
            }
            std::borrow::Cow::Owned(b)
        } else {
            std::borrow::Cow::Borrowed(text)
        };

        // Invalidate if incoming text does not start with our committed prefix
        if !clean_body.starts_with(&self.committed_raw_text) {
            self.clear();
            self.cached_width = width;
            self.cached_nerd = nerd;
        }

        // Initialize header if empty
        if self.committed_lines.is_empty() {
            let icon = crate::icons::assistant_icon(nerd);
            self.committed_lines.push(Line::from(vec![
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
            self.committed_lines.push(Line::from(""));
        }

        // Find the latest completed block boundary
        let boundary = find_last_block_boundary(&clean_body);

        // If a new block completed beyond our current committed offset, parse and freeze it
        if boundary > self.committed_raw_text.len() {
            let new_slice = &clean_body[self.committed_raw_text.len()..boundary];
            let new_lines =
                crate::markdown::parse_markdown_lines_with_theme(new_slice, colors, width, true);
            for l in new_lines {
                self.committed_lines.extend(wrap_line(l, width as u16));
            }
            self.committed_raw_text.push_str(new_slice);
        }

        // Parse trailing in-flight block (if any)
        let in_flight = &clean_body[self.committed_raw_text.len()..];
        if in_flight.is_empty() {
            self.committed_lines.clone()
        } else {
            let in_flight_raw =
                crate::markdown::parse_markdown_lines_with_theme(in_flight, colors, width, true);
            let mut out = self.committed_lines.clone();
            for l in in_flight_raw {
                out.extend(wrap_line(l, width as u16));
            }
            out
        }
    }
}

/// A deep, cohesive layout engine that encapsulates text wrapping, sizing,
/// prompt-specific card styling, caching, and cache invalidation.
#[derive(Clone, Default)]
pub(crate) struct TimelineLayoutEngine {
    pub(crate) item_cache: PreparedCache,
    pub(crate) entries: Vec<PreparedTimelineEntry>,
    pub(crate) version: u64,
    pub(crate) timeline_w: usize,
    pub(crate) expand_all: bool,
    pub(crate) expanded_hash: u64,
    pub(crate) is_processing: bool,
    pub(crate) cached_is_processing: bool,
    pub(crate) streaming_text: Option<String>,
    pub(crate) streaming_entry: Option<PreparedTimelineEntry>,
    pub(crate) streaming_cache: StreamingMarkdownCache,
    pub(crate) reasoning_text: Option<String>,
    pub(crate) reasoning_entry: Option<PreparedTimelineEntry>,
    pub(crate) status_text: Option<String>,
    pub(crate) status_entry: Option<PreparedTimelineEntry>,
    /// Set whenever a dynamic (reasoning/streaming) entry is invalidated or
    /// freshly prepared, forcing the next `layout_items` call to rewrite the
    /// tail of `entries` even when the entry *count* is unchanged.
    tail_dirty: bool,
}

impl TimelineLayoutEngine {
    pub fn new() -> Self {
        Self {
            item_cache: PreparedCache::new(),
            entries: Vec::new(),
            version: 0,
            timeline_w: 0,
            expand_all: false,
            expanded_hash: 0,
            is_processing: false,
            cached_is_processing: false,
            streaming_text: None,
            streaming_entry: None,
            streaming_cache: StreamingMarkdownCache::new(),
            reasoning_text: None,
            reasoning_entry: None,
            status_text: None,
            status_entry: None,
            tail_dirty: false,
        }
    }

    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.item_cache.clear();
        self.entries.clear();
        self.version = 0;
        self.timeline_w = 0;
        self.expand_all = false;
        self.expanded_hash = 0;
        self.is_processing = false;
        self.cached_is_processing = false;
        self.streaming_text = None;
        self.streaming_entry = None;
        self.streaming_cache.clear();
        self.reasoning_text = None;
        self.reasoning_entry = None;
        self.status_text = None;
        self.status_entry = None;
        self.tail_dirty = false;
    }

    pub fn set_processing(&mut self, processing: bool) {
        if self.is_processing != processing {
            self.is_processing = processing;
            self.tail_dirty = true;
        }
    }

    pub fn prepare_entries(
        &mut self,
        entries: &[TimelineEntry<'_>],
        width: usize,
        expand_all: bool,
        expanded_items: &std::collections::HashSet<TimelineKey>,
        colors: &ThemeColors,
        nerd: bool,
    ) -> Vec<PreparedTimelineEntry> {
        prepare_timeline_entries(
            entries,
            width,
            expand_all,
            expanded_items,
            colors,
            nerd,
            &mut self.item_cache,
            self.is_processing,
        )
    }

    pub fn set_active_stream(&mut self, streaming: Option<&str>) {
        if streaming.is_none() {
            self.streaming_cache.clear();
        }
        if self.streaming_text.as_deref() != streaming {
            self.streaming_text = streaming.map(String::from);
            self.streaming_entry = None; // Invalidate the single-entry streaming cache
            self.tail_dirty = true;
        }
    }

    pub fn set_active_reasoning(&mut self, reasoning: Option<&str>) {
        if self.reasoning_text.as_deref() != reasoning {
            self.reasoning_text = reasoning.map(String::from);
            self.reasoning_entry = None; // Invalidate the single-entry reasoning cache
            self.tail_dirty = true;
        }
    }

    pub fn set_active_status(&mut self, status: Option<&str>) {
        if self.status_text.as_deref() != status {
            self.status_text = status.map(String::from);
            // The status entry is rebuilt every frame (its spinner text is
            // animated by the caller), so only the text cache is updated here.
            // Appearing/disappearing is detected by the count check in
            // `reconcile_dynamic_tail`; setting `tail_dirty` would force the
            // cached streaming/reasoning entries to be re-cloned each frame.
            self.status_entry = None; // Invalidate the single-entry status cache
        }
    }

    fn prepare_reasoning_entry(&mut self, next_index: usize, colors: &ThemeColors, nerd: bool) {
        if self.reasoning_entry.is_some() {
            return;
        }

        if let Some(ref s) = self.reasoning_text {
            let reasoning_entry = TimelineEntry::reasoning(next_index, s);
            let mut lines = Vec::new();
            let effective_w = self.timeline_w.saturating_sub(2);
            reasoning_entry.render_with_state(
                effective_w,
                self.expand_all,
                &Default::default(), // Live reasoning is never collapsed
                &mut lines,
                colors,
                nerd,
            );

            let mut pre_wrapped_lines = Vec::new();
            for l in lines {
                pre_wrapped_lines.extend(wrap_line(l, effective_w as u16));
            }
            let rows = pre_wrapped_lines.len() as u16;
            let card_style = if self.is_processing {
                CardStyle::ActiveTurn
            } else {
                CardStyle::Assistant
            };
            self.reasoning_entry = Some(PreparedTimelineEntry {
                lines: pre_wrapped_lines,
                rows,
                card_style,
            });
            self.tail_dirty = true;
        }
    }

    fn prepare_streaming_entry(&mut self, _next_index: usize, colors: &ThemeColors, nerd: bool) {
        if self.streaming_entry.is_some() {
            return;
        }

        if let Some(ref s) = self.streaming_text {
            let effective_w = self.timeline_w.saturating_sub(2);
            let pre_wrapped_lines = self.streaming_cache.render(s, effective_w, colors, nerd);
            let rows = pre_wrapped_lines.len() as u16;
            let card_style = if self.is_processing {
                CardStyle::ActiveTurn
            } else {
                CardStyle::Assistant
            };
            self.streaming_entry = Some(PreparedTimelineEntry {
                lines: pre_wrapped_lines,
                rows,
                card_style,
            });
            self.tail_dirty = true;
        }
    }

    fn prepare_status_entry(&mut self, next_index: usize, colors: &ThemeColors, nerd: bool) {
        if let Some(ref s) = self.status_text {
            let status_entry = TimelineEntry::status(next_index, s);
            let mut lines = Vec::new();
            let effective_w = self.timeline_w.saturating_sub(2);
            status_entry.render_with_state(
                effective_w,
                self.expand_all,
                &Default::default(), // Live status is never collapsed
                &mut lines,
                colors,
                nerd,
            );

            let mut pre_wrapped_lines = Vec::new();
            for l in lines {
                pre_wrapped_lines.extend(wrap_line(l, effective_w as u16));
            }
            let rows = pre_wrapped_lines.len() as u16;
            let card_style = if self.is_processing {
                CardStyle::ActiveTurn
            } else {
                CardStyle::None
            };
            self.status_entry = Some(PreparedTimelineEntry {
                lines: pre_wrapped_lines,
                rows,
                card_style,
            });
        }
    }

    /// Evaluates layout for a sequence of conversation lines, automatically managing caching.
    /// If content, width, or expanded state changes, the layout is automatically recalculated.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_items(
        &mut self,
        lines: &[RenderLine],
        timeline_w: usize,
        expand_all: bool,
        expanded_items: &std::collections::HashSet<TimelineKey>,
        colors: &ThemeColors,
        nerd: bool,
        content_version: u64,
    ) -> &[PreparedTimelineEntry] {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Derive a stable hash of expanded_items for cache invalidation.
        let expanded_hash = {
            let mut h = DefaultHasher::new();
            let mut items: Vec<_> = expanded_items.iter().collect();
            items.sort();
            for k in &items {
                k.hash(&mut h);
            }
            h.finish()
        };

        // Check if historical layout remains exactly the same
        let history_clean = self.version == content_version
            && self.timeline_w == timeline_w
            && self.expand_all == expand_all
            && self.expanded_hash == expanded_hash
            && self.cached_is_processing == self.is_processing;

        if history_clean {
            self.reconcile_dynamic_tail(lines.len(), colors, nerd);
            &self.entries
        } else {
            // Cache miss for history — rebuild everything
            if self.timeline_w != timeline_w {
                self.item_cache.clear();
                self.streaming_entry = None; // clear streaming cache as width changed
                self.reasoning_entry = None; // clear reasoning cache as width changed
                self.status_entry = None; // clear status cache as width changed
                self.tail_dirty = true;
            }
            let entries = build_timeline_entries(lines);
            let p = prepare_timeline_entries(
                &entries,
                timeline_w,
                expand_all,
                expanded_items,
                colors,
                nerd,
                &mut self.item_cache,
                self.is_processing,
            );

            self.version = content_version;
            self.timeline_w = timeline_w;
            self.expand_all = expand_all;
            self.expanded_hash = expanded_hash;
            self.cached_is_processing = self.is_processing;

            self.entries = p;
            self.reconcile_dynamic_tail(lines.len(), colors, nerd);
            &self.entries
        }
    }

    /// Ensure `self.entries` ends with exactly the live dynamic tail: the
    /// active-reasoning entry (while thinking), the active-stream entry (while
    /// assistant text is streaming), and the live status line — in that order.
    /// The historical portion is untouched — each dynamic entry is cached
    /// independently and only re-rendered when its content actually changes.
    fn reconcile_dynamic_tail(&mut self, next_index: usize, colors: &ThemeColors, nerd: bool) {
        self.prepare_reasoning_entry(next_index, colors, nerd);
        self.prepare_streaming_entry(next_index, colors, nerd);
        self.prepare_status_entry(next_index, colors, nerd);

        let dynamic_count = usize::from(self.reasoning_entry.is_some())
            + usize::from(self.streaming_entry.is_some())
            + usize::from(self.status_entry.is_some());
        if self.tail_dirty || self.entries.len() != next_index + dynamic_count {
            self.entries.truncate(next_index);
            if let Some(ref entry) = self.reasoning_entry {
                self.entries.push(entry.clone());
            }
            if let Some(ref entry) = self.streaming_entry {
                self.entries.push(entry.clone());
            }
            if let Some(ref entry) = self.status_entry {
                self.entries.push(entry.clone());
            }
            self.tail_dirty = false;
        } else if let Some(ref entry) = self.status_entry {
            // Only the animated status changed between frames.  Refresh its
            // cell in place so the cached streaming/reasoning entries aren't
            // re-cloned on every draw.  The status entry is rebuilt fresh each
            // frame, so it can never be stale here.
            let status_pos = self.entries.len() - 1;
            self.entries[status_pos] = entry.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_last_block_boundary_paragraphs() {
        let single = "Hello world";
        assert_eq!(
            find_last_block_boundary(single),
            0,
            "single in-flight paragraph has no boundary"
        );

        let two_paras = "First paragraph\n\nSecond paragraph";
        let boundary = find_last_block_boundary(two_paras);
        assert_eq!(boundary, "First paragraph\n\n".len());
        assert_eq!(&two_paras[..boundary], "First paragraph\n\n");
    }

    #[test]
    fn test_find_last_block_boundary_code_blocks() {
        let open_code = "Header\n\n```rust\nlet x = 1;\n\nlet y = 2;";
        let boundary = find_last_block_boundary(open_code);
        // Should stop at the header, not inside the unclosed code block
        assert_eq!(boundary, "Header\n\n".len());

        let closed_code = "Header\n\n```rust\nlet x = 1;\n```\nTrailing";
        let boundary2 = find_last_block_boundary(closed_code);
        assert_eq!(boundary2, "Header\n\n```rust\nlet x = 1;\n```\n".len());
    }

    #[test]
    fn test_streaming_markdown_cache_incremental_growth() {
        let colors = ThemeColors::default();
        let mut cache = StreamingMarkdownCache::new();

        // Chunk 1: in-flight first paragraph
        let lines1 = cache.render("Hello world", 80, &colors, false);
        assert!(!lines1.is_empty());
        assert_eq!(cache.committed_raw_text, "");

        // Chunk 2: paragraph 1 completed, paragraph 2 started
        let text2 = "Hello world\n\nSecond paragraph in flight";
        let lines2 = cache.render(text2, 80, &colors, false);
        assert!(lines2.len() >= lines1.len());
        assert_eq!(cache.committed_raw_text, "Hello world\n\n");
        let committed_count = cache.committed_lines.len();

        // Chunk 3: paragraph 2 continues
        let text3 = "Hello world\n\nSecond paragraph in flight with more tokens";
        let lines3 = cache.render(text3, 80, &colors, false);
        assert!(lines3.len() >= lines2.len());
        // Committed lines remained frozen/reused
        assert_eq!(cache.committed_lines.len(), committed_count);
    }

    #[test]
    fn test_streaming_markdown_cache_width_invalidation() {
        let colors = ThemeColors::default();
        let mut cache = StreamingMarkdownCache::new();
        cache.render("Hello world\n\nSecond paragraph\n\n", 80, &colors, false);
        assert_eq!(cache.cached_width, 80);
        assert!(!cache.committed_raw_text.is_empty());

        // Width resize invalidates cache cleanly
        cache.render("Hello world\n\nSecond paragraph\n\n", 120, &colors, false);
        assert_eq!(cache.cached_width, 120);
    }

    #[test]
    fn test_active_turn_container_card_style() {
        let colors = ThemeColors::default();
        let expanded = std::collections::HashSet::new();
        let mut item_cache = PreparedCache::new();

        let lines = vec![
            RenderLine::UserMessage("Please inspect the codebase".to_string()),
            RenderLine::ToolCall {
                name: "read_file".to_string(),
                preview: "src/main.rs".to_string(),
            },
            RenderLine::ToolResult {
                is_error: false,
                content: "fn main() {}".to_string(),
            },
        ];
        let entries = build_timeline_entries(&lines);

        // While is_processing is true: tool calls and results get CardStyle::ActiveTurn
        let in_flight = prepare_timeline_entries(
            &entries,
            80,
            false,
            &expanded,
            &colors,
            true,
            &mut item_cache,
            true,
        );
        assert_eq!(in_flight[0].card_style, CardStyle::User);
        assert_eq!(in_flight[1].card_style, CardStyle::ActiveTurn);
        assert_eq!(in_flight[2].card_style, CardStyle::ActiveTurn);

        // When is_processing finishes: tool calls and results settle into CardStyle::ToolCall
        let mut settled_cache = PreparedCache::new();
        let completed = prepare_timeline_entries(
            &entries,
            80,
            false,
            &expanded,
            &colors,
            true,
            &mut settled_cache,
            false,
        );
        assert_eq!(completed[0].card_style, CardStyle::User);
        assert_eq!(completed[1].card_style, CardStyle::ToolCall);
        assert_eq!(completed[2].card_style, CardStyle::ToolCall);
    }
}
