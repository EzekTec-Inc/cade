use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
};
use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SyntectStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

fn syntect_to_tui_style(style: SyntectStyle) -> Style {
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
fn code_border_style() -> Style {
    Style::default().fg(RC::Rgb(60, 60, 60))
}

pub fn parse_markdown_lines(text: &str) -> Vec<Line<'static>> {
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
    let mut highlighter: Option<HighlightLines<'static>> = None;

    let mut list_depth: usize = 0;
    let mut list_counters: Vec<Option<u64>> = Vec::new();

    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_cell = String::new();
    let mut in_table = false;

    // Track whether we just closed a block element so we can insert spacing.
    let mut last_was_block_end = false;

    let push_line =
        |lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>, blockquote: bool| {
            if !spans.is_empty() {
                let mut prefix_spans = Vec::new();
                if blockquote {
                    prefix_spans.push(Span::styled(
                        format!("{INDENT}▎ "),
                        Style::default().fg(RC::Rgb(80, 140, 200)),
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
                }
                Tag::Heading { level, .. } => {
                    // Blank line before every heading for visual breathing room.
                    if !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    last_was_block_end = false;

                    let style = match level {
                        HeadingLevel::H1 => Style::default()
                            .fg(RC::Rgb(100, 220, 255))
                            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                        HeadingLevel::H2 => Style::default()
                            .fg(RC::Rgb(100, 210, 240))
                            .add_modifier(Modifier::BOLD),
                        HeadingLevel::H3 => Style::default()
                            .fg(RC::Rgb(120, 200, 230))
                            .add_modifier(Modifier::BOLD),
                        _ => Style::default().fg(RC::Cyan),
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
                    
                    let syntax = SYNTAX_SET
                        .find_syntax_by_token(&current_lang)
                        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
                    let theme = &THEME_SET.themes["base16-ocean.dark"];
                    highlighter = Some(HighlightLines::new(syntax, theme));

                    // Top border with optional language label
                    let label = if current_lang.is_empty() {
                        format!("{INDENT}┌─────────────────────────────────")
                    } else {
                        format!("{INDENT}┌── {} ──────────────────────────", current_lang)
                    };
                    lines.push(Line::from(Span::styled(label, code_border_style())));
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
                                    .fg(RC::Rgb(180, 180, 255))
                                    .add_modifier(Modifier::BOLD),
                            ));
                            *count += 1;
                        } else {
                            current_spans.push(Span::raw(format!("{INDENT}  {indent_padding}")));
                            current_spans.push(Span::styled(
                                "• ",
                                Style::default().fg(RC::Rgb(100, 207, 180)),
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
                        .fg(RC::Rgb(255, 255, 255))
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
                Tag::Table(_) => {
                    if last_was_block_end && !lines.is_empty() {
                        lines.push(Line::from(""));
                    }
                    last_was_block_end = false;
                    in_table = true;
                    table_rows.clear();
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
                        .fg(RC::Rgb(100, 160, 255))
                        .add_modifier(Modifier::UNDERLINED);
                    style_stack.push(s);
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
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
                    // Bottom border
                    lines.push(Line::from(Span::styled(
                        format!("{INDENT}└─────────────────────────────────"),
                        code_border_style(),
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
                    lines.extend(render_table_data(&table_rows));
                    table_rows.clear();
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
                _ => {}
            },
            Event::Text(text) => {
                if in_table {
                    current_cell.push_str(&text);
                } else if in_code_block {
                    if let Some(ref mut h) = highlighter {
                        let line_iter = LinesWithEndings::from(&text);
                        let mut first = true;
                        for raw_line in line_iter {
                            if !first {
                                push_line(&mut lines, &mut current_spans, in_blockquote);
                            }
                            first = false;

                            let mut spans = vec![Span::styled(
                                format!("{INDENT}{CODE_INDENT}"),
                                code_border_style(),
                            )];

                            let highlighted = h.highlight_line(raw_line, &SYNTAX_SET).unwrap_or_default();
                            for (style, content) in highlighted {
                                let clean_content = content.trim_end_matches('\n').trim_end_matches('\r');
                                if !clean_content.is_empty() {
                                    spans.push(Span::styled(
                                        clean_content.to_string(),
                                        syntect_to_tui_style(style),
                                    ));
                                }
                            }

                            current_spans.extend(spans);
                        }
                        if text.ends_with('\n') {
                            push_line(&mut lines, &mut current_spans, in_blockquote);
                        }
                    }
                } else {
                    let style = style_stack.last().copied().unwrap_or_default();
                    current_spans.push(Span::styled(text.into_string(), style));
                }
            }
            Event::Code(text) => {
                if in_table {
                    current_cell.push_str(&format!("`{text}`"));
                } else {
                    // Inline code: bright on a subtle background via reversed dim
                    let style = Style::default()
                        .fg(RC::Rgb(230, 180, 80))
                        .bg(RC::Rgb(40, 36, 30));
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
                lines.push(Line::from(Span::styled(
                    format!("{INDENT}{}", "─".repeat(40)),
                    Style::default().fg(RC::Rgb(60, 60, 60)),
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

fn render_table_data(data: &[Vec<String>]) -> Vec<Line<'static>> {
    if data.is_empty() {
        return vec![];
    }
    let num_cols = data.iter().map(|row| row.len()).max().unwrap_or(0);
    if num_cols == 0 {
        return vec![];
    }

    let mut col_widths = vec![0; num_cols];
    for row in data {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                col_widths[i] = col_widths[i].max(cell.len());
            }
        }
    }

    let border_style = Style::default().fg(RC::Rgb(60, 60, 60));
    let mut lines = Vec::new();

    for (row_idx, row) in data.iter().enumerate() {
        let mut spans = vec![Span::styled(format!("{INDENT}│ "), border_style)];
        for (i, cell) in row.iter().take(num_cols).enumerate() {
            let style = if row_idx == 0 {
                Style::default()
                    .fg(RC::Rgb(100, 210, 240))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            spans.push(Span::styled(
                format!("{:<width$}", cell, width = col_widths[i]),
                style,
            ));
            if i < num_cols - 1 {
                spans.push(Span::styled(" │ ", border_style));
            }
        }
        spans.push(Span::styled(" │", border_style));
        lines.push(Line::from(spans));

        // Separator line after the header row
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
    lines
}

