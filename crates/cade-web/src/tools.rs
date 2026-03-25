/// Agent-callable tool wrappers for web operations.
use serde_json::{Value, json};

use crate::{Result, fetch, search};

// region:    --- WebSearchTool

pub struct WebSearchTool;
impl WebSearchTool {
    pub async fn run(args: &Value) -> Result<String> {
        let query = args["query"].as_str().unwrap_or("").trim();
        let limit = args["limit"].as_u64().unwrap_or(10) as usize;

        if query.is_empty() {
            return Ok("Error: 'query' is required".to_string());
        }

        let results = search::web_search(query, limit).await?;
        if results.is_empty() {
            return Ok(format!("No results found for '{query}'."));
        }

        let mut out = format!(
            "Web search results for '{query}' ({} result(s)):\n\n",
            results.len()
        );
        for (i, r) in results.iter().enumerate() {
            out.push_str(&format!(
                "{}. **{}**\n   {}\n   {}\n\n",
                i + 1,
                r.title,
                r.url,
                r.snippet
            ));
        }
        Ok(out.trim_end().to_string())
    }

    pub fn schema() -> Value {
        json!({
            "name": "web_search",
            "description": "Search the web and return results with titles, URLs, and snippets. Uses Brave Search API (BRAVE_SEARCH_API_KEY) if set, otherwise DuckDuckGo instant answers.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of results to return (default 10)"
                    }
                },
                "required": ["query"]
            }
        })
    }
}

// endregion: --- WebSearchTool

// region:    --- FetchDocTool

pub struct FetchDocTool;
impl FetchDocTool {
    pub async fn run(args: &Value) -> Result<String> {
        let url = args["url"].as_str().unwrap_or("").trim();
        let max_chars = args["max_chars"].as_u64().unwrap_or(20_000) as usize;

        if url.is_empty() {
            return Ok("Error: 'url' is required".to_string());
        }

        let doc = fetch::fetch_doc(url, max_chars).await?;
        Ok(doc.to_context_block())
    }

    pub fn schema() -> Value {
        json!({
            "name": "fetch_doc",
            "description": "Fetch a URL and return its main text content, stripped of HTML boilerplate, scripts, and navigation. Useful for reading documentation, articles, or any web page.",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch (http or https)"
                    },
                    "max_chars": {
                        "type": "integer",
                        "description": "Maximum characters to return (default 20000)"
                    }
                },
                "required": ["url"]
            }
        })
    }
}

// endregion: --- FetchDocTool

// region:    --- BrowserScreenshotTool (delegate to cade-desktop when available)

/// Placeholder — actual implementation lives in cade-desktop.
/// This schema ensures the agent knows about browser screenshot capability.
pub struct BrowserScreenshotTool;
impl BrowserScreenshotTool {
    pub fn schema() -> Value {
        json!({
            "name": "browser_screenshot",
            "description": "Navigate to a URL and take a screenshot. Returns the saved image path. Requires either cade-desktop (for local capture) or a headless browser installed.",
            "parameters": {
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to navigate to and screenshot"
                    },
                    "save_path": {
                        "type": "string",
                        "description": "Where to save the PNG (default: ~/Pictures/browser_<timestamp>.png)"
                    },
                    "wait_ms": {
                        "type": "integer",
                        "description": "Milliseconds to wait for page load (default 2000)"
                    }
                },
                "required": ["url"]
            }
        })
    }
}

// endregion: --- BrowserScreenshotTool
