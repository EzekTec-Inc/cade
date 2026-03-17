use ratatui::{
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
};

/// Left-margin indent for markdown paragraphs (matches CADE Code style).
const INDENT: &str = "";

/// Convert a complete markdown text string into a `Vec<Line>` for ratatui rendering.
/// Handles: headings, bullets, numbered lists, code fences, horizontal rules, blockquotes, inline bold/code, and simple tables.
pub fn parse_markdown_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_fence = false;
    let mut current_lang = String::new();
    let mut table_buffer: Vec<String> = Vec::new();

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();

        // ── Code fence toggle ────────────────────────────────────────────
        if trimmed.starts_with("```") {
            // Flush any pending table before entering code fence
            if !table_buffer.is_empty() {
                lines.extend(render_table(&table_buffer));
                table_buffer.clear();
            }

            in_fence = !in_fence;
            if in_fence {
                current_lang = trimmed.trim_start_matches('`').trim().to_lowercase();
                if !current_lang.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("{INDENT}  {current_lang}"),
                        Style::default()
                            .fg(RC::DarkGray)
                            .add_modifier(Modifier::DIM),
                    )));
                }
            } else {
                current_lang.clear();
            }
            continue;
        }

        if in_fence {
            let mut spans = vec![Span::raw(format!("{INDENT}  "))];
            spans.extend(highlight_code(raw_line, &current_lang));
            lines.push(Line::from(spans));
            continue;
        }

        // ── Table Detection ───────────────────────────────────────────────
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            table_buffer.push(trimmed.to_string());
            continue;
        } else if !table_buffer.is_empty() {
            // End of table
            lines.extend(render_table(&table_buffer));
            table_buffer.clear();
        }

        // ── Empty line ────────────────────────────────────────────────────
        if trimmed.is_empty() {
            lines.push(Line::from(""));
            continue;
        }

        let leading_spaces = raw_line.chars().take_while(|c| c.is_whitespace()).count();
        let trimmed_start = raw_line.trim_start();

        // ── Headings ──────────────────────────────────────────────────────
        if let Some(rest) = trimmed_start.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{rest}"),
                Style::default().fg(RC::Cyan),
            )));
            continue;
        }
        if let Some(rest) = trimmed_start.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{rest}"),
                Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = trimmed_start.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{rest}"),
                Style::default()
                    .fg(RC::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }

        // ── Horizontal rule ────────────────────────────────────────────────
        if trimmed == "---" || trimmed == "***" || trimmed == "===" {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{}", "─".repeat(40)),
                Style::default().fg(RC::DarkGray),
            )));
            continue;
        }

        // ── Blockquotes ───────────────────────────────────────────────────
        if let Some(rest) = trimmed_start.strip_prefix("> ") {
            let mut spans: Vec<Span<'static>> = vec![Span::styled(
                format!("{INDENT}▎ "),
                Style::default().fg(RC::DarkGray),
            )];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // ── Bullet list ────────────────────────────────────────────────────
        let bullet_rest = trimmed_start
            .strip_prefix("- ")
            .or_else(|| trimmed_start.strip_prefix("* "))
            .or_else(|| trimmed_start.strip_prefix("• "));
        if let Some(rest) = bullet_rest {
            let indent_padding = " ".repeat(leading_spaces);
            let mut spans: Vec<Span<'static>> = vec![
                Span::raw(format!("{INDENT}  {indent_padding}")),
                Span::styled("• ", Style::default().fg(RC::Green)),
            ];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // ── Numbered list ─────────────────────────────────────────────────
        if let Some((num, rest)) = parse_list_prefix(trimmed_start) {
            let indent_padding = " ".repeat(leading_spaces);
            let mut spans: Vec<Span<'static>> = vec![
                Span::raw(format!("{INDENT}  {indent_padding}")),
                Span::styled(
                    format!("{num}. "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // ── Normal paragraph line with inline spans ───────────────────────
        let mut spans: Vec<Span<'static>> = vec![Span::raw(INDENT)];
        spans.extend(parse_inline(raw_line));
        lines.push(Line::from(spans));
    }

    // Final flush
    if !table_buffer.is_empty() {
        lines.extend(render_table(&table_buffer));
    }

    lines
}

/// Simple table renderer that aligns columns.
fn render_table(rows: &[String]) -> Vec<Line<'static>> {
    if rows.len() < 2 {
        // Not a valid table, just return as plain lines
        return rows.iter().map(|r| Line::from(r.clone())).collect();
    }

    let mut data: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            r.split('|')
                .skip(1) // leading |
                .map(|s| s.trim().to_string())
                .collect::<Vec<String>>()
        })
        .collect();

    // Remove the last empty element if line ended with |
    for row in data.iter_mut() {
        if row.last().map_or(false, |s| s.is_empty()) {
            row.pop();
        }
    }

    if data.is_empty() {
        return vec![];
    }

    let num_cols = data[0].len();
    let mut col_widths = vec![0; num_cols];

    for row in &data {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                col_widths[i] = col_widths[i].max(cell.len());
            }
        }
    }

    let mut lines = Vec::new();
    for (row_idx, row) in data.into_iter().enumerate() {
        // Skip the separator row (e.g. |---|---|) but use it to detect valid tables
        if row_idx == 1 && row.iter().all(|s| s.chars().all(|c| c == '-' || c == ':')) {
            continue;
        }

        let mut spans = vec![Span::styled(
            format!("{INDENT}│ "),
            Style::default().fg(RC::DarkGray),
        )];
        for (i, cell) in row.into_iter().take(num_cols).enumerate() {
            let style = if row_idx == 0 {
                Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(RC::White)
            };
            spans.push(Span::styled(
                format!("{:<width$}", cell, width = col_widths[i]),
                style,
            ));
            if i < num_cols - 1 {
                spans.push(Span::styled(" │ ", Style::default().fg(RC::DarkGray)));
            }
        }
        spans.push(Span::styled(" │", Style::default().fg(RC::DarkGray)));
        lines.push(Line::from(spans));
    }

    lines
}

