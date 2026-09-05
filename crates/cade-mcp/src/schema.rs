//! Tool Schema Normalization & Adaptation.
//!
//! Translates raw MCP tool definitions and input schemas into OpenAI / Anthropic
//! compatible parameter schemas.

// region:    --- Imports

use rmcp::model::Tool;
use serde_json::{Value, json};
use std::collections::HashSet;

// endregion: --- Imports

// region:    --- Types

/// A normalized and cached tool schema ready for LLM consumption.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpToolSchema {
    pub server_key: String,
    pub prefixed_name: String,
    pub original_name: String,
    pub schema: Value, // OpenAI-compatible: { name, description, parameters }
    /// If true, calling this tool requires user permission.
    pub is_write: bool,
}

// endregion: --- Types

// region:    --- Normalizer

/// Deep module normalizing diverse MCP tool schemas into standard provider formats.
pub struct ToolSchemaNormalizer;

impl ToolSchemaNormalizer {
    /// Normalize a single MCP tool definition.
    pub fn normalize(server_key: &str, tool: &Tool, write_tools: &[String]) -> McpToolSchema {
        let original = tool.name.to_string();
        let prefixed = format!("{server_key}__{original}");
        let description = tool.description.as_deref().unwrap_or("").to_string();

        // Convert MCP input_schema to OpenAI parameters Value
        let mut parameters = Value::Object((*tool.input_schema).clone());

        // OpenAI strictly requires parameters to have "type": "object" and "properties" even if empty.
        if let Some(obj) = parameters.as_object_mut() {
            if !obj.contains_key("type") {
                obj.insert("type".to_string(), json!("object"));
            }
            if obj.get("type").and_then(|t| t.as_str()) == Some("object")
                && !obj.contains_key("properties")
            {
                obj.insert("properties".to_string(), json!({}));
            }
        }

        // Infer write tool:
        // 1. Explicit config.write_tools list (if non-empty -> whitelist mode)
        // 2. If list is empty -> default: all tools are write (conservative)
        // 3. Check ToolAnnotations.readOnlyHint if available
        let write_set: HashSet<&str> = write_tools.iter().map(|s| s.as_str()).collect();
        let is_write = if !write_tools.is_empty() {
            write_set.contains(original.as_str())
        } else if let Some(annotations) = &tool.annotations {
            !annotations.read_only_hint.unwrap_or(false)
        } else {
            true
        };

        let schema = json!({
            "name": prefixed,
            "description": description,
            "parameters": parameters,
        });

        McpToolSchema {
            server_key: server_key.to_string(),
            prefixed_name: prefixed,
            original_name: original,
            schema,
            is_write,
        }
    }
}

// endregion: --- Normalizer

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::JsonObject;

    #[test]
    fn test_normalize_schema_adds_missing_properties() {
        let mut input_schema = JsonObject::new();
        input_schema.insert("type".to_string(), json!("object"));

        let tool = Tool::new("test_tool", "A test tool", input_schema);

        let normalized = ToolSchemaNormalizer::normalize("my_server", &tool, &["test_tool".into()]);
        assert_eq!(normalized.prefixed_name, "my_server__test_tool");
        assert_eq!(normalized.original_name, "test_tool");
        assert!(normalized.is_write);

        let params = normalized
            .schema
            .get("parameters")
            .and_then(|p| p.as_object());
        assert!(params.is_some());
        if let Some(p) = params {
            assert_eq!(p.get("type"), Some(&json!("object")));
            assert_eq!(p.get("properties"), Some(&json!({})));
        }
    }

    #[test]
    fn test_whitelist_non_write_tool() {
        let tool = Tool::new("read_query", "Reads data", JsonObject::new());

        let normalized = ToolSchemaNormalizer::normalize("db", &tool, &["write_data".into()]);
        assert!(!normalized.is_write);
    }
}

// endregion: --- Tests
