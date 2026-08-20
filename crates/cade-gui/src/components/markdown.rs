//! Rich Markdown rendering component for Dioxus in CADE GUI.
//!
//! Parses Markdown text into structured blocks and inlines with syntax-aware
//! code blocks, copy-to-clipboard buttons, tables, and Tailwind styling.

use dioxus::prelude::*;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Inline styled span element
#[derive(Debug, Clone, PartialEq)]
pub enum InlineSpan {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Kbd(String),
    Link { text: String, url: String },
    Html(String),
}

/// Top-level Markdown block element
#[derive(Debug, Clone, PartialEq)]
pub enum MarkdownBlock {
    Heading { level: u32, inlines: Vec<InlineSpan> },
    Paragraph(Vec<InlineSpan>),
    CodeBlock { lang: String, code: String },
    Blockquote(Vec<InlineSpan>),
    List { ordered: bool, items: Vec<Vec<InlineSpan>> },
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    HorizontalRule,
}

/// Parse raw markdown text into structured `Vec<MarkdownBlock>`.
pub fn parse_markdown(text: &str) -> Vec<MarkdownBlock> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(text, options);
    let mut blocks = Vec::new();

    let mut current_inlines = Vec::new();
    let mut is_bold = false;
    let mut is_italic = false;
    let mut current_link_url = None;
    let mut current_code_lang = String::new();
    let mut current_code_text = String::new();
    let mut in_code_block = false;
    let mut current_heading_level = 1;
    let mut in_blockquote = false;
    let mut in_list = false;
    let mut list_ordered = false;
    let mut list_items = Vec::new();
    let mut current_list_item = Vec::new();
    let mut in_table = false;
    let mut table_headers = Vec::new();
    let mut table_rows = Vec::new();
    let mut current_table_row = Vec::new();
    let mut current_cell_text = String::new();
    let mut in_table_head = false;
    let mut in_kbd = false;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    current_inlines.clear();
                }
                Tag::Heading { level, .. } => {
                    current_heading_level = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };
                    current_inlines.clear();
                }
                Tag::BlockQuote(_) => {
                    in_blockquote = true;
                    current_inlines.clear();
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    current_code_text.clear();
                    current_code_lang = match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) => lang.to_string(),
                        pulldown_cmark::CodeBlockKind::Indented => String::new(),
                    };
                }
                Tag::List(first_num) => {
                    in_list = true;
                    list_ordered = first_num.is_some();
                    list_items.clear();
                }
                Tag::Item => {
                    current_list_item.clear();
                }
                Tag::Table(_) => {
                    in_table = true;
                    table_headers.clear();
                    table_rows.clear();
                }
                Tag::TableHead => {
                    in_table_head = true;
                }
                Tag::TableRow => {
                    current_table_row.clear();
                }
                Tag::TableCell => {
                    current_cell_text.clear();
                }
                Tag::Emphasis => {
                    is_italic = true;
                }
                Tag::Strong => {
                    is_bold = true;
                }
                Tag::Link { dest_url, .. } => {
                    current_link_url = Some(dest_url.to_string());
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph => {
                    if !in_list && !in_blockquote && !current_inlines.is_empty() {
                        blocks.push(MarkdownBlock::Paragraph(std::mem::take(&mut current_inlines)));
                    }
                }
                TagEnd::Heading(_) => {
                    blocks.push(MarkdownBlock::Heading {
                        level: current_heading_level,
                        inlines: std::mem::take(&mut current_inlines),
                    });
                }
                TagEnd::BlockQuote(_) => {
                    in_blockquote = false;
                    blocks.push(MarkdownBlock::Blockquote(std::mem::take(&mut current_inlines)));
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    blocks.push(MarkdownBlock::CodeBlock {
                        lang: std::mem::take(&mut current_code_lang),
                        code: std::mem::take(&mut current_code_text),
                    });
                }
                TagEnd::List(_) => {
                    in_list = false;
                    if !list_items.is_empty() {
                        blocks.push(MarkdownBlock::List {
                            ordered: list_ordered,
                            items: std::mem::take(&mut list_items),
                        });
                    }
                }
                TagEnd::Item => {
                    if !current_list_item.is_empty() {
                        list_items.push(std::mem::take(&mut current_list_item));
                    } else if !current_inlines.is_empty() {
                        list_items.push(std::mem::take(&mut current_inlines));
                    }
                }
                TagEnd::TableHead => {
                    in_table_head = false;
                }
                TagEnd::TableRow => {
                    if in_table_head {
                        table_headers = std::mem::take(&mut current_table_row);
                    } else {
                        table_rows.push(std::mem::take(&mut current_table_row));
                    }
                }
                TagEnd::TableCell => {
                    current_table_row.push(std::mem::take(&mut current_cell_text));
                }
                TagEnd::Table => {
                    in_table = false;
                    blocks.push(MarkdownBlock::Table {
                        headers: std::mem::take(&mut table_headers),
                        rows: std::mem::take(&mut table_rows),
                    });
                }
                TagEnd::Emphasis => {
                    is_italic = false;
                }
                TagEnd::Strong => {
                    is_bold = false;
                }
                TagEnd::Link => {
                    current_link_url = None;
                }
                _ => {}
            },
            Event::Text(t) => {
                let s = t.to_string();
                if in_code_block {
                    current_code_text.push_str(&s);
                } else if in_table {
                    current_cell_text.push_str(&s);
                } else {
                    let span = if in_kbd {
                        InlineSpan::Kbd(s)
                    } else if let Some(ref url) = current_link_url {
                        InlineSpan::Link {
                            text: s,
                            url: url.clone(),
                        }
                    } else if is_bold {
                        InlineSpan::Bold(s)
                    } else if is_italic {
                        InlineSpan::Italic(s)
                    } else {
                        InlineSpan::Text(s)
                    };

                    if in_list {
                        current_list_item.push(span);
                    } else {
                        current_inlines.push(span);
                    }
                }
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                let s = h.to_string();
                if in_code_block {
                    current_code_text.push_str(&s);
                } else if in_table {
                    current_cell_text.push_str(&s);
                } else {
                    let trimmed = s.trim().to_lowercase();
                    if trimmed == "<kbd>" {
                        in_kbd = true;
                    } else if trimmed == "</kbd>" {
                        in_kbd = false;
                    } else if trimmed.starts_with("<kbd>") && trimmed.ends_with("</kbd>") {
                        let inner = s.trim()
                            .strip_prefix("<kbd>")
                            .and_then(|t| t.strip_suffix("</kbd>"))
                            .unwrap_or(&s);
                        let span = InlineSpan::Kbd(inner.to_string());
                        if in_list {
                            current_list_item.push(span);
                        } else {
                            current_inlines.push(span);
                        }
                    } else {
                        let span = InlineSpan::Html(s);
                        if in_list {
                            current_list_item.push(span);
                        } else {
                            current_inlines.push(span);
                        }
                    }
                }
            }
            Event::Code(c) => {
                let span = InlineSpan::Code(c.to_string());
                if in_list {
                    current_list_item.push(span);
                } else {
                    current_inlines.push(span);
                }
            }
            Event::Rule => {
                blocks.push(MarkdownBlock::HorizontalRule);
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_code_block {
                    current_code_text.push('\n');
                } else if in_list {
                    current_list_item.push(InlineSpan::Text(" ".to_string()));
                } else {
                    current_inlines.push(InlineSpan::Text(" ".to_string()));
                }
            }
            _ => {}
        }
    }

    if !current_inlines.is_empty() {
        blocks.push(MarkdownBlock::Paragraph(current_inlines));
    }

    blocks
}