/// Simple keyword-based syntax highlighter.
fn highlight_code(line: &str, lang: &str) -> Vec<Span<'static>> {
    let style_keyword = Style::default()
        .fg(RC::LightBlue)
        .add_modifier(Modifier::BOLD);
    let style_comment = Style::default()
        .fg(RC::DarkGray)
        .add_modifier(Modifier::ITALIC);
    let style_string = Style::default().fg(RC::LightGreen);
    let style_type = Style::default().fg(RC::LightCyan);

    let keywords = match lang {
        "rust" | "rs" => vec![
            "fn", "let", "mut", "pub", "use", "mod", "crate", "impl", "trait", "struct", "enum",
            "match", "if", "else", "for", "while", "loop", "return", "await", "async", "type",
            "as",
        ],
        "python" | "py" => vec![
            "def", "class", "import", "from", "as", "if", "elif", "else", "for", "while", "try",
            "except", "finally", "with", "return", "yield", "async", "await", "lambda", "None",
            "True", "False",
        ],
        "javascript" | "js" | "typescript" | "ts" => vec![
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
        ],
        _ => vec![],
    };

    let types = match lang {
        "rust" | "rs" => vec![
            "String", "Vec", "Option", "Result", "i32", "u32", "i64", "u64", "f32", "f64", "bool",
            "usize",
        ],
        _ => vec![],
    };

    if line.trim().starts_with("//") || line.trim().starts_with("#") {
        return vec![Span::styled(line.to_string(), style_comment)];
    }

    let mut spans = Vec::new();
    let mut words = Vec::new();
    let mut current_word = String::new();

    // Very naive tokenizer
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

/// Parse inline markdown spans within a single line of text.
/// Handles: `**bold**`, `` `code` ``, `*italic*`.
fn parse_inline(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text.to_string();

    while !rest.is_empty() {
        // ── Bold: **…** ────────────────────────────────────────────────
        if let Some(pos) = rest.find("**") {
            let before = rest[..pos].to_string();
            if !before.is_empty() {
                spans.push(Span::raw(before));
            }
            let after_open = &rest[pos + 2..];
            if let Some(end) = after_open.find("**") {
                spans.push(Span::styled(
                    after_open[..end].to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                rest = after_open[end + 2..].to_string();
                continue;
            } else {
                spans.push(Span::raw(format!("**{after_open}")));
                break;
            }
        }
        // ── Inline code: `…` ──────────────────────────────────────────
        if let Some(pos) = rest.find('`') {
            let before = rest[..pos].to_string();
            if !before.is_empty() {
                spans.push(Span::raw(before));
            }
            let after_open = &rest[pos + 1..];
            if let Some(end) = after_open.find('`') {
                spans.push(Span::styled(
                    after_open[..end].to_string(),
                    Style::default().fg(RC::Yellow),
                ));
                rest = after_open[end + 1..].to_string();
                continue;
            } else {
                spans.push(Span::raw(format!("`{after_open}")));
                break;
            }
        }
        // ── Italic: *…* ───────────────────────────────────────────────
        if let Some(pos) = rest.find('*') {
            let before = rest[..pos].to_string();
            if !before.is_empty() {
                spans.push(Span::raw(before));
            }
            let after_open = &rest[pos + 1..];
            if let Some(end) = after_open.find('*') {
                spans.push(Span::styled(
                    after_open[..end].to_string(),
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
                rest = after_open[end + 1..].to_string();
                continue;
            } else {
                spans.push(Span::raw(format!("*{after_open}")));
                break;
            }
        }
        // ── Plain text ──────────────────────────────────────────────
        spans.push(Span::raw(rest.clone()));
        break;
    }

    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    spans
}

fn parse_list_prefix(s: &str) -> Option<(&str, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit())?;
    if end == 0 {
        return None;
    }
    let rest = s[end..].strip_prefix(". ")?;
    Some((&s[..end], rest))
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
        // Header line + Data line (separator skipped)
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_asymmetric_table_parsing() {
        let md = "| Col 1 | Col 2 |\n|---|---|\n| val 1 | val 2 | val 3 |";
        let lines = parse_markdown_lines(md);
        assert_eq!(lines.len(), 2);
    }
}
