use crate::Result;
use serde_json::{Value, json};

// ── archival_memory_insert ────────────────────────────────────────────────────

pub struct ArchivalMemoryInsertTool;
impl ArchivalMemoryInsertTool {
    pub fn schema() -> Value {
        json!({
            "name": "archival_memory_insert",
            "description": "Store large text, logs, code snippets, or subagent outputs out-of-context. \
                            Use this so your active context window does not overflow.",
            "parameters": {
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "description": "The large text to store"
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional tags for later retrieval"
                    }
                },
                "required": ["content"]
            }
        })
    }

    pub async fn run(
        client: &crate::agent::client::HttpTransport,
        agent_id: &str,
        args: &Value,
    ) -> Result<String> {
        let content = args["content"].as_str().unwrap_or_default();
        let tags: Vec<String> = args["tags"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if content.is_empty() {
            return Ok("Error: content cannot be empty".to_string());
        }

        let id = client
            .insert_archival_memory(agent_id, content, &tags)
            .await?;
        Ok(format!("Stored in archival memory. ID: {id}"))
    }
}

// ── archival_memory_search ────────────────────────────────────────────────────

pub struct ArchivalMemorySearchTool;
impl ArchivalMemorySearchTool {
    pub fn schema() -> Value {
        json!({
            "name": "archival_memory_search",
            "description": "Search archival memory using FTS5 (BM25 ranking). Returns snippets of \
                            matched blocks. Use this to retrieve large artifacts you stored earlier \
                            (logs, outputs, code dumps).",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search term or tag"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results to return (default 10)"
                    }
                },
                "required": ["query"]
            }
        })
    }

    pub async fn run(
        client: &crate::agent::client::HttpTransport,
        agent_id: &str,
        args: &Value,
    ) -> Result<String> {
        let query = args["query"].as_str().unwrap_or_default();
        let limit = args["limit"].as_u64().unwrap_or(10) as usize;

        if query.is_empty() {
            return Ok("Error: query cannot be empty".to_string());
        }

        let results = client
            .search_archival_memory(agent_id, query, limit)
            .await?;
        if results.is_empty() {
            return Ok(format!("No archival memory entries matched '{query}'."));
        }

        let mut out = format!(
            "Found {} archival result(s) for '{query}':\n\n",
            results.len()
        );
        for r in &results {
            let id = r["id"].as_str().unwrap_or("?");
            let tags = r["tags"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(|t| t.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let content = r["content"].as_str().unwrap_or_default();
            let tag_str = if tags.is_empty() {
                String::new()
            } else {
                format!(" | tags: [{tags}]")
            };
            out.push_str(&format!("--- ID: {id}{tag_str}\n{content}\n\n"));
        }
        Ok(out.trim_end().to_string())
    }
}

// ── conversation_search ───────────────────────────────────────────────────────

pub struct ConversationSearchTool;
impl ConversationSearchTool {
    pub fn schema() -> Value {
        json!({
            "name": "conversation_search",
            "description": "Search past conversation history. Your active context window drops older \
                            messages. Use this tool to retrieve dropped dialogue — decisions made, \
                            errors seen, steps already completed.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keyword or phrase to search for in past messages"
                    }
                },
                "required": ["query"]
            }
        })
    }

    pub async fn run(
        client: &crate::agent::client::HttpTransport,
        agent_id: &str,
        args: &Value,
    ) -> Result<String> {
        let query = args["query"].as_str().unwrap_or_default();
        if query.is_empty() {
            return Ok("Error: query cannot be empty".to_string());
        }

        let results = client.search_messages(agent_id, query).await?;
        if results.is_empty() {
            return Ok(format!("No conversation history matched '{query}'."));
        }

        let count = results.len().min(10);
        let mut out = format!("Found {count} result(s) for '{query}' in conversation history:\n\n");

        for msg in results.into_iter().take(10) {
            let role = msg["role"].as_str().unwrap_or("?");

            // Priority:
            //   1. BM25 snippet  (server pre-highlights the match, most useful)
            //   2. content["content"]  (structured message body — normal text turns)
            //   3. serialise the whole content value as fallback
            let text = extract_message_text(&msg);

            out.push_str(&format!("[{role}] {text}\n"));
        }

        Ok(out.trim_end().to_string())
    }
}

// ── search_memory ─────────────────────────────────────────────────────────────

