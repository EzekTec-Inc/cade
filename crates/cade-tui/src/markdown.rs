use ratatui::{
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
};
use pulldown_cmark::{Parser, Event, Tag, TagEnd, CodeBlockKind, Options, HeadingLevel};

const INDENT: &str = "";

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
    
    let push_line = |lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>, blockquote: bool| {
        if !spans.is_empty() {
            let mut prefix_spans = Vec::new();
            if blockquote {
                prefix_spans.push(Span::styled(format!("{INDENT}▎ "), Style::default().fg(RC::DarkGray)));
            }
            prefix_spans.extend(spans.drain(..));
            lines.push(Line::from(prefix_spans));
        }
    };

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {},
                Tag::Heading { level, .. } => {
                    let style = match level {
                        HeadingLevel::H1 => Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                        HeadingLevel::H2 => Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD),
                        _ => Style::default().fg(RC::Cyan),
                    };
                    style_stack.push(style);
                    
                    let level_num = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };
                    let prefix = "#".repeat(level_num);
                    current_spans.push(Span::styled(format!("{INDENT}{} ", prefix), style));
                }
                Tag::BlockQuote(_) => {
                    in_blockquote = true;
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    if let CodeBlockKind::Fenced(lang) = kind {
                        current_lang = lang.to_string();
                        if !current_lang.is_empty() {
                            lines.push(Line::from(Span::styled(
                                format!("{INDENT}  {}", current_lang),
                                Style::default().fg(RC::DarkGray).add_modifier(Modifier::DIM),
                            )));
                        }
                    }
                }
                Tag::List(start) => {
                    list_depth += 1;
                    list_counters.push(start);
                }
                Tag::Item => {
                    let indent_padding = "  ".repeat(list_depth.saturating_sub(1));
                    if let Some(counters) = list_counters.last_mut() {
                        if let Some(count) = counters {
                            current_spans.push(Span::raw(format!("{INDENT}  {indent_padding}")));
                            current_spans.push(Span::styled(format!("{count}. "), Style::default().add_modifier(Modifier::BOLD)));
                            *count += 1;
                        } else {
                            current_spans.push(Span::raw(format!("{INDENT}  {indent_padding}")));
                            current_spans.push(Span::styled("• ", Style::default().fg(RC::Green)));
                        }
                    }
                }
                Tag::Emphasis => {
                    let s = style_stack.last().copied().unwrap_or_default().add_modifier(Modifier::ITALIC);
                    style_stack.push(s);
                }
                Tag::Strong => {
                    let s = style_stack.last().copied().unwrap_or_default().add_modifier(Modifier::BOLD);
                    style_stack.push(s);
                }
                Tag::Strikethrough => {
                    let s = style_stack.last().copied().unwrap_or_default().add_modifier(Modifier::CROSSED_OUT);
                    style_stack.push(s);
                }
                Tag::Table(_) => {
                    in_table = true;
                    table_rows.clear();
                }
                Tag::TableHead | Tag::TableRow => {
                    table_rows.push(Vec::new());
                }
                Tag::TableCell => {
                    current_cell.clear();
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
                }
                TagEnd::Heading(_) => {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
                    style_stack.pop();
                }
                TagEnd::BlockQuote(_) => {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
                    in_blockquote = false;
                }
                TagEnd::CodeBlock => {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
                    in_code_block = false;
                    current_lang.clear();
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    list_counters.pop();
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
                }
                TagEnd::TableCell => {
                    if let Some(last_row) = table_rows.last_mut() {
                        last_row.push(current_cell.clone());
                    }
                    current_cell.clear();
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
                        let mut spans = vec![Span::raw(format!("{INDENT}  "))];
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
                    let style = style_stack.last().copied().unwrap_or_default().fg(RC::Yellow);
                    current_spans.push(Span::styled(text.into_string(), style));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_table {
                    current_cell.push(' ');
                } else if in_code_block {
                    push_line(&mut lines, &mut current_spans, in_blockquote);
                } else {
                    if let Event::HardBreak = event {
                        push_line(&mut lines, &mut current_spans, in_blockquote);
                    } else {
                        let style = style_stack.last().copied().unwrap_or_default();
                        current_spans.push(Span::styled(" ", style));
                    }
                }
            }
            Event::Rule => {
                lines.push(Line::from(Span::styled(
                    format!("{INDENT}{}", "─".repeat(40)),
                    Style::default().fg(RC::DarkGray),
                )));
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
            col_widths[i] = col_widths[i].max(cell.len());
        }
    }

    let mut lines = Vec::new();
    for (row_idx, row) in data.iter().enumerate() {
        let mut spans = vec![Span::styled(format!("{INDENT}│ "), Style::default().fg(RC::DarkGray))];
        for (i, cell) in row.iter().take(num_cols).enumerate() {
            let style = if row_idx == 0 {
                Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(RC::White)
            };
            spans.push(Span::styled(format!("{:<width$}", cell, width = col_widths[i]), style));
            if i < num_cols - 1 {
                spans.push(Span::styled(" │ ", Style::default().fg(RC::DarkGray)));
            }
        }
        spans.push(Span::styled(" │", Style::default().fg(RC::DarkGray)));
        lines.push(Line::from(spans));
    }
    lines
}