/// Rich Markdown view component rendering parsed blocks
#[component]
pub fn MarkdownView(content: String) -> Element {
    let blocks = parse_markdown(&content);

    rsx! {
        div { class: "markdown-body flex flex-col space-y-2 text-sm text-gray-200 leading-relaxed break-words",
            for (idx, block) in blocks.into_iter().enumerate() {
                {render_block(block, idx)}
            }
        }
    }
}

fn render_inlines(inlines: Vec<InlineSpan>) -> Element {
    rsx! {
        for (i, span) in inlines.into_iter().enumerate() {
            match span {
                InlineSpan::Text(t) => rsx! { span { key: "{i}", "{t}" } },
                InlineSpan::Bold(b) => rsx! { strong { key: "{i}", class: "font-bold text-white", "{b}" } },
                InlineSpan::Italic(it) => rsx! { em { key: "{i}", class: "italic text-gray-300", "{it}" } },
                InlineSpan::Code(c) => rsx! {
                    code {
                        key: "{i}",
                        class: "bg-[#181c29] text-[#ff7c5c] px-1.5 py-0.5 rounded-md text-xs font-mono font-semibold border border-[#2c3349] mx-0.5",
                        "{c}"
                    }
                },
                InlineSpan::Kbd(k) => rsx! {
                    kbd {
                        key: "{i}",
                        class: "bg-[#151824] text-slate-200 border border-[#343b52] px-1.5 py-0.5 rounded-md text-[11px] font-mono font-semibold shadow-[0_2px_0_0_#232838] mx-0.5 inline-block",
                        "{k}"
                    }
                },
                InlineSpan::Link { text, url } => rsx! {
                    a {
                        key: "{i}",
                        class: "text-sky-400 hover:text-sky-300 underline underline-offset-2 transition",
                        href: "{url}",
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "{text}"
                    }
                },
                InlineSpan::Html(h) => rsx! {
                    span {
                        key: "{i}",
                        class: "inline-html text-gray-300 font-mono text-xs",
                        "{h}"
                    }
                },
            }
        }
    }
}