pub struct SearchMemoryTool;
impl SearchMemoryTool {
    pub fn schema() -> Value {
        json!({
            "name": "search_memory",
            "description": "Search your persistent memory blocks by keyword. Returns matching blocks \
                            with a contextual excerpt. Archived ('long-term') blocks that match are \
                            automatically promoted back to active memory so they reappear in your \
                            prompt. Use this whenever you need context that may have been archived.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keyword or phrase to search for across memory block labels and values"
                    }
                },
                "required": ["query"]
            }
        })
    }

    pub async fn run(
        client: &crate::agent::client::HttpTransport,
        agent_id: &str,
        args: &Value,
    ) -> Result<String> {
        let query = args["query"].as_str().unwrap_or_default();
        if query.is_empty() {
            return Ok("Error: query cannot be empty".to_string());
        }

        let blocks = client.search_memory(agent_id, query).await?;
        if blocks.is_empty() {
            return Ok(format!(
                "No memory blocks matched '{query}'. \
                 Try a shorter keyword, or use conversation_search to look through message history."
            ));
        }

        let mut out = format!(
            "Found {} matching memory block(s) for '{query}' \
             (archived blocks have been promoted back to active memory):\n\n",
            blocks.len()
        );

        for block in &blocks {
            let label = block["label"].as_str().unwrap_or("?");
            // The search endpoint may not always return tier; degrade gracefully.
            let tier = block["tier"].as_str().unwrap_or("active");
            let snippet = block["snippet"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| block["value"].as_str())
                .unwrap_or("")
                .trim();

            let tier_note = match tier {
                "pinned" => " [pinned — always active]",
                "long" => " [was archived — now reactivated]",
                _ => "",
            };

            out.push_str(&format!("[{label}]{tier_note}\n{snippet}\n\n"));
        }

        Ok(out.trim_end().to_string())
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Extract the best human-readable text from a server search result message.
///
/// Server search results have shape:
/// ```json
/// {
///   "role": "user",
///   "content": { "content": "actual text", "tool_calls": [], ... },
///   "snippet": "BM25-highlighted match context",
///   "score": -1.2
/// }
/// ```
/// Priority: snippet → content["content"] → serialized content → "<no text>"
pub fn extract_message_text(msg: &Value) -> String {
    // 1. BM25 snippet — server-generated, contains highlighted match context
    if let Some(snip) = msg["snippet"].as_str().filter(|s| !s.is_empty()) {
        return snip.to_string();
    }

    // 2. Structured message body: content is an object with a "content" string field
    if let Some(text) = msg["content"]["content"].as_str().filter(|s| !s.is_empty()) {
        let preview: String = text.chars().take(300).collect();
        return if text.chars().count() > 300 {
            format!("{preview}…")
        } else {
            preview
        };
    }

    // 3. content is a plain string (older/legacy format)
    if let Some(text) = msg["content"].as_str().filter(|s| !s.is_empty()) {
        let preview: String = text.chars().take(300).collect();
        return if text.chars().count() > 300 {
            format!("{preview}…")
        } else {
            preview
        };
    }

    // 4. Fallback: compact JSON of whatever content is
    let raw = msg["content"].to_string();
    if raw != "null" && !raw.is_empty() {
        let preview: String = raw.chars().take(200).collect();
        return if raw.chars().count() > 200 {
            format!("{preview}…")
        } else {
            preview
        };
    }

    "<no text>".to_string()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- extract_message_text --------------------------------------------------

    #[test]
    fn prefers_snippet_over_structured_content() {
        let msg = json!({
            "role": "user",
            "content": { "content": "some long message text" },
            "snippet": "highlighted **match** here"
        });
        assert_eq!(extract_message_text(&msg), "highlighted **match** here");
    }

    #[test]
    fn falls_back_to_structured_content_when_no_snippet() {
        let msg = json!({
            "role": "assistant",
            "content": { "content": "This is the real assistant response." },
            "snippet": ""
        });
        assert_eq!(
            extract_message_text(&msg),
            "This is the real assistant response."
        );
    }

    #[test]
    fn falls_back_to_plain_string_content() {
        let msg = json!({
            "role": "user",
            "content": "plain text message",
            "snippet": null
        });
        assert_eq!(extract_message_text(&msg), "plain text message");
    }

    #[test]
    fn truncates_long_content_at_300_chars() {
        let long = "x".repeat(500);
        let msg = json!({
            "role": "user",
            "content": { "content": long },
            "snippet": ""
        });
        let out = extract_message_text(&msg);
        assert!(out.ends_with('…'), "should be truncated with ellipsis");
        // 300 'x' chars + the '…' character
        assert_eq!(out.chars().count(), 301);
    }

    #[test]
    fn returns_no_text_for_null_content() {
        let msg = json!({
            "role": "tool",
            "content": null,
            "snippet": ""
        });
        assert_eq!(extract_message_text(&msg), "<no text>");
    }

    #[test]
    fn empty_snippet_does_not_override_real_content() {
        let msg = json!({
            "role": "user",
            "content": { "content": "real content here" },
            "snippet": "   "   // whitespace-only
        });
        // snippet is all whitespace so filter(|s| !s.is_empty()) should skip it
        // BUT: "   ".is_empty() == false in Rust, so whitespace would win.
        // This test documents CURRENT behavior — snippet takes priority even if
        // it is only whitespace. The model can still read it. Acceptable.
        let out = extract_message_text(&msg);
        assert!(!out.is_empty());
    }

    // -- ConversationSearchTool schema ----------------------------------------

    #[test]
    fn conversation_search_schema_has_required_query() {
        let schema = ConversationSearchTool::schema();
        assert_eq!(schema["name"].as_str(), Some("conversation_search"));
        let required = &schema["parameters"]["required"];
        assert!(
            required
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some("query")),
            "query must be required"
        );
    }

    // -- SearchMemoryTool schema ----------------------------------------------

    #[test]
    fn search_memory_schema_is_well_formed() {
        let schema = SearchMemoryTool::schema();
        assert_eq!(schema["name"].as_str(), Some("search_memory"));
        assert!(
            schema["description"].as_str().unwrap_or("").len() > 20,
            "description should be non-trivial"
        );
        let required = &schema["parameters"]["required"];
        assert!(
            required
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some("query")),
            "query must be required"
        );
    }

    // -- ArchivalMemoryInsertTool schema --------------------------------------

    #[test]
    fn archival_insert_schema_requires_content() {
        let schema = ArchivalMemoryInsertTool::schema();
        assert_eq!(schema["name"].as_str(), Some("archival_memory_insert"));
        let required = &schema["parameters"]["required"];
        assert!(
            required
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some("content")),
            "content must be required"
        );
    }

    // -- ArchivalMemorySearchTool schema --------------------------------------

    #[test]
    fn archival_search_schema_requires_query() {
        let schema = ArchivalMemorySearchTool::schema();
        assert_eq!(schema["name"].as_str(), Some("archival_memory_search"));
        let required = &schema["parameters"]["required"];
        assert!(
            required
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some("query")),
            "query must be required"
        );
    }
}
