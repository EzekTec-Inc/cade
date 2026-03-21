use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
};

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
                prefix_spans.extend(spans.drain(..));
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
                    }
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
                    for (i, line) in text.lines().enumerate() {
                        if i > 0 {
                            push_line(&mut lines, &mut current_spans, in_blockquote);
                        }
                        let mut spans = vec![Span::styled(
                            format!("{INDENT}{CODE_INDENT}"),
                            code_border_style(),
                        )];
                        spans.extend(highlight_code(line, &current_lang));
                        current_spans.extend(spans);
                    }
                    if text.ends_with('\n') {
                        push_line(&mut lines, &mut current_spans, in_blockquote);
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

fn highlight_code(line: &str, lang: &str) -> Vec<Span<'static>> {
    let style_keyword = Style::default()
        .fg(RC::Rgb(130, 170, 255))
        .add_modifier(Modifier::BOLD);
    let style_comment = Style::default()
        .fg(RC::Rgb(90, 90, 90))
        .add_modifier(Modifier::ITALIC);
    let style_string = Style::default().fg(RC::Rgb(140, 220, 140));
    let style_type = Style::default().fg(RC::Rgb(120, 220, 220));
    let style_number = Style::default().fg(RC::Rgb(220, 170, 120));
    let style_default = Style::default().fg(RC::Rgb(200, 200, 200));

    let keywords = match lang {
        "rust" | "rs" => vec![
            "fn", "let", "mut", "pub", "use", "mod", "crate", "impl", "trait", "struct", "enum",
            "match", "if", "else", "for", "while", "loop", "return", "await", "async", "type",
            "as", "where", "self", "Self", "super", "const", "static", "ref", "move", "break",
            "continue", "unsafe", "extern", "dyn", "in",
        ],
        "python" | "py" => vec![
            "def", "class", "import", "from", "as", "if", "elif", "else", "for", "while", "try",
            "except", "finally", "with", "return", "yield", "async", "await", "lambda", "None",
            "True", "False", "raise", "pass", "del", "in", "not", "and", "or", "is", "global",
            "nonlocal",
        ],
        "javascript" | "js" | "typescript" | "ts" | "jsx" | "tsx" => vec![
            "function",
            "const",
            "let",
            "var",
            "import",
            "export",
            "from",
            "class",
            "if",
            "else",
            "for",
            "while",
            "try",
            "catch",
            "finally",
            "return",
            "await",
            "async",
            "type",
            "interface",
            "extends",
            "new",
            "this",
            "super",
            "switch",
            "case",
            "default",
            "break",
            "continue",
            "throw",
            "typeof",
            "instanceof",
            "void",
            "delete",
            "in",
            "of",
            "yield",
            "enum",
            "implements",
            "static",
        ],
        "bash" | "sh" | "zsh" | "shell" => vec![
            "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac",
            "function", "return", "exit", "export", "local", "readonly", "declare", "unset",
            "echo", "printf", "cd", "ls", "grep", "sed", "awk", "cat", "rm", "cp", "mv", "mkdir",
        ],
        "json" => vec![],
        "toml" => vec!["true", "false"],
        "yaml" | "yml" => vec!["true", "false", "null", "yes", "no"],
        _ => vec![],
    };

    let types = match lang {
        "rust" | "rs" => vec![
            "String", "Vec", "Option", "Result", "Box", "Arc", "Mutex", "HashMap", "HashSet", "i8",
            "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
            "f32", "f64", "bool", "char", "str",
        ],
        "typescript" | "ts" | "tsx" => vec![
            "string", "number", "boolean", "any", "void", "never", "unknown", "object", "Array",
            "Promise", "Record", "Partial", "Required", "Readonly",
        ],
        _ => vec![],
    };

    // Full-line comment detection
    let trimmed = line.trim();
    if trimmed.starts_with("//")
        || trimmed.starts_with('#')
            && !lang.is_empty()
            && !matches!(
                lang,
                "python" | "py" | "bash" | "sh" | "zsh" | "shell" | "yaml" | "yml" | "toml"
            )
    {
        return vec![Span::styled(line.to_string(), style_comment)];
    }
    // Python/bash/shell comments
    if matches!(
        lang,
        "python" | "py" | "bash" | "sh" | "zsh" | "shell" | "yaml" | "yml" | "toml"
    ) && trimmed.starts_with('#')
    {
        return vec![Span::styled(line.to_string(), style_comment)];
    }
    // C-style line comments
    if trimmed.starts_with("//") {
        return vec![Span::styled(line.to_string(), style_comment)];
    }

    let mut spans = Vec::new();
    let mut words = Vec::new();
    let mut current_word = String::new();

    for c in line.chars() {
        if c.is_alphanumeric() || c == '_' {
            current_word.push(c);
        } else {
            if !current_word.is_empty() {
                words.push(current_word.clone());
                current_word.clear();
            }
            words.push(c.to_string());
        }
    }
    if !current_word.is_empty() {
        words.push(current_word);
    }

    let mut in_string = false;
    let mut string_char = ' ';

    for word in words {
        if !in_string && (word == "\"" || word == "'" || word == "`") {
            in_string = true;
            // SAFETY: word is guaranteed non-empty by the condition above
            string_char = word.chars().next().unwrap_or('"');
            spans.push(Span::styled(word, style_string));
            continue;
        }
        if in_string {
            spans.push(Span::styled(word.clone(), style_string));
            if word.len() == 1 && word.starts_with(string_char) {
                in_string = false;
            }
            continue;
        }

        if keywords.contains(&word.as_str()) {
            spans.push(Span::styled(word, style_keyword));
        } else if types.contains(&word.as_str()) {
            spans.push(Span::styled(word, style_type));
        } else if word.chars().all(|c| c.is_ascii_digit() || c == '.')
            && !word.is_empty()
            && word.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            spans.push(Span::styled(word, style_number));
        } else if word.chars().all(|c| !c.is_alphanumeric() && c != '_') {
            spans.push(Span::styled(word, style_default));
        } else {
            spans.push(Span::styled(word, style_default));
        }
    }

    spans
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;

    #[test]
    fn test_markdown_parse_basic() {
        // -- Setup & Fixtures
        let md = "# Header\n\n**Bold** and `code`\n\n- Item 1\n- Item 2";

        // -- Exec
        let lines = parse_markdown_lines(md);

        // -- Check
        assert!(lines.len() >= 4, "got {} lines", lines.len());
    }

    #[test]
    fn test_markdown_table_parsing() {
        // -- Setup & Fixtures
        let md = "| Col 1 | Col 2 |\n|---|---|\n| val 1 | val 2 |";

        // -- Exec
        let lines = parse_markdown_lines(md);

        // -- Check
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_markdown_asymmetric_table_parsing() {
        // -- Setup & Fixtures
        let md = "| Col 1 | Col 2 |\n|---|---|\n| val 1 | val 2 | val 3 |";

        // -- Exec
        let lines = parse_markdown_lines(md);

        // -- Check
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_markdown_code_block_has_borders() -> Result<()> {
        // -- Setup & Fixtures
        let md = "```rust\nlet x = 1;\n```";

        // -- Exec
        let lines = parse_markdown_lines(md);
        let text: Vec<String> = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();

        // -- Check
        assert!(
            text[0].contains("┌"),
            "first line should have top border: {:?}",
            text[0]
        );
        let last = text.last().ok_or("Should have at least one line")?;
        assert!(
            last.contains("└"),
            "last line should have bottom border"
        );

        Ok(())
    }

    #[test]
    fn test_markdown_paragraph_spacing() {
        // -- Setup & Fixtures
        let md = "First paragraph.\n\nSecond paragraph.";

        // -- Exec
        let lines = parse_markdown_lines(md);

        // -- Check
        assert!(
            lines.len() >= 3,
            "got {} lines: {:?}",
            lines.len(),
            lines
                .iter()
                .map(|l| l
                    .spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>())
                .collect::<Vec<_>>()
        );
    }
}

// endregion: --- Tests
