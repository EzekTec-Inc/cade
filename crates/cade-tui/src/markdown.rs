use crate::colors::{ThemeColorsExt, ColorDefExt};
use crate::colors::ThemeColors;
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
use syntect::util::LinesWithEndings;

#[cfg(feature = "syntax-highlighting")]
pub(crate) static SYNTAX_SET: LazyLock<SyntaxSet> =
    LazyLock::new(SyntaxSet::load_defaults_newlines);

#[cfg(feature = "syntax-highlighting")]
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
/// Keeps text visually inset from the viewport edge and from tool-call/tool-result
/// gutters, creating a clear content hierarchy.
const INDENT: &str = "  ";

/// Extra indent inside code blocks (on top of INDENT).
const CODE_INDENT: &str = "    ";

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

        for word in content.split_inclusive(|c: char| c == ' ') {
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

pub fn parse_markdown_lines(text: &str) -> Vec<Line<'static>> {
    parse_markdown_lines_with_theme(text, &ThemeColors::dark(), 0)
}

/// Parse markdown text into styled `Line`s.
///
/// `max_width` is the available viewport width in columns.  When `> 0`
/// it is used to cap table column widths and truncate long code-block
/// lines so that rendered content stays within the viewport.  Pass `0`
/// to disable width-capping (legacy callers).
pub fn parse_markdown_lines_with_theme(text: &str, colors: &ThemeColors, max_width: usize) -> Vec<Line<'static>> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);

    let parser = Parser::new_ext(text, options);

    let mut lines = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();

    let mut style_stack = vec![Style::default()];

    let mut in_blockquote = false;
    let mut in_code_block = false;
    let mut current_lang = String::new();
    #[cfg(feature = "syntax-highlighting")]
    let dyn_theme = crate::colors::generate_syntect_theme(colors);
    #[cfg(feature = "syntax-highlighting")]
    let mut highlighter: Option<HighlightLines<'_>> = None;
    #[cfg(not(feature = "syntax-highlighting"))]
    let mut highlighter: Option<()> = None;

    let mut list_depth: usize = 0;
    let mut list_counters: Vec<Option<u64>> = Vec::new();

    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut table_alignments: Vec<Alignment> = Vec::new();
    let mut current_cell = String::new();
    let mut in_table = false;

    // V7: stack of image dest URLs for nested image tags (rare but legal).
    let mut image_url_stack: Vec<String> = Vec::new();

    // Track whether we just closed a block element so we can insert spacing.
    let mut last_was_block_end = false;

    let push_line =
        |lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>, blockquote: bool| {
            if !spans.is_empty() {
                let mut prefix_spans = Vec::new();
                if blockquote {
                    prefix_spans.push(Span::styled(
                        format!("{INDENT}▎ "),
                        colors.md_quote_border(),
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
                            .fg(colors.md_heading.to_ratatui())
                            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                        HeadingLevel::H2 => Style::default()
                            .fg(colors.md_heading.to_ratatui())
                            .add_modifier(Modifier::BOLD),
                        HeadingLevel::H3 => Style::default()
                            .fg(colors.md_heading.to_ratatui())
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
                }
                Tag::CodeBlock(kind) => {
                    if last_was_block_end && !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    last_was_block_end = false;
                    in_code_block = true;

                    if let CodeBlockKind::Fenced(lang) = kind {
                        current_lang = lang.to_string();
                    } else {
                        current_lang.clear();
                    }

                    #[cfg(feature = "syntax-highlighting")]
                    {
                        let syntax = SYNTAX_SET
                            .find_syntax_by_token(&current_lang)
                            .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());

                        highlighter = Some(HighlightLines::new(syntax, &dyn_theme));
                    }

                    // Top border with optional language label, sized to viewport
                    let border_w = if max_width > 2 {
                        max_width.saturating_sub(INDENT.len()).max(8)
                    } else {
                        33
                    };
                    let label = if current_lang.is_empty() {
                        let dashes = "─".repeat(border_w.saturating_sub(1));
                        format!("{INDENT}┌{dashes}")
                    } else {
                        let prefix = format!("┌── {} ", current_lang);
                        let prefix_w = UnicodeWidthStr::width(prefix.as_str());
                        let dashes = "─".repeat(border_w.saturating_sub(prefix_w));
                        format!("{INDENT}{prefix}{dashes}")
                    };
                    lines.push(Line::from(Span::styled(label, code_border_style(colors))));
                }
                Tag::List(start) => {
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
                                    .fg(colors.md_link.to_ratatui())
                                    .add_modifier(Modifier::BOLD),
                            ));
                            *count += 1;
                        } else {
                            current_spans.push(Span::raw(format!("{INDENT}  {indent_padding}")));
                            current_spans.push(Span::styled(
                                "• ",
                                colors.md_list_bullet(),
                            ));
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
                        .fg(colors.text_primary.to_ratatui())
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
                        .fg(colors.md_link.to_ratatui())
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
                        .fg(colors.md_link.to_ratatui());
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
                    // Pre-wrap paragraph spans at viewport width so continuation
                    // lines get a proper INDENT prefix (or blockquote bar) — ratatui's
                    // `Wrap { trim: false }` would otherwise leave wrapped lines
                    // hanging flush-left.
                    if max_width > 0 && !current_spans.is_empty() {
                        let cont_prefix = if in_blockquote {
                            Some(Span::styled(
                                format!("{INDENT}▎ "),
                                colors.md_quote_border(),
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
                        push_line(&mut lines, &mut current_spans, in_blockquote);
                    }
                    last_was_block_end = true;
                }
                TagEnd::Heading(_) => {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
                    style_stack.pop();
                    last_was_block_end = true;
                }
                TagEnd::BlockQuote(_) => {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
                    in_blockquote = false;
                    last_was_block_end = true;
                }
                TagEnd::CodeBlock => {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
                    // Bottom border, sized to viewport
                    let border_w = if max_width > 2 {
                        max_width.saturating_sub(INDENT.len()).max(8)
                    } else {
                        33
                    };
                    let dashes = "─".repeat(border_w.saturating_sub(1));
                    lines.push(Line::from(Span::styled(
                        format!("{INDENT}└{dashes}"),
                        code_border_style(colors),
                    )));
                    in_code_block = false;
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
                    push_line(&mut lines, &mut current_spans, in_blockquote);
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
                    // V7: close the bracketed alt text, append "(url)" if
                    // we captured one.  Style stack pop matches Tag::Image.
                    let url = image_url_stack.pop().unwrap_or_default();
                    let img_style = style_stack
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .fg(colors.md_link.to_ratatui());
                    if !url.is_empty() {
                        current_spans.push(Span::styled(
                            format!("] ({url})"),
                            img_style,
                        ));
                    } else {
                        current_spans.push(Span::styled("]".to_string(), img_style));
                    }
                    style_stack.pop();
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_table {
                    current_cell.push_str(&text);
                } else if in_code_block {
                    // Body width inside the code-block frame:
                    //   max_width − INDENT(2) − CODE_INDENT(4)  = max_width − 6
                    // Falls back to 0 (no wrap) when max_width is unknown.
                    let body_width = if max_width > 6 { max_width - 6 } else { 0 };
                    let prefix_span = Span::styled(
                        format!("{INDENT}{CODE_INDENT}"),
                        code_border_style(colors),
                    );
                    let cont_prefix_span = Span::styled(
                        format!("{INDENT}{CODE_INDENT}"),
                        code_border_style(colors),
                    );

                    #[cfg(feature = "syntax-highlighting")]
                    if let Some(ref mut h) = highlighter {
                        let line_iter = LinesWithEndings::from(&text);
                        let mut first = true;
                        for raw_line in line_iter {
                            if !first {
                                push_line(&mut lines, &mut current_spans, in_blockquote);
                            }
                            first = false;

                            // Build body spans (highlighted tokens) with no prefix.
                            let mut body: Vec<Span<'static>> = Vec::new();
                            let highlighted =
                                h.highlight_line(raw_line, &SYNTAX_SET).unwrap_or_default();
                            for (style, content) in highlighted {
                                let clean_content =
                                    content.trim_end_matches('\n').trim_end_matches('\r');
                                if !clean_content.is_empty() {
                                    body.push(Span::styled(
                                        clean_content.to_string(),
                                        syntect_to_tui_style(style),
                                    ));
                                }
                            }

                            // Hard-wrap at viewport so long code lines stay
                            // inside the code-block frame.
                            let wrapped = wrap_code_line_spans(
                                vec![prefix_span.clone()],
                                body,
                                vec![cont_prefix_span.clone()],
                                body_width,
                            );
                            for l in wrapped {
                                current_spans.extend(l.spans);
                                push_line(&mut lines, &mut current_spans, in_blockquote);
                            }
                        }
                        // The trailing newline behaviour is preserved by the
                        // line_iter loop above; no extra push needed.
                    }
                    #[cfg(not(feature = "syntax-highlighting"))]
                    {
                        // Plain rendering without syntax highlighting.
                        let _ = &highlighter; // suppress unused warning
                        for raw_line in text.lines() {
                            let body = vec![Span::styled(
                                raw_line.to_string(),
                                colors.text_primary(),
                            )];
                            let wrapped = wrap_code_line_spans(
                                vec![prefix_span.clone()],
                                body,
                                vec![cont_prefix_span.clone()],
                                body_width,
                            );
                            for l in wrapped {
                                current_spans.extend(l.spans);
                                push_line(&mut lines, &mut current_spans, in_blockquote);
                            }
                        }
                    }
                } else {
                    let style = style_stack.last().copied().unwrap_or_default();
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
                    let style = colors.md_code().bg(colors.bg_surface1.to_ratatui());
                    current_spans.push(Span::styled(format!(" {text} "), style));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_table {
                    current_cell.push(' ');
                } else if in_code_block {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
                } else {
                    // SoftBreak is a line break within a paragraph — treat as space.
                    // HardBreak (two trailing spaces or backslash) forces a new line.
                    if matches!(event, Event::HardBreak) {
                        push_line(&mut lines, &mut current_spans, in_blockquote);
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

    push_line(&mut lines, &mut current_spans, false);

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

    // ── Top border:  ┌─────┬─────┐ ────────────────────────────────────────
    let mut top_spans = vec![Span::styled(format!("{INDENT}┌─"), border_style)];
    for (i, w) in col_widths.iter().enumerate() {
        top_spans.push(Span::styled("─".repeat(*w), border_style));
        if i < num_cols - 1 {
            top_spans.push(Span::styled("─┬─", border_style));
        }
    }
    top_spans.push(Span::styled("─┐", border_style));
    lines.push(Line::from(top_spans));

    // ── Data rows + header separator ────────────────────────────────────────
    for (row_idx, row) in data.iter().enumerate() {
        let mut spans = vec![Span::styled(format!("{INDENT}│ "), border_style)];
        for (i, cell) in row.iter().take(num_cols).enumerate() {
            let style = if row_idx == 0 {
                Style::default()
                    .fg(colors.md_heading.to_ratatui())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
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
            spans.push(Span::raw(blank));
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

    // ── Bottom border:  └─────┴─────┘ ───────────────────────────────────────
    let mut bot_spans = vec![Span::styled(format!("{INDENT}└─"), border_style)];
    for (i, w) in col_widths.iter().enumerate() {
        bot_spans.push(Span::styled("─".repeat(*w), border_style));
        if i < num_cols - 1 {
            bot_spans.push(Span::styled("─┴─", border_style));
        }
    }
    bot_spans.push(Span::styled("─┘", border_style));
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

            while let Some((i, ch)) = iter.next() {
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
    use crate::colors::ThemeColors;

    fn dark() -> ThemeColors {
        ThemeColors::dark()
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn lines_text(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(line_text).collect()
    }

    // ── Unicode-width table rendering ─────────────────────────────────────

    #[test]
    fn table_uses_unicode_display_width_for_emoji() {
        // Each emoji is 2 cols wide; "🚀🚀" measures 4 cols, not 8 (bytes).
        let data = vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["🚀🚀".to_string(), "ok".to_string()],
        ];
        let lines = render_table_data(&data, &[], &dark(), 80);
        // Layout: INDENT + ┌─ + (─*col0_w) + ─┬─ + (─*col1_w) + ─┐
        // col0_w = 4 (emoji width), col1_w = 2 ("ok") ⇒ 6 dashes, then 4.
        let top = line_text(&lines[0]);
        assert!(
            top.contains("┌──────┬────┐"),
            "expected col0=4 col1=2 layout, got: {top}"
        );
    }

    #[test]
    fn table_truncates_with_ellipsis_at_unicode_boundary() {
        let data = vec![
            vec!["Header".to_string()],
            vec!["a-very-long-value-here".to_string()],
        ];
        let lines = render_table_data(&data, &[], &dark(), 20);
        let body = line_text(&lines[3]);
        assert!(body.contains('…'), "expected ellipsis, got: {body}");
        assert!(!body.contains("very-long-value-here"));
    }

    #[test]
    fn table_renders_top_and_bottom_borders() {
        let data = vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["1".to_string(), "2".to_string()],
        ];
        let lines = render_table_data(&data, &[], &dark(), 80);
        assert_eq!(lines.len(), 5);
        let top = line_text(&lines[0]);
        let bot = line_text(&lines[4]);
        assert!(top.starts_with("  ┌"), "expected top border, got: {top}");
        assert!(top.contains('┬'), "expected ┬ in top border");
        assert!(top.ends_with('┐'), "expected ┐ corner");
        assert!(bot.starts_with("  └"), "expected bottom border, got: {bot}");
        assert!(bot.contains('┴'), "expected ┴ in bottom border");
        assert!(bot.ends_with('┘'), "expected ┘ corner");
    }

    #[test]
    fn table_respects_right_alignment() {
        let data = vec![
            vec!["Hdr".to_string()],
            vec!["x".to_string()],
        ];
        let lines = render_table_data(&data, &[Alignment::Right], &dark(), 80);
        let body = line_text(&lines[3]);
        assert!(body.contains("  x "), "expected right-aligned x, got: {body:?}");
    }

    #[test]
    fn table_respects_center_alignment() {
        let data = vec![
            vec!["Hdr".to_string()],
            vec!["x".to_string()],
        ];
        let lines = render_table_data(&data, &[Alignment::Center], &dark(), 80);
        let body = line_text(&lines[3]);
        assert!(body.contains(" x "), "expected centered x, got: {body:?}");
    }

    #[test]
    fn table_pads_jagged_rows_to_full_width() {
        let data = vec![
            vec!["A".to_string(), "B".to_string(), "C".to_string()],
            vec!["x".to_string()],
        ];
        let lines = render_table_data(&data, &[], &dark(), 80);
        let body = line_text(&lines[3]);
        assert!(body.ends_with(" │"), "jagged row not padded: {body:?}");
    }

    // ── Code block + horizontal rule width ────────────────────────────────

    #[test]
    fn code_block_borders_size_to_viewport() {
        let md = "```\nlet x = 1;\n```";
        let lines = parse_markdown_lines_with_theme(md, &dark(), 50);
        let top = line_text(&lines[0]);
        let dashes = top.chars().filter(|c| *c == '─').count();
        assert!(
            dashes >= 40,
            "code top border too short ({dashes} dashes): {top:?}"
        );
    }

    #[test]
    fn horizontal_rule_sizes_to_viewport() {
        let md = "para\n\n---\n\npara2";
        let lines = parse_markdown_lines_with_theme(md, &dark(), 60);
        let hr_line = lines
            .iter()
            .find(|l| {
                let t = line_text(l);
                t.chars().filter(|c| *c == '─').count() > 20
            })
            .expect("expected an HR line");
        let dashes = line_text(hr_line)
            .chars()
            .filter(|c| *c == '─')
            .count();
        assert!(dashes >= 50, "HR too short ({dashes} dashes)");
    }

    // ── Paragraph indent + word wrap ──────────────────────────────────────

    #[test]
    fn paragraph_text_starts_with_indent() {
        let md = "hello world";
        let lines = parse_markdown_lines_with_theme(md, &dark(), 80);
        let first = line_text(&lines[0]);
        assert!(
            first.starts_with("  "),
            "paragraph not indented: {first:?}"
        );
    }

    #[test]
    fn paragraph_wraps_long_text_with_continuation_indent() {
        let words = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do".to_string();
        let lines = parse_markdown_lines_with_theme(&words, &dark(), 30);
        let para_lines: Vec<_> = lines
            .iter()
            .filter(|l| !line_text(l).trim().is_empty())
            .collect();
        assert!(
            para_lines.len() >= 2,
            "expected wrap into multiple lines, got {}: {:?}",
            para_lines.len(),
            lines_text(&lines)
        );
        for l in &para_lines {
            let t = line_text(l);
            assert!(
                t.starts_with("  "),
                "wrapped line missing indent: {t:?}"
            );
        }
    }

    #[test]
    fn paragraph_wrap_preserves_bold_style_across_break() {
        let md = "intro **VERY-LONG-BOLD-WORD-HERE** outro";
        let lines = parse_markdown_lines_with_theme(md, &dark(), 30);
        let mut found_bold = false;
        for l in &lines {
            for s in &l.spans {
                if s.content.contains("VERY-LONG-BOLD-WORD-HERE") {
                    let has_bold = s.style.add_modifier.contains(Modifier::BOLD);
                    assert!(has_bold, "bold modifier lost on wrap: {:?}", s.style);
                    found_bold = true;
                }
            }
        }
        assert!(found_bold, "bold span never appeared in output");
    }

    #[test]
    fn empty_input_returns_empty_lines() {
        let lines = parse_markdown_lines_with_theme("", &dark(), 80);
        assert!(lines.is_empty(), "expected no lines, got {lines:?}");
    }

    #[test]
    fn pad_cell_aligned_handles_emoji_width() {
        let s = pad_cell_aligned("🚀", 6, Alignment::Left);
        assert_eq!(UnicodeWidthStr::width(s.as_str()), 6, "{s:?}");
        assert!(s.starts_with('🚀'));
    }

    #[test]
    fn pad_cell_aligned_truncates_with_ellipsis() {
        let s = pad_cell_aligned("hello world", 5, Alignment::Left);
        assert_eq!(s, "hell…");
        assert_eq!(UnicodeWidthStr::width(s.as_str()), 5);
    }

    #[test]
    fn pad_cell_aligned_center_with_odd_padding() {
        let s = pad_cell_aligned("ab", 5, Alignment::Center);
        assert_eq!(s, " ab  ");
    }

    // ── V2: code block hard-wrap at viewport edge ─────────────────────────

    #[test]
    fn code_block_long_line_hard_wraps_at_viewport() {
        // Line of 80 chars; viewport 30 cols ⇒ body width 30-6 = 24.
        // Expect at least 2 wrapped lines for the code body (excluding borders).
        let body = "abcdefghij".repeat(8); // 80 chars, no spaces
        let md = format!("```\n{body}\n```");
        let lines = parse_markdown_lines_with_theme(&md, &dark(), 30);
        // Each rendered code line must fit within max_width display columns.
        for l in &lines {
            let t = line_text(l);
            assert!(
                UnicodeWidthStr::width(t.as_str()) <= 30,
                "code line exceeds viewport: {} cols ({:?})",
                UnicodeWidthStr::width(t.as_str()),
                t
            );
        }
        // Should produce > 4 lines (top border + ≥ 2 wrapped body + bottom).
        assert!(lines.len() >= 4, "expected ≥4 lines, got {}", lines.len());
    }

    #[test]
    fn code_block_short_line_not_wrapped() {
        let md = "```\nlet x = 1;\n```";
        let lines = parse_markdown_lines_with_theme(&md, &dark(), 80);
        // Layout: top, body, bottom = 3 lines.
        assert_eq!(lines.len(), 3, "{:?}", lines_text(&lines));
    }

    // ── B6: inline code in tables drops backticks ─────────────────────────

    #[test]
    fn inline_code_in_table_renders_without_backticks() {
        let md = "| Cmd | Desc |\n|---|---|\n| `ls` | list |";
        let lines = parse_markdown_lines_with_theme(&md, &dark(), 80);
        let body_line = lines
            .iter()
            .find(|l| {
                let t = line_text(l);
                t.contains("ls") && t.contains("list")
            })
            .expect("expected body row");
        let t = line_text(body_line);
        assert!(!t.contains('`'), "table cell still has backticks: {t:?}");
    }

    // ── V7: image alt text rendering ──────────────────────────────────────

    #[test]
    fn image_renders_alt_and_url() {
        let md = "![logo](https://example.com/logo.png)";
        let lines = parse_markdown_lines_with_theme(&md, &dark(), 80);
        let combined: String = lines.iter().map(line_text).collect();
        assert!(combined.contains("🖼"), "expected image glyph in: {combined:?}");
        assert!(combined.contains("[logo]"), "expected [alt] in: {combined:?}");
        assert!(
            combined.contains("https://example.com/logo.png"),
            "expected URL in: {combined:?}"
        );
    }

    #[test]
    fn image_with_empty_alt_still_renders() {
        let md = "![](https://example.com/x.png)";
        let lines = parse_markdown_lines_with_theme(&md, &dark(), 80);
        let combined: String = lines.iter().map(line_text).collect();
        assert!(combined.contains("🖼"), "expected image glyph");
        assert!(
            combined.contains("https://example.com/x.png"),
            "expected URL"
        );
    }
}
