use ratatui::{
    style::{Color as RC, Modifier, Style},
    text::{Line, Span},
};

/// Left-margin indent for markdown paragraphs (matches Letta Code style).
const INDENT: &str = "";

/// Convert a complete markdown text string into a `Vec<Line>` for ratatui rendering.
/// Handles: headings, bullets, numbered lists, code fences, horizontal rules, blockquotes, inline bold/code.
pub fn parse_markdown_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_fence = false;
    let mut current_lang = String::new();

    for raw_line in text.lines() {
        let leading_spaces = raw_line.chars().take_while(|c| c.is_whitespace()).count();
        let trimmed = raw_line.trim_start();

        // ── Code fence toggle ────────────────────────────────────────────
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            if in_fence {
                current_lang = trimmed.trim_start_matches('`').trim().to_lowercase();
                if !current_lang.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("{INDENT}  {current_lang}"),
                        Style::default().fg(RC::DarkGray).add_modifier(Modifier::DIM),
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

        // ── Empty line ────────────────────────────────────────────────────
        if raw_line.trim().is_empty() {
            lines.push(Line::from(""));
            continue;
        }

        // ── Headings ──────────────────────────────────────────────────────
        if let Some(rest) = trimmed.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{rest}"),
                Style::default().fg(RC::Cyan),
            )));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{rest}"),
                Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                format!("{INDENT}{rest}"),
                Style::default().fg(RC::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
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
        if let Some(rest) = trimmed.strip_prefix("> ") {
            let mut spans: Vec<Span<'static>> = vec![
                Span::styled(format!("{INDENT}▎ "), Style::default().fg(RC::DarkGray)),
            ];
            spans.extend(parse_inline(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // ── Bullet list ────────────────────────────────────────────────────
        let bullet_rest = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("• "));
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
        if let Some((num, rest)) = parse_list_prefix(trimmed) {
            let indent_padding = " ".repeat(leading_spaces);
            let mut spans: Vec<Span<'static>> = vec![
                Span::raw(format!("{INDENT}  {indent_padding}")),
                Span::styled(format!("{num}. "), Style::default().add_modifier(Modifier::BOLD)),
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

    lines
}

/// Simple keyword-based syntax highlighter.
fn highlight_code(line: &str, lang: &str) -> Vec<Span<'static>> {
    let style_keyword = Style::default().fg(RC::LightBlue).add_modifier(Modifier::BOLD);
    let style_comment = Style::default().fg(RC::DarkGray).add_modifier(Modifier::ITALIC);
    let style_string  = Style::default().fg(RC::LightGreen);
    let style_type    = Style::default().fg(RC::LightCyan);
    // let style_fn      = Style::default().fg(RC::LightYellow); // Future use for fn highlighting

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
            // Check if it looks like a function call (next non-whitespace is '(')
            // Naive check: just look at the word itself for now
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
    fn test_nested_lists() {
        let md = "- Parent\n  - Child";
        let lines = parse_markdown_lines(md);
        assert_eq!(lines.len(), 2);
        // First line: INDENT + "  " + "• " + "Parent"
        // Second line: INDENT + "  " + "  " + "• " + "Child"
        let first_line_str: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let second_line_str: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(first_line_str, "  • Parent");
        assert_eq!(second_line_str, "    • Child");
    }

    #[test]
    fn test_blockquote() {
        let md = "> This is a quote";
        let lines = parse_markdown_lines(md);
        assert_eq!(lines.len(), 1);
        let line_str: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(line_str, "▎ This is a quote");
    }

    #[test]
    fn test_syntax_highlighting() {
        let md = "```rust\nfn main() {}\n```";
        let lines = parse_markdown_lines(md);
        // Line 0: rust (dim gray)
        // Line 1: fn main() {} (highlighted)
        assert_eq!(lines.len(), 2);
        let first_line_str: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first_line_str.contains("rust"));
        
        // Verify keyword highlighting in second line
        let fn_span = lines[1].spans.iter().find(|s| s.content == "fn");
        assert!(fn_span.is_some());
        assert_eq!(fn_span.unwrap().style.fg, Some(RC::LightBlue));
    }

    #[test]
    fn test_streaming_markdown() {
        let mut text = "This is **bol".to_string();
        let lines = parse_markdown_lines(&text);
        assert_eq!(lines.len(), 1);
        let line_str: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(line_str, "This is **bol");

        text.push_str("d** text");
        let lines2 = parse_markdown_lines(&text);
        assert_eq!(lines2.len(), 1);
        let line_str2: String = lines2[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(line_str2, "This is bold text");
        
        // Verify bold styling
        let bold_span = lines2[0].spans.iter().find(|s| s.content == "bold");
        assert!(bold_span.is_some());
        assert!(bold_span.unwrap().style.add_modifier.contains(Modifier::BOLD));
    }
}
