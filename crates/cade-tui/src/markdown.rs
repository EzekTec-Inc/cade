use crate::colors::ThemeColors;
use crate::colors::ThemeColorsExt;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
};
use std::sync::LazyLock;
use unicode_width::UnicodeWidthStr;

#[cfg(feature = "syntax-highlighting")]
use syntect::easy::HighlightLines;
#[cfg(feature = "syntax-highlighting")]
use syntect::highlighting::Style as SyntectStyle;
#[cfg(feature = "syntax-highlighting")]
use syntect::parsing::SyntaxSet;

#[cfg(feature = "syntax-highlighting")]
pub(crate) static SYNTAX_SET: LazyLock<SyntaxSet> =
    LazyLock::new(SyntaxSet::load_defaults_newlines);

#[cfg(feature = "syntax-highlighting")]
#[allow(dead_code)]
pub(crate) fn syntect_to_tui_style(style: SyntectStyle) -> Style {
    let mut s = Style::default().fg(RC::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));
    use syntect::highlighting::FontStyle;
    let mut modifier = Modifier::empty();
    if style.font_style.contains(FontStyle::BOLD) {
        modifier |= Modifier::BOLD;
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        modifier |= Modifier::ITALIC;
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        modifier |= Modifier::UNDERLINED;
    }
    if !modifier.is_empty() {
        s = s.add_modifier(modifier);
    }
    s
}

/// Left margin applied to all body content (paragraphs, headings, lists, etc.).
/// Set to empty string so the Block container's left gutter rail and padding
/// are the sole source of truth for viewport margin alignment.
const INDENT: &str = "";

/// Extra indent inside code blocks.
const CODE_INDENT: &str = "  ";

/// Style for the dim code-block border lines (┌── / └──).
fn code_border_style(colors: &ThemeColors) -> Style {
    colors.md_code_block_border()
}

/// Word-wrap a vector of styled spans into one or more `Line`s, breaking
/// only on whitespace and preserving each span's style across line breaks.
///
/// Behaviour:
/// - Splits each span's text into whitespace-delimited words; each word
///   carries the span's style.
/// - Greedily fills lines up to `max_width` Unicode display columns.
/// - Words longer than `max_width` are placed on their own line (and may
///   still overflow — ratatui's `Wrap` will then break them mid-word).
/// - Continuation lines start with `continuation_prefix` (typically the
///   same indent as the first line) so wrapped paragraphs stay aligned.
/// - Spans with embedded line breaks are NOT special-cased — the parser
///   already converts `SoftBreak`/`HardBreak` into spaces / explicit pushes.
///
/// `max_width = 0` disables wrapping entirely (a single Line is returned).
fn wrap_spans_to_width(
    spans: Vec<Span<'static>>,
    max_width: usize,
    continuation_prefix: Option<Span<'static>>,
) -> Vec<Line<'static>> {
    if spans.is_empty() {
        return vec![];
    }
    if max_width == 0 {
        return vec![Line::from(spans)];
    }

    // Width of the leading prefix span (if any) — counts toward the first
    // line's used width so the first word doesn't immediately overflow.
    let prefix_width = |s: &Span<'_>| UnicodeWidthStr::width(s.content.as_ref());

    // Compute width of any leading raw-INDENT/glyph spans so we treat them
    // as already-laid-out prefix on the first line.  Walk forward over
    // spans whose content is whitespace-only — those are layout spans.
    let mut first_prefix_w = 0usize;
    for s in &spans {
        let c = s.content.as_ref();
        if c.is_empty() {
            continue;
        }
        if c.chars().all(|ch| ch.is_whitespace()) {
            first_prefix_w += prefix_width(s);
        } else {
            break;
        }
    }

    let cont_w = continuation_prefix.as_ref().map(prefix_width).unwrap_or(0);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut current_w: usize = first_prefix_w;
    let mut on_first_line = true;

    let push_current = |lines: &mut Vec<Line<'static>>,
                        current: &mut Vec<Span<'static>>,
                        on_first_line: &mut bool,
                        current_w: &mut usize,
                        cont_prefix: &Option<Span<'static>>| {
        if !current.is_empty() {
            lines.push(Line::from(std::mem::take(current)));
        }
        *on_first_line = false;
        if let Some(p) = cont_prefix.clone() {
            current.push(p);
            *current_w = cont_w;
        } else {
            *current_w = 0;
        }
    };

    for span in spans {
        let style = span.style;
        let content = span.content.into_owned();

        // Layout-only spans (pure whitespace at the leading edge) pass through.
        if content.is_empty() {
            continue;
        }

        // Tokenise: split_inclusive(' ') keeps the trailing space attached
        // to the word, so widths accumulate correctly.
        let mut buf = String::new();
        let mut buf_w = 0usize;

        let flush_buf = |buf: &mut String,
                         buf_w: &mut usize,
                         current: &mut Vec<Span<'static>>,
                         current_w: &mut usize| {
            if !buf.is_empty() {
                current.push(Span::styled(std::mem::take(buf), style));
                *current_w += *buf_w;
                *buf_w = 0;
            }
        };

        for word in content.split_inclusive(' ') {
            let word_w = UnicodeWidthStr::width(word);
            // If adding this word would overflow, flush + wrap.
            // Allow trailing whitespace to fit even if it pushes one over —
            // the trailing space is invisible at the line edge.
            let trimmed_w = UnicodeWidthStr::width(word.trim_end_matches(' '));
            if current_w + buf_w + trimmed_w > max_width && (current_w + buf_w) > 0 {
                // Flush the in-progress span buffer to current line, then wrap.
                flush_buf(&mut buf, &mut buf_w, &mut current, &mut current_w);
                push_current(
                    &mut lines,
                    &mut current,
                    &mut on_first_line,
                    &mut current_w,
                    &continuation_prefix,
                );
                // Drop leading spaces of the wrapped word so wrapped lines
                // do not start with a stray space.
                let stripped = word.trim_start_matches(' ');
                if !stripped.is_empty() {
                    buf.push_str(stripped);
                    buf_w += UnicodeWidthStr::width(stripped);
                }
            } else {
                buf.push_str(word);
                buf_w += word_w;
            }
        }

        flush_buf(&mut buf, &mut buf_w, &mut current, &mut current_w);
    }

    if !current.is_empty() {
        lines.push(Line::from(current));
    }

    // Suppress unused-variable lint when on_first_line is only updated.
    let _ = on_first_line;

    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CalloutKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl CalloutKind {
    pub(crate) fn from_str(s: &str) -> Option<(Self, &str)> {
        let trimmed = s.trim_start();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("[!note]") {
            Some((Self::Note, &trimmed[7..]))
        } else if lower.starts_with("[!info]") {
            Some((Self::Note, &trimmed[7..]))
        } else if lower.starts_with("[!tip]") {
            Some((Self::Tip, &trimmed[6..]))
        } else if lower.starts_with("[!idea]") {
            Some((Self::Tip, &trimmed[7..]))
        } else if lower.starts_with("[!important]") {
            Some((Self::Important, &trimmed[12..]))
        } else if lower.starts_with("[!warning]") {
            Some((Self::Warning, &trimmed[10..]))
        } else if lower.starts_with("[!caution]") {
            Some((Self::Caution, &trimmed[10..]))
        } else {
            None
        }
    }

    pub(crate) fn label_and_color(
        &self,
        colors: &ThemeColors,
    ) -> (&'static str, ratatui::style::Color) {
        match self {
            Self::Note => ("ℹ Note", colors.c_border_accent()),
            Self::Tip => ("💡 Tip", colors.c_success()),
            Self::Important => ("⚡ Important", colors.c_primary()),
            Self::Warning => ("⚠ Warning", colors.c_warning()),
            Self::Caution => ("🚨 Caution", colors.c_error()),
        }
    }
}

