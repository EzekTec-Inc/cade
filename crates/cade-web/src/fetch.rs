/// Document fetching: download a URL and extract clean readable text.
use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

use crate::Result;

// region:    --- Types

#[derive(Debug, Clone)]
pub struct FetchedDoc {
    pub url:         String,
    pub title:       String,
    pub text:        String,
    pub word_count:  usize,
    pub truncated:   bool,
}

impl FetchedDoc {
    /// Build context block for LLM injection.
    pub fn to_context_block(&self) -> String {
        let trunc = if self.truncated { " (truncated)" } else { "" };
        let (title, url, words, text) = (&self.title, &self.url, self.word_count, &self.text);
        format!("## {title}\nSource: {url} ({words} words{trunc})\n\n{text}")
    }
}

// endregion: --- Types

// region:    --- Public API

/// Fetch a URL and extract its main text content.
/// Returns clean prose — strips navigation, ads, scripts, and boilerplate.
pub async fn fetch_doc(url_str: &str, max_chars: usize) -> Result<FetchedDoc> {
    // Validate URL
    let parsed = Url::parse(url_str)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(crate::Error::custom(format!("Unsupported scheme: {}", parsed.scheme())));
    }

    let client = Client::builder()
        .user_agent("CADE/0.2 (+https://github.com/EzekTec-Inc/CADE)")
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;

    let resp = client.get(url_str).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(crate::Error::custom(format!("HTTP {status} for {url_str}")));
    }

    let content_type = resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body = resp.text().await?;

    // Route to appropriate parser
    if content_type.contains("text/html") || content_type.is_empty() {
        parse_html(url_str, &body, max_chars)
    } else {
        // Plain text or other formats
        let text = body.chars().take(max_chars).collect::<String>();
        let truncated = body.len() > max_chars;
        Ok(FetchedDoc {
            url:        url_str.to_string(),
            title:      url_str.to_string(),
            word_count: text.split_whitespace().count(),
            truncated,
            text,
        })
    }
}

// endregion: --- Public API

// region:    --- HTML parsing

fn parse_html(url_str: &str, html: &str, max_chars: usize) -> Result<FetchedDoc> {
    let document = Html::parse_document(html);

    // -- Extract title
    let title = extract_title(&document)
        .unwrap_or_else(|| url_str.to_string());

    // -- Remove noise elements
    let noise_selectors = [
        "script", "style", "noscript", "nav", "footer", "header",
        "aside", ".ad", ".ads", ".advertisement", ".cookie-banner",
        ".social-share", ".comments", ".sidebar", "[aria-hidden='true']",
        "form", "button", "input", "select",
    ];

    // -- Extract main content
    // Priority: <main>, <article>, [role=main], then fallback to <body>
    let main_sel = Selector::parse("main, article, [role='main'], .content, .post-content, .entry-content, #content, #main")
        .expect("valid selector");

    let body_sel = Selector::parse("body").expect("valid selector");

    let content_node = document.select(&main_sel).next()
        .or_else(|| document.select(&body_sel).next());

    let Some(content_node) = content_node else {
        return Ok(FetchedDoc {
            url: url_str.to_string(),
            title,
            text: "No content found.".to_string(),
            word_count: 0,
            truncated: false,
        });
    };

    // Walk the content tree, collecting text from meaningful elements
    let text = extract_text_from_node(&document, html, &content_node, &noise_selectors);

    // Clean up whitespace
    let text = clean_whitespace(&text);

    // Apply limit
    let truncated = text.chars().count() > max_chars;
    let text: String = text.chars().take(max_chars).collect();
    let word_count = text.split_whitespace().count();

    Ok(FetchedDoc { url: url_str.to_string(), title, text, word_count, truncated })
}

fn extract_title(document: &Html) -> Option<String> {
    // Try <meta property="og:title"> first (usually cleaner)
    if let Ok(sel) = Selector::parse("meta[property='og:title']")
        && let Some(el) = document.select(&sel).next()
            && let Some(content) = el.value().attr("content") {
                let t = content.trim().to_string();
                if !t.is_empty() { return Some(t); }
            }
    // Fall back to <title>
    if let Ok(sel) = Selector::parse("title")
        && let Some(el) = document.select(&sel).next() {
            let t = el.text().collect::<String>().trim().to_string();
            if !t.is_empty() { return Some(t); }
        }
    None
}

fn extract_text_from_node(
    _document: &Html,
    _html: &str,
    node: &scraper::ElementRef<'_>,
    _noise_selectors: &[&str],
) -> String {
    // Walk the subtree and collect text from paragraph-level elements
    let mut text = String::new();
    let block_tags = ["p", "h1", "h2", "h3", "h4", "h5", "h6",
                      "li", "td", "th", "blockquote", "pre",
                      "div", "section", "article"];

    collect_text_recursive(node, &block_tags, &mut text, 0);
    text
}

fn collect_text_recursive(
    node: &scraper::ElementRef<'_>,
    block_tags: &[&str],
    out: &mut String,
    depth: usize,
) {
    if depth > 50 { return; } // avoid deep recursion on malformed HTML

    let tag = node.value().name().to_lowercase();

    // Skip noise tags entirely
    if matches!(tag.as_str(), "script" | "style" | "noscript" | "nav" |
                "footer" | "header" | "aside" | "form" | "button" | "input") {
        return;
    }

    if block_tags.contains(&tag.as_str()) {
        // Collect direct text + recurse into inline children
        let node_text: String = node.text().collect::<Vec<_>>().join(" ");
        let cleaned = node_text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !cleaned.is_empty() {
            out.push_str(&cleaned);
            out.push('\n');
        }
    } else {
        // Recurse into children
        for child in node.children() {
            if let Some(el) = scraper::ElementRef::wrap(child) {
                collect_text_recursive(&el, block_tags, out, depth + 1);
            }
        }
    }
}

fn clean_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_was_newline = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !last_was_newline {
                result.push('\n');
                last_was_newline = true;
            }
        } else {
            result.push_str(trimmed);
            result.push('\n');
            last_was_newline = false;
        }
    }
    result.trim().to_string()
}

// endregion: --- HTML parsing

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_whitespace() {
        // -- Setup & Fixtures
        let input = "  Hello world  \n\n\n  Foo bar  \n\n";

        // -- Exec
        let out = clean_whitespace(input);

        // -- Check
        assert_eq!(out, "Hello world\n\nFoo bar");
    }

    #[test]
    fn test_fetched_doc_context_block() {
        // -- Setup & Fixtures
        let doc = FetchedDoc {
            url:        "https://example.com".to_string(),
            title:      "Example Domain".to_string(),
            text:       "This domain is for use in examples.".to_string(),
            word_count: 7,
            truncated:  false,
        };

        // -- Exec
        let block = doc.to_context_block();

        // -- Check
        assert!(block.contains("Example Domain"));
        assert!(block.contains("example.com"));
        assert!(block.contains("7 words"));
    }

    #[test]
    fn test_fetched_doc_truncated_indicator() {
        // -- Setup & Fixtures
        let doc = FetchedDoc {
            url:        "https://example.com".to_string(),
            title:      "Test".to_string(),
            text:       "content".to_string(),
            word_count: 1,
            truncated:  true,
        };

        // -- Exec & Check
        assert!(doc.to_context_block().contains("truncated"));
    }
}

// endregion: --- Tests