fn highlight_code(line: &str, lang: &str) -> Vec<Span<'static>> {
    let style_keyword = Style::default().fg(RC::LightBlue).add_modifier(Modifier::BOLD);
    let style_comment = Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC);
    let style_string  = Style::default().fg(RC::LightGreen);
    let style_type    = Style::default().fg(RC::LightCyan);

    let keywords = match lang {
        "rust" | "rs" => vec![
            "fn", "let", "mut", "pub", "use", "mod", "crate", "impl", "trait", "struct", "enum",
            "match", "if", "else", "for", "while", "loop", "return", "await", "async", "type", "as",
        ],
        "python" | "py" => vec![
            "def", "class", "import", "from", "as", "if", "elif", "else", "for", "while", "try",
            "except", "finally", "with", "return", "yield", "async", "await", "lambda", "None", "True", "False",
        ],
        "javascript" | "js" | "typescript" | "ts" => vec![
            "function", "const", "let", "var", "import", "export", "from", "class", "if", "else", "for",
            "while", "try", "catch", "finally", "return", "await", "async", "type", "interface", "extends",
        ],
        _ => vec![],
    };

    let types = match lang {
        "rust" | "rs" => vec!["String", "Vec", "Option", "Result", "i32", "u32", "i64", "u64", "f32", "f64", "bool", "usize"],
        _ => vec![],
    };

    if line.trim().starts_with("//") || line.trim().starts_with("#") {
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
        if !in_string && (word == "\"" || word == "'") {
            in_string = true;
            string_char = word.chars().next().unwrap();
            spans.push(Span::styled(word, style_string));
            continue;
        }
        if in_string {
            spans.push(Span::styled(word.clone(), style_string));
            if word == string_char.to_string() {
                in_string = false;
            }
            continue;
        }

        if keywords.contains(&word.as_str()) {
            spans.push(Span::styled(word, style_keyword));
        } else if types.contains(&word.as_str()) {
            spans.push(Span::styled(word, style_type));
        } else if word.chars().all(|c| !c.is_alphanumeric() && c != '_') {
            spans.push(Span::raw(word));
        } else {
            spans.push(Span::raw(word));
        }
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_markdown() {
        let md = "# Header\n**Bold** and `code`\n- Item 1\n- Item 2";
        let lines = parse_markdown_lines(md);
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn test_table_parsing() {
        let md = "| Col 1 | Col 2 |\n|---|---|\n| val 1 | val 2 |";
        let lines = parse_markdown_lines(md);
        // Header line + Data line
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_asymmetric_table_parsing() {
        let md = "| Col 1 | Col 2 |\n|---|---|\n| val 1 | val 2 | val 3 |";
        let lines = parse_markdown_lines(md);
        assert_eq!(lines.len(), 2);
    }
}
