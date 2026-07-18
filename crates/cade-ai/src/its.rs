use crate::LlmMessage;
use serde_json::Value;
use std::collections::HashSet;

/// A lightweight representation of a tool schema and its db tags.
#[derive(Debug, Clone)]
pub struct TaggedToolSchema {
    pub schema: Value,
    pub tags: Vec<String>,
}

/// Polymorphic interface for intelligent tool selection (pruning and compression).
pub trait IntelligentToolSelector: Send + Sync {
    /// Selects, prunes, and compresses tool schemas based on the active conversation context.
    fn select_tools(
        &self,
        messages: &[LlmMessage],
        tools: Vec<TaggedToolSchema>,
    ) -> Vec<Value>;
}

// ── Adaptive Tool Selector ──────────────────────────────────────────────────

pub struct AdaptiveToolSelector {
    pub recent_window: usize,
    pub char_cap: usize,
}

impl Default for AdaptiveToolSelector {
    fn default() -> Self {
        Self {
            recent_window: 20,
            char_cap: 80,
        }
    }
}

impl AdaptiveToolSelector {
    /// Compresses a single tool schema by truncating top-level descriptions
    /// and stripping per-property comments.
    pub fn compress_tool_schema(&self, mut schema: Value) -> Value {
        if let Some(desc) = schema.get("description").and_then(|v| v.as_str()) {
            let trimmed: String = desc
                .split('\n')
                .next()
                .unwrap_or(desc)
                .chars()
                .take(self.char_cap)
                .collect();
            schema["description"] = Value::String(trimmed);
        }

        for params_key in ["parameters", "input_schema"] {
            if let Some(params) = schema.get_mut(params_key)
                && let Some(props) = params.get_mut("properties")
                && let Some(obj) = props.as_object_mut()
            {
                for (_, prop_val) in obj.iter_mut() {
                    if let Some(prop_obj) = prop_val.as_object_mut() {
                        prop_obj.remove("description");
                        prop_obj.remove("examples");
                    }
                }
            }
        }

        schema
    }
}

impl IntelligentToolSelector for AdaptiveToolSelector {
    fn select_tools(
        &self,
        messages: &[LlmMessage],
        tools: Vec<TaggedToolSchema>,
    ) -> Vec<Value> {
        let is_long_session = messages.len() > 1 + self.recent_window;

        let recently_used: HashSet<String> = if is_long_session {
            let recent_start = messages.len().saturating_sub(self.recent_window);
            messages[recent_start..]
                .iter()
                .filter_map(|m| m.tool_calls.as_ref())
                .flat_map(|calls| calls.iter().map(|tc| tc.name.clone()))
                .collect()
        } else {
            HashSet::new()
        };

        if is_long_session {
            tools
                .into_iter()
                .filter(|tagged| {
                    let name = tagged.schema["name"].as_str().unwrap_or("");
                    let is_mcp = tagged.tags.contains(&"mcp".to_string());
                    if !is_mcp {
                        return true;
                    }
                    recently_used.contains(name)
                })
                .map(|tagged| {
                    let name = tagged.schema["name"].as_str().unwrap_or("").to_string();
                    let is_mcp = tagged.tags.contains(&"mcp".to_string());
                    if !is_mcp || recently_used.contains(&name) {
                        tagged.schema
                    } else {
                        self.compress_tool_schema(tagged.schema)
                    }
                })
                .collect()
        } else {
            tools.into_iter().map(|t| t.schema).collect()
        }
    }
}

// ── Pass-Through Tool Selector ───────────────────────────────────────────────

pub struct PassThroughToolSelector;

impl IntelligentToolSelector for PassThroughToolSelector {
    fn select_tools(
        &self,
        _messages: &[LlmMessage],
        tools: Vec<TaggedToolSchema>,
    ) -> Vec<Value> {
        tools.into_iter().map(|t| t.schema).collect()
    }
}

// ── Resolver ─────────────────────────────────────────────────────────────────

/// Resolves the optimal tool selector based on the active model ID.
pub fn resolve_tool_selector(_model_id: &str) -> Box<dyn IntelligentToolSelector> {
    // Standard model resolution: defaults to AdaptiveToolSelector.
    // Highly extensible for models that need different selection metrics.
    Box::new(AdaptiveToolSelector::default())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LlmMessage, LlmToolCall};
    use serde_json::json;

    #[test]
    fn test_pass_through_selector() {
        let selector = PassThroughToolSelector;
        let messages = vec![];
        let tools = vec![
            TaggedToolSchema {
                schema: json!({"name": "bash"}),
                tags: vec!["cade".to_string()],
            },
            TaggedToolSchema {
                schema: json!({"name": "mcp_tool"}),
                tags: vec!["mcp".to_string()],
            },
        ];

        let selected = selector.select_tools(&messages, tools);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0]["name"], "bash");
        assert_eq!(selected[1]["name"], "mcp_tool");
    }

    #[test]
    fn test_adaptive_selector_short_session() {
        let selector = AdaptiveToolSelector {
            recent_window: 5,
            char_cap: 10,
        };
        // 3 messages is less than 1 + recent_window (6) -> short session, no pruning
        let messages = vec![
            LlmMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            LlmMessage {
                role: "assistant".to_string(),
                content: "Hi".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
        ];

        let tools = vec![
            TaggedToolSchema {
                schema: json!({
                    "name": "mcp_tool",
                    "description": "This is a very long description that should be compressed"
                }),
                tags: vec!["mcp".to_string()],
            },
        ];

        let selected = selector.select_tools(&messages, tools);
        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0]["description"],
            "This is a very long description that should be compressed"
        );
    }

    #[test]
    fn test_adaptive_selector_long_session_pruning() {
        let selector = AdaptiveToolSelector {
            recent_window: 2,
            char_cap: 10,
        };
        // 4 messages is > 1 + recent_window (3) -> long session, unused MCP tool should be pruned
        let messages = vec![
            LlmMessage {
                role: "user".to_string(),
                content: "Hello".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            LlmMessage {
                role: "assistant".to_string(),
                content: "Hi".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            LlmMessage {
                role: "user".to_string(),
                content: "Next".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            LlmMessage {
                role: "assistant".to_string(),
                content: "".to_string(),
                tool_call_id: None,
                tool_calls: Some(vec![LlmToolCall {
                    id: "call_1".to_string(),
                    name: "used_mcp_tool".to_string(),
                    arguments: json!({}),
                    thought_signature: None,
                }]),
                images: None,
                cache_control: None,
            },
        ];

        let tools = vec![
            // Unused MCP tool -> pruned
            TaggedToolSchema {
                schema: json!({
                    "name": "unused_mcp_tool",
                    "description": "some description"
                }),
                tags: vec!["mcp".to_string()],
            },
            // Used MCP tool -> kept
            TaggedToolSchema {
                schema: json!({
                    "name": "used_mcp_tool",
                    "description": "some description"
                }),
                tags: vec!["mcp".to_string()],
            },
            // Core CADE tool -> kept regardless of usage
            TaggedToolSchema {
                schema: json!({
                    "name": "bash",
                    "description": "execute shell"
                }),
                tags: vec!["cade".to_string()],
            },
        ];

        let selected = selector.select_tools(&messages, tools);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0]["name"], "used_mcp_tool");
        assert_eq!(selected[1]["name"], "bash");
    }
}