pub fn parse_markdown_lines(text: &str) -> Vec<Line<'static>> {
    parse_markdown_lines_with_theme(text, &ThemeColors::default(), 0, true)
}

/// Parse markdown text into styled `Line`s.
///
/// `max_width` is the available viewport width in columns.  When `> 0`
/// it is used to cap table column widths and truncate long code-block
/// lines so that rendered content stays within the viewport.  Pass `0`
/// to disable width-capping (legacy callers).
pub fn parse_markdown_lines_with_theme(
    text: &str,
    colors: &ThemeColors,
    max_width: usize,
    is_expanded: bool,
) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(text, options);

    let mut lines = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();

    let mut style_stack = vec![Style::default()];

    let mut in_blockquote = false;
    let mut in_code_block = false;
    let mut current_lang = String::new();

    let mut list_depth: usize = 0;
    let mut list_counters: Vec<Option<u64>> = Vec::new();

    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut table_alignments: Vec<Alignment> = Vec::new();
    let mut current_cell = String::new();
    let mut in_table = false;
    let mut code_block_buf = String::new();

    // V7: stack of image dest URLs for nested image tags (rare but legal).
    let mut image_url_stack: Vec<String> = Vec::new();

    // Track whether we just closed a block element so we can insert spacing.
    let mut last_was_block_end = false;
    let mut current_callout: Option<CalloutKind> = None;
    let mut callout_buf: Option<String> = None;

    let push_line = |lines: &mut Vec<Line<'static>>,
                     spans: &mut Vec<Span<'static>>,
                     blockquote: bool,
                     callout: Option<CalloutKind>| {
        if !spans.is_empty() {
            let mut prefix_spans = Vec::new();
            if blockquote {
                let rail_color = callout
                    .map(|c| c.label_and_color(colors).1)
                    .unwrap_or_else(|| colors.c_md_quote_border());
                prefix_spans.push(Span::styled(
                    format!("{INDENT}▎ "),
                    Style::default().fg(rail_color),
                ));
            }
            prefix_spans.append(spans);
            lines.push(Line::from(prefix_spans));
        }
    };

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    // Insert a blank line before paragraphs when following another
                    // block element (heading, code block, list, previous paragraph).
                    if last_was_block_end && !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    last_was_block_end = false;
                    // Indent paragraph body so it aligns with headings/lists.
                    // Skip the indent inside blockquotes (the "▎ " prefix already
                    // provides visual inset) and inside list items (Tag::Item
                    // emits its own bullet/number indent prefix).
                    if !in_blockquote && list_depth == 0 && current_spans.is_empty() {
                        current_spans.push(Span::raw(INDENT.to_string()));
                    }
                }
                Tag::Heading { level, .. } => {
                    // Blank line before every heading for visual breathing room.
                    if !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    last_was_block_end = false;

                    let style = match level {
                        HeadingLevel::H1 => Style::default()
                            .fg(colors.c_md_heading())
                            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                        HeadingLevel::H2 => Style::default()
                            .fg(colors.c_md_heading())
                            .add_modifier(Modifier::BOLD),
                        HeadingLevel::H3 => Style::default()
                            .fg(colors.c_md_heading())
                            .add_modifier(Modifier::BOLD),
                        _ => colors.md_heading(),
                    };
                    style_stack.push(style);

                    let glyph = match level {
                        HeadingLevel::H1 => "◆ ",
                        HeadingLevel::H2 => "◇ ",
                        HeadingLevel::H3 => "▸ ",
                        _ => "· ",
                    };
                    current_spans.push(Span::styled(format!("{INDENT}{glyph}"), style));
                }
                Tag::BlockQuote(_) => {
                    if last_was_block_end && !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    last_was_block_end = false;
                    in_blockquote = true;
                    current_callout = None;
                    callout_buf = Some(String::new());
                }
                Tag::CodeBlock(kind) => {
                    if last_was_block_end && !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    last_was_block_end = false;
                    in_code_block = true;
                    code_block_buf.clear();

                    if let CodeBlockKind::Fenced(lang) = kind {
                        current_lang = lang.to_string();
                    } else {
                        current_lang.clear();
                    }
                }
                Tag::List(start) => {
                    // Pre-wrap any accumulated parent list item spans before starting nested list
                    if !current_spans.is_empty() {
                        if max_width > 0 {
                            let pad = "  ".repeat(list_depth.saturating_sub(1));
                            let cont_prefix = if list_depth == 0 {
                                Some(Span::raw(INDENT.to_string()))
                            } else {
                                Some(Span::raw(format!("{INDENT}    {pad}")))
                            };
                            let spans = std::mem::take(&mut current_spans);
                            let prefixed = if in_blockquote {
                                let mut v = vec![Span::styled(
                                    format!("{INDENT}▎ "),
                                    colors.md_quote_border(),
                                )];
                                v.extend(spans);
                                v
                            } else {
                                spans
                            };
                            let wrapped = wrap_spans_to_width(prefixed, max_width, cont_prefix);
                            for l in wrapped {
                                lines.push(l);
                            }
                        } else {
                            push_line(
                                &mut lines,
                                &mut current_spans,
                                in_blockquote,
                                current_callout,
                            );
                        }
                    }

                    // Blank line before a top-level list.
                    if list_depth == 0 && last_was_block_end && !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    last_was_block_end = false;
                    list_depth += 1;
                    list_counters.push(start);
                }
                Tag::Item => {
                    let indent_padding = "  ".repeat(list_depth.saturating_sub(1));
                    if let Some(counters) = list_counters.last_mut() {
                        if let Some(count) = counters {
                            current_spans.push(Span::raw(format!("{INDENT}  {indent_padding}")));
                            current_spans.push(Span::styled(
                                format!("{count}. "),
                                Style::default()
                                    .fg(colors.c_md_link())
                                    .add_modifier(Modifier::BOLD),
                            ));
                            *count += 1;
                        } else {
                            current_spans.push(Span::raw(format!("{INDENT}  {indent_padding}")));
                            current_spans.push(Span::styled("• ", colors.md_list_bullet()));
                        }
                    }
                }
                Tag::Emphasis => {
                    let s = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::ITALIC);
                    style_stack.push(s);
                }
                Tag::Strong => {
                    let s = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .fg(colors.c_text_primary())
                        .add_modifier(Modifier::BOLD);
                    style_stack.push(s);
                }
                Tag::Strikethrough => {
                    let s = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .add_modifier(Modifier::CROSSED_OUT);
                    style_stack.push(s);
                }
                Tag::Table(alignments) => {
                    if last_was_block_end && !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    last_was_block_end = false;
                    in_table = true;
                    table_rows.clear();
                    table_alignments = alignments;
                }
                Tag::TableHead | Tag::TableRow => {
                    table_rows.push(Vec::new());
                }
                Tag::TableCell => {
                    current_cell.clear();
                }
                Tag::Link { .. } => {
                    let s = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .fg(colors.c_md_link())
                        .add_modifier(Modifier::UNDERLINED);
                    style_stack.push(s);
                }
                Tag::Image { dest_url, .. } => {
                    // V7: emit a glyph + bracketed alt text so images don't
                    // silently disappear.  The alt-text content arrives as
                    // inner `Event::Text` events between Start and End — we
                    // push a leading "🖼  [" here, then the text events, then
                    // the closing "] (url)" at TagEnd::Image.
                    let img_style = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .fg(colors.c_md_link());
                    current_spans.push(Span::styled("🖼  [".to_string(), img_style));
                    // Push image text style on the stack so inner text
                    // inherits the link color but stays unstyled otherwise.
                    style_stack.push(img_style);
                    // Stash the URL on a side channel via the title-like
                    // suffix appended at TagEnd::Image.  We use a sentinel
                    // span content carrying the URL so the end tag can read
                    // it; cheaper than a separate stack.
                    if !dest_url.is_empty() {
                        // Marker — picked up at TagEnd::Image (see below).
                        // Empty span carries the URL in its content but
                        // renders as nothing because we replace it on End.
                        // (Simpler: just remember the URL in a local var.)
                    }
                    image_url_stack.push(dest_url.into_string());
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    if let Some(buf) = callout_buf.take()
                        && !buf.is_empty()
                    {
                        let style = style_stack.last().copied().unwrap_or_default();
                        current_spans.push(Span::styled(buf, style));
                    }
                    // Pre-wrap paragraph spans at viewport width so continuation
                    // lines get a proper INDENT prefix (or blockquote bar) — ratatui's
                    // `Wrap { trim: false }` would otherwise leave wrapped lines
                    // hanging flush-left.
                    let quote_color = current_callout
                        .map(|c| c.label_and_color(colors).1)
                        .unwrap_or_else(|| colors.c_md_quote_border());
                    if max_width > 0 && !current_spans.is_empty() {
                        let cont_prefix = if in_blockquote {
                            Some(Span::styled(
                                format!("{INDENT}▎ "),
                                Style::default().fg(quote_color),
                            ))
                        } else if list_depth == 0 {
                            Some(Span::raw(INDENT.to_string()))
                        } else {
                            // Inside list items the bullet/number prefix is already
                            // on the first line; continuations align under the body.
                            let pad = "  ".repeat(list_depth.saturating_sub(1));
                            Some(Span::raw(format!("{INDENT}    {pad}")))
                        };
                        let spans = std::mem::take(&mut current_spans);
                        let prefixed = if in_blockquote {
                            // Insert blockquote bar at the head of the first line.
                            let mut v = vec![Span::styled(
                                format!("{INDENT}▎ "),
                                Style::default().fg(quote_color),
                            )];
                            v.extend(spans);
                            v
                        } else {
                            spans
                        };
                        let wrapped = wrap_spans_to_width(prefixed, max_width, cont_prefix);
                        for l in wrapped {
                            lines.push(l);
                        }
                    } else {
                        push_line(
                            &mut lines,
                            &mut current_spans,
                            in_blockquote,
                            current_callout,
                        );
                    }
                    last_was_block_end = true;
                }
                TagEnd::Heading(_) => {
                    push_line(
                        &mut lines,
                        &mut current_spans,
                        in_blockquote,
                        current_callout,
                    );
                    style_stack.pop();
                    last_was_block_end = true;
                }
                TagEnd::BlockQuote(_) => {
                    if let Some(buf) = callout_buf.take()
                        && !buf.is_empty()
                    {
                        let style = style_stack.last().copied().unwrap_or_default();
                        current_spans.push(Span::styled(buf, style));
                    }
                    push_line(
                        &mut lines,
                        &mut current_spans,
                        in_blockquote,
                        current_callout,
                    );
                    in_blockquote = false;
                    current_callout = None;
                    callout_buf = None;
                    last_was_block_end = true;
                }
                TagEnd::CodeBlock => {
                    push_line(
                        &mut lines,
                        &mut current_spans,
                        in_blockquote,
                        current_callout,
                    );
                    let body_width = max_width.saturating_sub(6);
                    let prefix_span =
                        Span::styled(format!("{INDENT}{CODE_INDENT}"), code_border_style(colors));

                    let total_lines = code_block_buf.lines().count();
                    let should_collapse = !is_expanded && total_lines > 15;

                    let lines_to_render = if should_collapse {
                        code_block_buf.lines().take(3).collect::<Vec<_>>()
                    } else {
                        code_block_buf.lines().collect::<Vec<_>>()
                    };

                    // Top border with language and line counts
                    let border_w = if max_width > 2 {
                        max_width.saturating_sub(INDENT.len()).max(8)
                    } else {
                        33
                    };

                    let line_badge = format!("[{total_lines} lines]");
                    let line_badge_w = line_badge.len();
                    if current_lang.is_empty() {
                        let dashes = "─".repeat(border_w.saturating_sub(line_badge_w + 4));
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("{INDENT}╭─ {dashes} ["),
                                code_border_style(colors),
                            ),
                            Span::styled(
                                format!("{total_lines} lines"),
                                Style::default().fg(colors.c_text_muted()),
                            ),
                            Span::styled("] ─╮", code_border_style(colors)),
                        ]));
                    } else {
                        let prefix = format!("╭─ [{}] ", current_lang);
                        let prefix_w = UnicodeWidthStr::width(prefix.as_str());
                        let dashes =
                            "─".repeat(border_w.saturating_sub(prefix_w + line_badge_w + 4));
                        lines.push(Line::from(vec![
                            Span::styled(format!("{INDENT}╭─ ["), code_border_style(colors)),
                            Span::styled(
                                current_lang.clone(),
                                Style::default()
                                    .fg(colors.c_primary())
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(format!("] {dashes} ["), code_border_style(colors)),
                            Span::styled(
                                format!("{total_lines} lines"),
                                Style::default().fg(colors.c_text_muted()),
                            ),
                            Span::styled("] ─╮", code_border_style(colors)),
                        ]));
                    }

                    let is_diff = current_lang == "diff";

                    #[cfg(feature = "syntax-highlighting")]
                    let dyn_theme = crate::colors::generate_syntect_theme(colors);
                    #[cfg(feature = "syntax-highlighting")]
                    let syntax = SYNTAX_SET
                        .find_syntax_by_token(&current_lang)
                        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
                    #[cfg(feature = "syntax-highlighting")]
                    let mut highlighter = Some(HighlightLines::new(syntax, &dyn_theme));

                    for raw_line in lines_to_render {
                        let mut spans = vec![prefix_span.clone()];

                        if is_diff {
                            let style = if raw_line.starts_with('+') {
                                Style::default()
                                    .fg(colors.c_diff_added())
                                    .bg(colors.c_bg_surface1())
                            } else if raw_line.starts_with('-') {
                                Style::default()
                                    .fg(colors.c_diff_removed())
                                    .bg(colors.c_bg_surface1())
                            } else if raw_line.starts_with('@') {
                                Style::default()
                                    .fg(colors.c_border_accent())
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                colors.text_primary()
                            };

                            let padded = if body_width > 0 && raw_line.len() < body_width {
                                format!(
                                    "{}{}",
                                    raw_line,
                                    " ".repeat(body_width.saturating_sub(raw_line.len()))
                                )
                            } else {
                                raw_line.to_string()
                            };

                            spans.push(Span::styled(padded, style));
                        } else {
                            #[cfg(feature = "syntax-highlighting")]
                            {
                                let mut line_str = raw_line.to_string();
                                if !line_str.ends_with('\n') {
                                    line_str.push('\n');
                                }
                                if let Some(ref mut h) = highlighter {
                                    if let Ok(regions) = h.highlight_line(&line_str, &SYNTAX_SET) {
                                        for (style, text) in regions {
                                            let c = style.foreground;
                                            let tc = ratatui::style::Color::Rgb(c.r, c.g, c.b);
                                            let mut s = Style::default().fg(tc);
                                            if style
                                                .font_style
                                                .contains(syntect::highlighting::FontStyle::BOLD)
                                            {
                                                s = s.add_modifier(Modifier::BOLD);
                                            }
                                            if style
                                                .font_style
                                                .contains(syntect::highlighting::FontStyle::ITALIC)
                                            {
                                                s = s.add_modifier(Modifier::ITALIC);
                                            }
                                            spans.push(Span::styled(text.replace('\n', ""), s));
                                        }
                                    } else {
                                        spans.push(Span::styled(
                                            raw_line.to_string(),
                                            colors.text_primary(),
                                        ));
                                    }
                                } else {
                                    spans.push(Span::styled(
                                        raw_line.to_string(),
                                        colors.text_primary(),
                                    ));
                                }
                            }
                            #[cfg(not(feature = "syntax-highlighting"))]
                            {
                                spans.push(Span::styled(
                                    raw_line.to_string(),
                                    colors.text_primary(),
                                ));
                            }
                        }

                        if in_blockquote {
                            let quote_color = current_callout
                                .map(|c| c.label_and_color(colors).1)
                                .unwrap_or_else(|| colors.c_md_quote_border());
                            let mut b_spans = vec![Span::styled(
                                format!("{INDENT}▎ "),
                                Style::default().fg(quote_color),
                            )];
                            b_spans.extend(spans);
                            lines.push(Line::from(b_spans));
                        } else {
                            lines.push(Line::from(spans));
                        }
                    }

                    if should_collapse {
                        let hidden_count = total_lines.saturating_sub(3);
                        let pill =
                            format!(" [▾ {hidden_count} more lines · Press Ctrl+O to expand] ");
                        let pill_w = UnicodeWidthStr::width(pill.as_str());
                        let dashes = "─".repeat(border_w.saturating_sub(pill_w + 2));
                        lines.push(Line::from(vec![
                            Span::styled(format!("{INDENT}╰{dashes}"), code_border_style(colors)),
                            Span::styled(
                                pill,
                                Style::default()
                                    .fg(colors.c_border_accent())
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled("─╯", code_border_style(colors)),
                        ]));
                    } else {
                        let dashes = "─".repeat(border_w.saturating_sub(2));
                        lines.push(Line::from(Span::styled(
                            format!("{INDENT}╰{dashes}╯"),
                            code_border_style(colors),
                        )));
                    }

                    in_code_block = false;
                    code_block_buf.clear();
                    current_lang.clear();
                    last_was_block_end = true;
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    list_counters.pop();
                    if list_depth == 0 {
                        last_was_block_end = true;
                    }
                }
                TagEnd::Item => {
                    if max_width > 0 && !current_spans.is_empty() {
                        let pad = "  ".repeat(list_depth.saturating_sub(1));
                        let cont_prefix = Some(Span::raw(format!("{INDENT}    {pad}")));
                        let spans = std::mem::take(&mut current_spans);
                        let quote_color = current_callout
                            .map(|c| c.label_and_color(colors).1)
                            .unwrap_or_else(|| colors.c_md_quote_border());
                        let prefixed = if in_blockquote {
                            let mut v = vec![Span::styled(
                                format!("{INDENT}▎ "),
                                Style::default().fg(quote_color),
                            )];
                            v.extend(spans);
                            v
                        } else {
                            spans
                        };
                        let wrapped = wrap_spans_to_width(prefixed, max_width, cont_prefix);
                        for l in wrapped {
                            lines.push(l);
                        }
                    } else {
                        push_line(
                            &mut lines,
                            &mut current_spans,
                            in_blockquote,
                            current_callout,
                        );
                    }
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::Table => {
                    in_table = false;
                    lines.extend(render_table_data(
                        &table_rows,
                        &table_alignments,
                        colors,
                        max_width,
                    ));
                    table_rows.clear();
                    table_alignments.clear();
                    last_was_block_end = true;
                }
                TagEnd::TableCell => {
                    if let Some(last_row) = table_rows.last_mut() {
                        last_row.push(current_cell.clone());
                    }
                    current_cell.clear();
                }
                TagEnd::Link => {
                    style_stack.pop();
                }
                TagEnd::Image => {
                    let url = image_url_stack.pop().unwrap_or_default();
                    let img_style = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .fg(colors.c_md_link());
                    if !url.is_empty() {
                        current_spans.push(Span::styled(format!("] ({url})"), img_style));
                    } else {
                        current_spans.push(Span::styled("]".to_string(), img_style));
                    }
                    style_stack.pop();
                }
                _ => {}
            },
            Event::TaskListMarker(checked) => {
                if current_spans.last().map(|s| s.content.as_ref()) == Some("• ") {
                    current_spans.pop();
                }
                if checked {
                    current_spans.push(Span::styled(
                        "☑ ",
                        Style::default()
                            .fg(colors.c_success())
                            .add_modifier(Modifier::BOLD),
                    ));
                } else {
                    current_spans.push(Span::styled(
                        "☐ ",
                        colors.border_muted().add_modifier(Modifier::BOLD),
                    ));
                }
            }
            Event::Text(text) => {
                if in_table {
                    current_cell.push_str(&text);
                } else if in_code_block {
                    code_block_buf.push_str(&text);
                } else {
                    let style = style_stack.last().copied().unwrap_or_default();

                    // Check for GFM callouts (> [!NOTE], > [!WARNING], etc.)
                    if in_blockquote && let Some(ref mut buf) = callout_buf {
                        buf.push_str(&text);
                        if buf.starts_with('[') {
                            if let Some(close_idx) = buf.find(']') {
                                let tag_str = &buf[..=close_idx];
                                let rest = buf[close_idx + 1..].to_string();
                                if let Some((kind, _)) = CalloutKind::from_str(tag_str) {
                                    current_callout = Some(kind);
                                    let (label, color) = kind.label_and_color(colors);
                                    current_spans.push(Span::styled(
                                        format!("[{label}] "),
                                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                                    ));
                                    let rest_trimmed = rest.trim_start();
                                    if !rest_trimmed.is_empty() {
                                        current_spans
                                            .push(Span::styled(rest_trimmed.to_string(), style));
                                    }
                                    callout_buf = None;
                                    continue;
                                } else {
                                    let flushed = std::mem::take(buf);
                                    callout_buf = None;
                                    current_spans.push(Span::styled(flushed, style));
                                    continue;
                                }
                            } else if buf.len() > 30 {
                                let flushed = std::mem::take(buf);
                                callout_buf = None;
                                current_spans.push(Span::styled(flushed, style));
                                continue;
                            } else {
                                // Still waiting for closing ']'
                                continue;
                            }
                        } else {
                            let flushed = std::mem::take(buf);
                            callout_buf = None;
                            current_spans.push(Span::styled(flushed, style));
                            continue;
                        }
                    }

                    // Fallback for GFM Task Lists (- [ ], - [x]) if not parsed as TaskListMarker
                    if list_depth > 0 && list_counters.last() == Some(&None) {
                        if let Some(stripped) = text.strip_prefix("[ ] ") {
                            if current_spans.last().map(|s| s.content.as_ref()) == Some("• ") {
                                current_spans.pop();
                                current_spans.push(Span::styled(
                                    "☐ ",
                                    colors.border_muted().add_modifier(Modifier::BOLD),
                                ));
                                current_spans.push(Span::styled(stripped.to_string(), style));
                                continue;
                            }
                        } else if (text.starts_with("[x] ") || text.starts_with("[X] "))
                            && current_spans.last().map(|s| s.content.as_ref()) == Some("• ")
                        {
                            let stripped = text
                                .strip_prefix("[x] ")
                                .or_else(|| text.strip_prefix("[X] "))
                                .unwrap_or("");
                            current_spans.pop();
                            current_spans.push(Span::styled(
                                "☑ ",
                                Style::default()
                                    .fg(colors.c_success())
                                    .add_modifier(Modifier::BOLD),
                            ));
                            current_spans.push(Span::styled(
                                stripped.to_string(),
                                style
                                    .fg(colors.c_text_muted())
                                    .add_modifier(Modifier::CROSSED_OUT),
                            ));
                            continue;
                        }
                    }

                    current_spans.push(Span::styled(text.into_string(), style));
                }
            }
            Event::Code(text) => {
                if in_table {
                    // B6: inside tables, drop the raw backticks so inline code
                    // looks consistent with the outside-table " code " form.
                    // Cells are plain-string (no styling), so we cannot apply
                    // the inverse-dim background — keep just the surrounding
                    // spaces as a visual cue.
                    current_cell.push_str(&format!(" {text} "));
                } else {
                    // Inline code: bright on a subtle background via reversed dim
                    let style = colors.md_code().bg(colors.c_bg_surface1());
                    current_spans.push(Span::styled(format!(" {text} "), style));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_table {
                    current_cell.push(' ');
                } else if in_code_block {
                    push_line(
                        &mut lines,
                        &mut current_spans,
                        in_blockquote,
                        current_callout,
                    );
                } else {
                    if matches!(event, Event::HardBreak) {
                        push_line(
                            &mut lines,
                            &mut current_spans,
                            in_blockquote,
                            current_callout,
                        );
                    } else {
                        let style = style_stack.last().copied().unwrap_or_default();
                        current_spans.push(Span::styled(" ", style));
                    }
                }
            }
            Event::Rule => {
                lines.push(Line::from(""));
                let hr_w = if max_width > 2 {
                    max_width.saturating_sub(INDENT.len()).max(8)
                } else {
                    40
                };
                lines.push(Line::from(Span::styled(
                    format!("{INDENT}{}", "─".repeat(hr_w)),
                    colors.md_hr(),
                )));
                lines.push(Line::from(""));
                last_was_block_end = true;
            }
            _ => {}
        }
    }

    push_line(&mut lines, &mut current_spans, false, None);

    lines
}

