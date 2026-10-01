//! Deep module for OpenAI tool schema normalization.
//!
//! Encapsulates:
//! 1. 64-character naming compliance matching `^[a-zA-Z0-9_-]{1,64}$`.
//! 2. Safe server-tool compaction when prefixed MCP tools exceed 64 chars.
//! 3. JSON schema cleaning (combinator removal, type: object guarantee).
//! 4. Target dialect packing (Chat Completions function vs Responses flat tool).

use serde_json::{Value, json};

pub struct ToolSchemaNormalizer;

impl ToolSchemaNormalizer {
    /// Normalize a raw tool schema into a certified OpenAI function tool definition.
    ///
    /// If `for_responses_api` is true, returns flat `{ "type": "function", "name": ..., ... }`.
    /// If false, returns `{ "type": "function", "function": { "name": ..., ... } }`.
    pub fn normalize(schema: &Value, for_responses_api: bool) -> Value {
        let raw_name = Self::extract_raw_name(schema);
        let normalized_name = Self::sanitize_tool_name(&raw_name);

        let description = schema
            .get("description")
            .or_else(|| schema.get("function").and_then(|f| f.get("description")))
            .cloned()
            .unwrap_or_else(|| Value::String(String::new()));

        let params_val = schema
            .get("parameters")
            .or_else(|| schema.get("input_schema"))
            .or_else(|| schema.get("function").and_then(|f| f.get("parameters")));

        let mut params = params_val
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}, "required": []}));

        crate::utils::inline_schema_refs(&mut params);
        crate::utils::clean_openai_schema(&mut params);

        // OpenAI strictly requires function parameters to have type 'object'
        // and no top-level combinators ('oneOf', 'anyOf', 'allOf', 'enum', etc.).
        if let Some(obj) = params.as_object_mut() {
            obj.remove("oneOf");
            obj.remove("anyOf");
            obj.remove("allOf");
            obj.remove("one_of");
            obj.remove("any_of");
            obj.remove("all_of");
            obj.remove("enum");
            obj.remove("const");
            obj.remove("not");
            obj.insert("type".to_string(), json!("object"));
            if !obj.contains_key("properties") {
                obj.insert("properties".to_string(), json!({}));
            }
        }

        // Sealing additionalProperties: false is only safe when strict: true or when
        // all properties are required. Leaving it loose prevents proxy/runtime 400 rejections.
        crate::utils::seal_top_level_additional_properties(&mut params);

        if for_responses_api {
            json!({
                "type": "function",
                "name": normalized_name,
                "description": description,
                "parameters": params,
                "strict": false
            })
        } else {
            json!({
                "type": "function",
                "function": {
                    "name": normalized_name,
                    "description": description,
                    "parameters": params,
                    "strict": false
                }
            })
        }
    }

    /// Extract tool name from varying schema locations.
    fn extract_raw_name(schema: &Value) -> String {
        schema
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| {
                schema
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("unknown_tool")
            .to_string()
    }

    /// Sanitize tool name to strictly conform to `^[a-zA-Z0-9_-]{1,64}$`.
    ///
    /// If longer than 64 characters, intelligently compacts the server prefix
    /// while preserving tool identity and uniqueness.
    pub fn sanitize_tool_name(name: &str) -> String {
        // 1. Replace any disallowed characters with underscores
        let sanitized: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        if sanitized.len() <= 64 {
            return sanitized;
        }

        // 2. If length > 64 and formatted as `{server}__{tool}`, compact prefix and suffix
        if let Some((server, tool)) = sanitized.split_once("__") {
            let tool_budget = 40.min(tool.len());
            let server_budget = 64 - tool_budget - 2; // -2 for "__"
            let short_server = if server.len() > server_budget {
                &server[..server_budget]
            } else {
                server
            };
            let short_tool = if tool.len() > tool_budget {
                &tool[..tool_budget]
            } else {
                tool
            };
            let compacted = format!("{short_server}__{short_tool}");
            if compacted.len() <= 64 {
                return compacted;
            }
        }

        // 3. Fallback: truncate to exactly 64 characters
        sanitized[..64].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitizes_disallowed_characters() {
        assert_eq!(
            ToolSchemaNormalizer::sanitize_tool_name("my.server:tool/action"),
            "my_server_tool_action"
        );
    }

    #[test]
    fn test_compacts_names_longer_than_64_chars() {
        let long_server = "desktop-commander-mcp-extended-provider";
        let long_tool = "give_feedback_and_diagnostic_telemetry_to_upstream";
        let full = format!("{long_server}__{long_tool}");
        assert!(full.len() > 64);

        let sanitized = ToolSchemaNormalizer::sanitize_tool_name(&full);
        assert!(sanitized.len() <= 64);
        assert!(sanitized.contains("__"));
        assert!(
            sanitized
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
    }

    #[test]
    fn test_normalize_chat_completions() {
        let schema = json!({
            "name": "test_tool",
            "description": "A test tool",
            "parameters": {
                "type": "object",
                "properties": {
                    "count": { "type": "integer" }
                }
            }
        });

        let out = ToolSchemaNormalizer::normalize(&schema, false);
        assert_eq!(out["type"], "function");
        assert_eq!(out["function"]["name"], "test_tool");
        assert_eq!(out["function"]["strict"], false);
        assert_eq!(out["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn test_normalize_responses_api() {
        let schema = json!({
            "name": "test_tool",
            "description": "A test tool",
            "parameters": {
                "type": "object",
                "properties": {
                    "count": { "type": "integer" }
                }
            }
        });

        let out = ToolSchemaNormalizer::normalize(&schema, true);
        assert_eq!(out["type"], "function");
        assert_eq!(out["name"], "test_tool");
        assert_eq!(out["strict"], false);
        assert_eq!(out["parameters"]["type"], "object");
        assert!(out.get("function").is_none());
    }
}
