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
    Link { text: String, url: String },
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
                    let span = if let Some(ref url) = current_link_url {
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
                        class: "bg-[#222530] text-[#ff7c5c] px-1.5 py-0.5 rounded text-xs font-mono font-medium border border-[#313545]",
                        "{c}"
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
            }
        }
    }
}

fn render_block(block: MarkdownBlock, idx: usize) -> Element {
    match block {
        MarkdownBlock::Heading { level, inlines } => {
            let class_name = match level {
                1 => "text-lg font-bold text-white mt-3 mb-1 pb-1 border-b border-[#272833]",
                2 => "text-base font-bold text-sky-400 mt-2.5 mb-1",
                3 => "text-sm font-semibold text-emerald-400 mt-2 mb-0.5",
                _ => "text-xs font-semibold text-purple-400 mt-1.5 mb-0.5 uppercase tracking-wide",
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
                div { key: "{idx}", class: "my-2.5 rounded-lg border border-[#272833] bg-[#0d0e12] overflow-hidden group/code",
                    div { class: "flex justify-between items-center px-3 py-1.5 bg-[#16171d] border-b border-[#272833] text-[11px] text-gray-400 font-mono select-none",
                        span { class: "uppercase font-semibold text-gray-400", "{display_lang}" }
                        button {
                            class: "text-gray-400 hover:text-white transition px-2 py-0.5 rounded bg-[#1e2029] hover:bg-[#282b37] border border-[#2d313f] text-[10px] font-sans flex items-center space-x-1",
                            onclick: move |_| {
                                let c = code_for_copy.clone();
                                crate::api::copyText(&c);
                            },
                            "Copy"
                        }
                    }
                    pre { class: "p-3 font-mono text-xs text-gray-200 overflow-x-auto whitespace-pre",
                        code { "{code}" }
                    }
                }
            }
        }
        MarkdownBlock::Blockquote(inlines) => rsx! {
            blockquote { key: "{idx}", class: "border-l-4 border-[#8b5cf6] bg-[#8b5cf6]/10 px-3 py-1.5 my-2 italic text-gray-300 text-xs rounded-r",
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
            div { key: "{idx}", class: "my-2 overflow-x-auto rounded-lg border border-[#272833]",
                table { class: "w-full border-collapse text-xs",
                    thead { class: "bg-[#16171d] border-b border-[#272833]",
                        tr {
                            for (h_idx, header) in headers.into_iter().enumerate() {
                                th { key: "{h_idx}", class: "p-2.5 text-left font-semibold text-gray-300 border-r border-[#272833] last:border-r-0",
                                    "{header}"
                                }
                            }
                        }
                    }
                    tbody { class: "divide-y divide-[#272833]",
                        for (r_idx, row) in rows.into_iter().enumerate() {
                            tr { key: "{r_idx}", class: "hover:bg-[#16171d]/50 transition",
                                for (c_idx, cell) in row.into_iter().enumerate() {
                                    td { key: "{c_idx}", class: "p-2.5 text-gray-300 border-r border-[#272833] last:border-r-0",
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