/// Render a parsed markdown table to styled lines.
///
/// Layout:
///   ┌─────────┬──────┐    top border
///   │ Header  │  Hdr │    header row (bold, themed)
///   ├─────────┼──────┤    header/body separator
///   │ cell    │ cell │    body rows
///   └─────────┴──────┘    bottom border
///
/// Column widths and cell text are measured using Unicode display width
/// (`UnicodeWidthStr::width`) so emoji, CJK, and accented Latin chars
/// align correctly.  When the natural total width exceeds `max_width`,
/// columns are proportionally shrunk to a `min_col` floor, and individual
/// cells are truncated with a trailing `…` to fit.
///
/// `alignments` should have the same length as the widest row; trailing
/// columns without an explicit alignment fall back to `Alignment::None`
/// (rendered left-aligned).
fn render_table_data(
    data: &[Vec<String>],
    alignments: &[Alignment],
    colors: &ThemeColors,
    max_width: usize,
) -> Vec<Line<'static>> {
    if data.is_empty() {
        return vec![];
    }
    let num_cols = data.iter().map(|row| row.len()).max().unwrap_or(0);
    if num_cols == 0 {
        return vec![];
    }

    // Width measurement uses Unicode display width, NOT byte length, so
    // emoji and CJK characters align correctly in the rendered grid.
    let mut col_widths = vec![0usize; num_cols];
    for row in data {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                col_widths[i] = col_widths[i].max(UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }

    // Cap column widths so the total row fits within max_width.
    //
    // Each row layout:  INDENT + "│ " + col0 + " │ " + col1 + " │ " + … + " │"
    //   prefix:    INDENT(2) + "│ "(2)            = 4
    //   suffix:    " │"(2)                        = 2
    //   inter-col: " │ "(3) × (num_cols - 1)
    //
    // Total non-content overhead = 6 + 3 * (num_cols - 1).  This MUST match
    // the separator/border calculations below so the borders line up exactly
    // with the cell pipes.
    let row_overhead = 6 + 3 * num_cols.saturating_sub(1);
    if max_width > 0 {
        let budget = max_width.saturating_sub(row_overhead);
        let total: usize = col_widths.iter().sum();
        if total > budget && budget > 0 {
            let min_col = 3usize;
            let min_total = min_col * num_cols;
            let target = budget.max(min_total);
            for w in col_widths.iter_mut() {
                let share = (*w as f64 / total as f64 * target as f64).floor() as usize;
                *w = share.max(min_col);
            }
        }
    }

    let border_style = colors.md_code_block_border();
    let mut lines = Vec::new();

    // ── Top border:  ╭─────┬─────╮ ────────────────────────────────────────
    let mut top_spans = vec![Span::styled(format!("{INDENT}╭─"), border_style)];
    for (i, w) in col_widths.iter().enumerate() {
        top_spans.push(Span::styled("─".repeat(*w), border_style));
        if i < num_cols - 1 {
            top_spans.push(Span::styled("─┬─", border_style));
        }
    }
    top_spans.push(Span::styled("─╮", border_style));
    lines.push(Line::from(top_spans));

    // ── Data rows + header separator ────────────────────────────────────────
    for (row_idx, row) in data.iter().enumerate() {
        let is_header = row_idx == 0;
        let is_even_body = row_idx > 0 && row_idx % 2 == 0;
        let row_bg = if is_header {
            Some(colors.c_bg_surface1())
        } else if is_even_body {
            Some(colors.c_bg_surface1())
        } else {
            None
        };

        let mut spans = vec![Span::styled(format!("{INDENT}│ "), border_style)];
        for (i, cell) in row.iter().take(num_cols).enumerate() {
            let mut style = if is_header {
                Style::default()
                    .fg(colors.c_text_primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.c_text_primary())
            };
            if let Some(bg) = row_bg {
                style = style.bg(bg);
            }

            let align = alignments.get(i).copied().unwrap_or(Alignment::None);
            let display = pad_cell_aligned(cell, col_widths[i], align);
            spans.push(Span::styled(display, style));
            if i < num_cols - 1 {
                spans.push(Span::styled(" │ ", border_style));
            }
        }
        // Pad missing trailing cells (jagged rows) so the right border lines up.
        for i in row.len()..num_cols {
            let blank = " ".repeat(col_widths[i]);
            let mut style = Style::default();
            if let Some(bg) = row_bg {
                style = style.bg(bg);
            }
            spans.push(Span::styled(blank, style));
            if i < num_cols - 1 {
                spans.push(Span::styled(" │ ", border_style));
            }
        }
        spans.push(Span::styled(" │", border_style));
        lines.push(Line::from(spans));

        // Header/body separator after row 0.
        if row_idx == 0 {
            let mut sep_spans = vec![Span::styled(format!("{INDENT}├─"), border_style)];
            for (i, w) in col_widths.iter().enumerate() {
                sep_spans.push(Span::styled("─".repeat(*w), border_style));
                if i < num_cols - 1 {
                    sep_spans.push(Span::styled("─┼─", border_style));
                }
            }
            sep_spans.push(Span::styled("─┤", border_style));
            lines.push(Line::from(sep_spans));
        }
    }

    // ── Bottom border:  ╰─────┴─────╯ ───────────────────────────────────────
    let mut bot_spans = vec![Span::styled(format!("{INDENT}╰─"), border_style)];
    for (i, w) in col_widths.iter().enumerate() {
        bot_spans.push(Span::styled("─".repeat(*w), border_style));
        if i < num_cols - 1 {
            bot_spans.push(Span::styled("─┴─", border_style));
        }
    }
    bot_spans.push(Span::styled("─╯", border_style));
    lines.push(Line::from(bot_spans));

    lines
}

