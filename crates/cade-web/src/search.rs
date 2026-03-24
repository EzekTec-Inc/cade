/// Web search implementations.
///
/// Provider priority:
/// 1. Brave Search API (when `BRAVE_SEARCH_API_KEY` is set) — best results
/// 2. DuckDuckGo Instant Answers API (free, no key) — fallback
use reqwest::Client;
use serde_json::Value;

use crate::Result;

// region:    --- Types

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title:   String,
    pub url:     String,
    pub snippet: String,
}

impl SearchResult {
    /// Format as a block suitable for LLM context injection.
    pub fn to_context_block(&self) -> String {
        format!("**{}**\n{}\n{}", self.title, self.url, self.snippet)
    }
}

// endregion: --- Types

// region:    --- Public API

/// Search the web and return a list of results with titles, URLs, and snippets.
///
/// Uses Brave Search API when `BRAVE_SEARCH_API_KEY` is set; falls back to
/// DuckDuckGo instant answers otherwise.
pub async fn web_search(query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    if let Ok(key) = std::env::var("BRAVE_SEARCH_API_KEY")
        && !key.trim().is_empty() {
            return brave_search(query, limit, &key).await;
        }
    ddg_search(query, limit).await
}

// endregion: --- Public API

// region:    --- Brave Search

async fn brave_search(query: &str, limit: usize, api_key: &str) -> Result<Vec<SearchResult>> {
    let client = Client::builder()
        .user_agent("CADE/0.2 (+https://github.com/EzekTec-Inc/CADE)")
        .build()?;

    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}&search_lang=en",
        urlencoding::encode(query),
        limit.min(20)
    );

    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip")
        .header("X-Subscription-Token", api_key)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        tracing::warn!("Brave Search returned {status} — falling back to DuckDuckGo");
        return ddg_search(query, limit).await;
    }

    let body: Value = resp.json().await?;
    let mut results = Vec::new();

    if let Some(items) = body["web"]["results"].as_array() {
        for item in items.iter().take(limit) {
            let title   = item["title"].as_str().unwrap_or("").to_string();
            let url_str = item["url"].as_str().unwrap_or("").to_string();
            let snippet = item["description"].as_str()
                .or_else(|| item["snippet"].as_str())
                .unwrap_or("")
                .to_string();
            if !url_str.is_empty() {
                results.push(SearchResult { title, url: url_str, snippet });
            }
        }
    }

    Ok(results)
}

// endregion: --- Brave Search

// region:    --- DuckDuckGo

/// DuckDuckGo Instant Answers API — free, no API key required.
///
/// Note: This returns instant answers and related topics, not a full
/// ranked list of search results. For production use, consider Brave API.
async fn ddg_search(query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let client = Client::builder()
        .user_agent("CADE/0.2 (+https://github.com/EzekTec-Inc/CADE)")
        .build()?;

    // DuckDuckGo Instant Answers API
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
        urlencoding::encode(query)
    );

    let resp = client.get(&url).send().await?;
    let body: Value = resp.json().await?;

    let mut results: Vec<SearchResult> = Vec::new();

    // Abstract (top result if present)
    let abstract_text = body["AbstractText"].as_str().unwrap_or("");
    let abstract_url  = body["AbstractURL"].as_str().unwrap_or("");
    let abstract_src  = body["AbstractSource"].as_str().unwrap_or("DuckDuckGo");

    if !abstract_text.is_empty() && !abstract_url.is_empty() {
        results.push(SearchResult {
            title:   format!("{abstract_src}: {query}"),
            url:     abstract_url.to_string(),
            snippet: abstract_text.to_string(),
        });
    }

    // Answer (instant calculation, definition, etc.)
    if let Some(answer) = body["Answer"].as_str().filter(|s| !s.is_empty()) {
        results.push(SearchResult {
            title:   format!("Instant answer for: {query}"),
            url:     format!("https://duckduckgo.com/?q={}", urlencoding::encode(query)),
            snippet: answer.to_string(),
        });
    }

    // Related topics
    if let Some(topics) = body["RelatedTopics"].as_array() {
        for topic in topics.iter().take(limit.saturating_sub(results.len())) {
            // Skip groupings (sub-arrays)
            if topic["Topics"].is_array() { continue; }

            let text = topic["Text"].as_str().unwrap_or("").to_string();
            let url_str = topic["FirstURL"].as_str().unwrap_or("").to_string();

            if text.is_empty() || url_str.is_empty() { continue; }

            // Extract title as the first sentence
            let title = text.split('.').next().unwrap_or(&text).to_string();
            let snippet = if text.len() > title.len() + 2 {
                text[title.len() + 1..].trim().to_string()
            } else {
                text.clone()
            };

            results.push(SearchResult { title, url: url_str, snippet });
            if results.len() >= limit { break; }
        }
    }

    // If DDG returned nothing useful, add a search engine link
    if results.is_empty() {
        results.push(SearchResult {
            title:   format!("Web search: {query}"),
            url:     format!("https://duckduckgo.com/?q={}", urlencoding::encode(query)),
            snippet: "No instant answer found. Visit the URL to search DuckDuckGo directly.\n\
                 Tip: Set BRAVE_SEARCH_API_KEY for richer search results.".to_string(),
        });
    }

    Ok(results)
}

// endregion: --- DuckDuckGo

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_result_context_block() {
        // -- Setup & Fixtures
        let result = SearchResult {
            title:   "Rust Programming Language".to_string(),
            url:     "https://www.rust-lang.org".to_string(),
            snippet: "A language empowering everyone to build reliable software.".to_string(),
        };

        // -- Exec
        let block = result.to_context_block();

        // -- Check
        assert!(block.contains("Rust Programming Language"));
        assert!(block.contains("rust-lang.org"));
        assert!(block.contains("reliable software"));
    }
}

// endregion: --- Tests