fn render_block(block: MarkdownBlock, idx: usize) -> Element {
    match block {
        MarkdownBlock::Heading { level, inlines } => {
            let class_name = match level {
                1 => "text-base font-bold text-transparent bg-clip-text bg-gradient-to-r from-sky-400 to-indigo-300 mt-4 mb-2 pb-1.5 border-b border-[#232738]",
                2 => "text-sm font-bold text-sky-400 mt-3 mb-1.5",
                3 => "text-xs font-semibold text-emerald-400 mt-2.5 mb-1",
                _ => "text-[11px] font-semibold text-purple-400 mt-2 mb-0.5 uppercase tracking-wider",
            };
            rsx! {
                div { key: "{idx}", class: "{class_name}",
                    {render_inlines(inlines)}
                }
            }
        }
        MarkdownBlock::Paragraph(inlines) => rsx! {
            p { key: "{idx}", class: "my-1 leading-normal",
                {render_inlines(inlines)}
            }
        },
        MarkdownBlock::CodeBlock { lang, code } => {
            let display_lang = if lang.is_empty() { "text".to_string() } else { lang.clone() };
            let code_for_copy = code.clone();
            rsx! {
                div { key: "{idx}", class: "my-3 rounded-xl border border-[#232738] bg-[#0b0d13] shadow-lg overflow-hidden group/code font-mono",
                    div { class: "flex justify-between items-center px-3.5 py-2 bg-[#12151f] border-b border-[#232738] text-[11px] select-none",
                        div { class: "flex items-center space-x-2",
                            span { class: "w-2.5 h-2.5 rounded-full bg-[#ff5f56]/80 inline-block" }
                            span { class: "w-2.5 h-2.5 rounded-full bg-[#ffbd2e]/80 inline-block" }
                            span { class: "w-2.5 h-2.5 rounded-full bg-[#27c93f]/80 inline-block" }
                            span { class: "font-semibold text-slate-400 pl-2 uppercase tracking-wider", "{display_lang}" }
                        }
                        button {
                            class: "text-slate-400 hover:text-white transition px-2.5 py-0.5 rounded-md bg-[#1c202e] hover:bg-[#282e42] border border-[#2e344a] text-[10px] font-sans flex items-center space-x-1.5 shadow-sm",
                            onclick: move |_| {
                                let c = code_for_copy.clone();
                                crate::api::copyText(&c);
                            },
                            svg { class: "w-3 h-3 text-slate-400", fill: "none", view_box: "0 0 24 24", stroke: "currentColor", "stroke-width": "2",
                                path { "stroke-linecap": "round", "stroke-linejoin": "round", d: "M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" }
                            }
                            span { "Copy" }
                        }
                    }
                    pre { class: "p-3.5 font-mono text-xs text-slate-200 overflow-x-auto whitespace-pre leading-relaxed",
                        code { "{code}" }
                    }
                }
            }
        }
        MarkdownBlock::Blockquote(inlines) => rsx! {
            blockquote { key: "{idx}", class: "border-l-4 border-violet-500 bg-violet-950/20 px-3.5 py-2 my-2.5 rounded-r-lg text-slate-300 text-xs italic shadow-inner",
                {render_inlines(inlines)}
            }
        },
        MarkdownBlock::List { ordered, items } => rsx! {
            if ordered {
                ol { key: "{idx}", class: "list-decimal list-inside space-y-1 my-1.5 pl-2 text-gray-300",
                    for (item_idx, item_inlines) in items.into_iter().enumerate() {
                        li { key: "{item_idx}", {render_inlines(item_inlines)} }
                    }
                }
            } else {
                ul { key: "{idx}", class: "list-disc list-inside space-y-1 my-1.5 pl-2 text-gray-300",
                    for (item_idx, item_inlines) in items.into_iter().enumerate() {
                        li { key: "{item_idx}", {render_inlines(item_inlines)} }
                    }
                }
            }
        },
        MarkdownBlock::Table { headers, rows } => rsx! {
            div { key: "{idx}", class: "my-3 overflow-x-auto rounded-xl border border-[#232738] bg-[#0f121a] shadow-md",
                table { class: "w-full border-collapse text-xs",
                    thead { class: "bg-[#161a26] border-b border-[#232738]",
                        tr {
                            for (h_idx, header) in headers.into_iter().enumerate() {
                                th { key: "{h_idx}", class: "p-2.5 text-left font-bold text-slate-200 uppercase tracking-wider text-[10px] border-r border-[#232738] last:border-r-0",
                                    "{header}"
                                }
                            }
                        }
                    }
                    tbody { class: "divide-y divide-[#232738]",
                        for (r_idx, row) in rows.into_iter().enumerate() {
                            tr { key: "{r_idx}", class: "hover:bg-[#1c2233]/60 transition even:bg-[#121520]/40",
                                for (c_idx, cell) in row.into_iter().enumerate() {
                                    td { key: "{c_idx}", class: "p-2.5 text-slate-300 border-r border-[#232738] last:border-r-0 font-normal",
                                        "{cell}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        MarkdownBlock::HorizontalRule => rsx! {
            hr { key: "{idx}", class: "my-3 border-[#272833]" }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_markdown_headings_and_code() {
        let md = "# Title

Here is a paragraph with **bold** and `inline_code`.

```rust
fn main() {}
```

- Item 1
- Item 2";
        let blocks = parse_markdown(md);
        assert_eq!(blocks.len(), 4);
        assert!(matches!(blocks[0], MarkdownBlock::Heading { level: 1, .. }));
        assert!(matches!(blocks[1], MarkdownBlock::Paragraph(_)));
        assert!(matches!(blocks[2], MarkdownBlock::CodeBlock { ref lang, .. } if lang == "rust"));
        assert!(matches!(blocks[3], MarkdownBlock::List { ordered: false, .. }));
    }

    #[test]
    fn test_parse_markdown_inline_html_and_kbd() {
        let md = "Press <kbd>Ctrl+C</kbd> to exit or <kbd>Enter</kbd> to submit.";
        let blocks = parse_markdown(md);
        assert_eq!(blocks.len(), 1);
        if let MarkdownBlock::Paragraph(ref inlines) = blocks[0] {
            assert!(inlines.iter().any(|i| matches!(i, InlineSpan::Kbd(k) if k == "Ctrl+C")));
            assert!(inlines.iter().any(|i| matches!(i, InlineSpan::Kbd(k) if k == "Enter")));
        } else {
            panic!("Expected MarkdownBlock::Paragraph");
        }
    }
}