/// Truncate a cell to fit within `width` Unicode display columns and pad
/// to that width using the given alignment.  Truncated cells get a trailing
/// `…` (single column) in place of the dropped tail.
fn pad_cell_aligned(cell: &str, width: usize, align: Alignment) -> String {
    let cell_w = UnicodeWidthStr::width(cell);
    let display = if cell_w > width {
        // Reserve one column for the ellipsis.
        let target = width.saturating_sub(1);
        let mut out = String::new();
        let mut acc = 0usize;
        for ch in cell.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if acc + cw > target {
                break;
            }
            out.push(ch);
            acc += cw;
        }
        out.push('…');
        out
    } else {
        cell.to_string()
    };

    let display_w = UnicodeWidthStr::width(display.as_str());
    let padding = width.saturating_sub(display_w);

    match align {
        Alignment::Right => format!("{}{}", " ".repeat(padding), display),
        Alignment::Center => {
            let left = padding / 2;
            let right = padding - left;
            format!("{}{}{}", " ".repeat(left), display, " ".repeat(right))
        }
        // Default + explicit Left both render left-aligned.
        Alignment::Left | Alignment::None => {
            format!("{}{}", display, " ".repeat(padding))
        }
    }
}

/// Hard-wrap (column-based) the styled content spans of a single code-block
/// line at `body_width` Unicode display columns.  Whitespace is preserved
/// (code formatting matters), so wraps happen at the exact column boundary
/// rather than at word boundaries.
///
/// `prefix_spans` is prepended to the FIRST output line; `cont_prefix_spans`
/// is prepended to every subsequent (wrapped) line.  Both should typically
/// carry the dim border style so the indent visually matches the code-block
/// frame.
///
/// Spans within the body are split mid-content as needed; each fragment
/// inherits the original span's style so syntax highlighting is preserved
/// across wrap boundaries.
///
/// `body_width = 0` disables wrapping (single line returned).
#[allow(dead_code)]
fn wrap_code_line_spans(
    prefix_spans: Vec<Span<'static>>,
    body_spans: Vec<Span<'static>>,
    cont_prefix_spans: Vec<Span<'static>>,
    body_width: usize,
) -> Vec<Line<'static>> {
    if body_width == 0 {
        let mut all = prefix_spans;
        all.extend(body_spans);
        return vec![Line::from(all)];
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = prefix_spans;
    let mut current_w: usize = 0;
    let mut on_first_line = true;

    for span in body_spans {
        let style = span.style;
        let mut text = span.content.into_owned();

        while !text.is_empty() {
            // Walk chars until we either consume the whole span fragment or
            // hit the body_width budget for the current line.
            let mut take_chars = 0usize;
            let mut take_w = 0usize;
            let mut iter = text.char_indices();
            let mut last_idx = 0usize;
            let mut consumed_any = false;

            for (i, ch) in iter.by_ref() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if current_w + take_w + cw > body_width {
                    break;
                }
                take_w += cw;
                take_chars += 1;
                last_idx = i + ch.len_utf8();
                consumed_any = true;
            }

            if consumed_any {
                let chunk: String = text.drain(..last_idx).collect();
                if !chunk.is_empty() {
                    current.push(Span::styled(chunk, style));
                    current_w += take_w;
                }
                let _ = take_chars;
            }

            if !text.is_empty() {
                // Buffer is full — push current line, start a continuation.
                lines.push(Line::from(std::mem::take(&mut current)));
                on_first_line = false;
                current = cont_prefix_spans.clone();
                current_w = 0;
                // If body_width is 0 (degenerate) bail to avoid infinite loop.
                if body_width == 0 {
                    break;
                }
            }
        }
    }

    if !current.is_empty() {
        lines.push(Line::from(current));
    } else if on_first_line {
        // Empty code line — preserve a blank line with prefix only.
        lines.push(Line::from(cont_prefix_spans));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gfm_callouts() {
        let colors = ThemeColors::default();
        let md = "> [!NOTE] This is a critical informational note.\n> Second line of note.";
        let lines = parse_markdown_lines_with_theme(md, &colors, 80, true);

        // Find the line containing the callout badge
        let badge_line = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("[ℹ Note]")));
        assert!(badge_line.is_some(), "should render [ℹ Note] callout badge");

        let warn_md = "> [!WARNING] Dangerous operationahead!";
        let warn_lines = parse_markdown_lines_with_theme(warn_md, &colors, 80, true);
        let warn_badge = warn_lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("[⚠ Warning]")));
        assert!(
            warn_badge.is_some(),
            "should render [⚠ Warning] callout badge"
        );
    }

    #[test]
    fn test_gfm_task_lists() {
        let colors = ThemeColors::default();
        let md = "- [ ] Pending task step\n- [x] Completed task step";
        let lines = parse_markdown_lines_with_theme(md, &colors, 80, true);

        let unchecked = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("☐ ")));
        assert!(unchecked.is_some(), "should render unchecked task box ☐");

        let checked = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("☑ ")));
        assert!(checked.is_some(), "should render checked task box ☑");
    }

    #[test]
    fn test_code_block_line_count_header() {
        let colors = ThemeColors::default();
        let md = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
        let lines = parse_markdown_lines_with_theme(md, &colors, 60, true);

        let top_border = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("╭─ [")));
        assert!(
            top_border.is_some(),
            "top border should start with rounded ╭─ ["
        );

        let has_line_count = top_border
            .unwrap()
            .spans
            .iter()
            .any(|s| s.content.contains("3 lines"));
        assert!(has_line_count, "top border should contain [3 lines]");
    }

    #[test]
    fn test_code_block_collapsed_discovery_pill() {
        let colors = ThemeColors::default();
        let mut md = String::from("```python\n");
        for i in 0..25 {
            md.push_str(&format!("print({i})\n"));
        }
        md.push_str("```");

        let lines = parse_markdown_lines_with_theme(&md, &colors, 60, false);
        let pill_line = lines.iter().find(|l| {
            l.spans
                .iter()
                .any(|s| s.content.contains("more lines · Press Ctrl+O to expand"))
        });
        assert!(
            pill_line.is_some(),
            "bottom border should show collapsed discovery pill"
        );
    }

    #[test]
    fn test_diff_code_block_full_line_background() {
        let colors = ThemeColors::default();
        let md = "```diff\n+added line\n-removed line\n@@ -1,3 +1,3 @@\n```";
        let lines = parse_markdown_lines_with_theme(md, &colors, 60, true);

        // Verify that added line has diff_added_bg
        let added_line = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.starts_with("+added line")));
        assert!(added_line.is_some(), "diff added line should be rendered");
        let added_span = added_line
            .unwrap()
            .spans
            .iter()
            .find(|s| s.content.starts_with("+added line"))
            .unwrap();
        assert_eq!(added_span.style.bg, Some(colors.c_bg_surface1()));

        // Verify that removed line has diff_removed_bg
        let removed_line = lines.iter().find(|l| {
            l.spans
                .iter()
                .any(|s| s.content.starts_with("-removed line"))
        });
        assert!(
            removed_line.is_some(),
            "diff removed line should be rendered"
        );
        let removed_span = removed_line
            .unwrap()
            .spans
            .iter()
            .find(|s| s.content.starts_with("-removed line"))
            .unwrap();
        assert_eq!(removed_span.style.bg, Some(colors.c_bg_surface1()));
    }

    #[test]
    fn test_rounded_tables_with_zebra_striping() {
        let colors = ThemeColors::default();
        let md =
            "| Metric | Value |\n|---|---|\n| Latency | 4ms |\n| QPS | 5000 |\n| Memory | 64MB |";
        let lines = parse_markdown_lines_with_theme(md, &colors, 60, true);

        // Verify rounded top border
        let top = lines.first().expect("table should produce lines");
        let top_str = top
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();
        assert!(
            top_str.contains("╭─"),
            "table top border should be rounded with ╭─"
        );
        assert!(
            top_str.contains("─╮"),
            "table top border should end with ─╮"
        );

        // Verify rounded bottom border
        let bot = lines.last().expect("table should produce lines");
        let bot_str = bot
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>();
        assert!(
            bot_str.contains("╰─"),
            "table bottom border should be rounded with ╰─"
        );
        assert!(
            bot_str.contains("─╯"),
            "table bottom border should end with ─╯"
        );
    }
}
