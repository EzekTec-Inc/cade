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
    fn select_tools(&self, messages: &[LlmMessage], tools: Vec<TaggedToolSchema>) -> Vec<Value>;
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
    fn select_tools(&self, messages: &[LlmMessage], tools: Vec<TaggedToolSchema>) -> Vec<Value> {
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
                    let is_core = tagged.tags.contains(&"core_mcp".to_string())
                        || tagged.tags.contains(&"meta".to_string())
                        || tagged.tags.contains(&"core".to_string());
                    let is_mcp = tagged.tags.contains(&"mcp".to_string());
                    if !is_mcp || is_core {
                        return true;
                    }
                    recently_used.contains(name)
                })
                .map(|tagged| {
                    let name = tagged.schema["name"].as_str().unwrap_or("").to_string();
                    let is_core = tagged.tags.contains(&"core_mcp".to_string())
                        || tagged.tags.contains(&"meta".to_string())
                        || tagged.tags.contains(&"core".to_string());
                    let is_mcp = tagged.tags.contains(&"mcp".to_string());
                    if !is_mcp || is_core || recently_used.contains(&name) {
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
    fn select_tools(&self, _messages: &[LlmMessage], tools: Vec<TaggedToolSchema>) -> Vec<Value> {
        tools.into_iter().map(|t| t.schema).collect()
    }
}

// ── Needle Tool Selector ──────────────────────────────────────────────────

use crate::needle::{NeedleConfig, NeedleEngine};
use std::sync::Arc;

pub struct NeedleToolSelector {
    pub engine: Arc<NeedleEngine>,
}

impl Default for NeedleToolSelector {
    fn default() -> Self {
        Self {
            engine: Arc::new(NeedleEngine::new(NeedleConfig::default())),
        }
    }
}

impl NeedleToolSelector {
    pub fn new(config: NeedleConfig) -> Self {
        Self {
            engine: Arc::new(NeedleEngine::new(config)),
        }
    }
}

impl IntelligentToolSelector for NeedleToolSelector {
    fn select_tools(&self, messages: &[LlmMessage], tools: Vec<TaggedToolSchema>) -> Vec<Value> {
        let latest_user_prompt = messages
            .iter()
            .rfind(|m| m.role == "user")
            .map(|m| m.content.as_str())
            .unwrap_or("");

        let mut core_tools = Vec::new();
        let mut optional_tools = Vec::new();

        for t in tools {
            if t.tags.iter().any(|tag| tag == "core_mcp" || tag == "cade") {
                core_tools.push(t.schema);
            } else {
                optional_tools.push(t.schema);
            }
        }

        let retrieved = self
            .engine
            .retrieve_top_k(latest_user_prompt, &optional_tools);
        let mut result = core_tools;
        result.extend(retrieved);
        result
    }
}

// ── Intent Tool Selector (Slice 1) ──────────────────────────────────────────

/// Zero-turn tool selector: prunes unrelated specialized tools from Turn 1
/// while guaranteeing core coding and system tools remain available.
pub struct IntentToolSelector {
    pub recent_window: usize,
    pub char_cap: usize,
}

impl Default for IntentToolSelector {
    fn default() -> Self {
        Self {
            recent_window: 15,
            char_cap: 80,
        }
    }
}

impl IntelligentToolSelector for IntentToolSelector {
    fn select_tools(&self, messages: &[LlmMessage], tools: Vec<TaggedToolSchema>) -> Vec<Value> {
        let mut conversation_text = String::new();
        let mut used_tools = HashSet::new();

        let start_idx = messages.len().saturating_sub(self.recent_window);
        for msg in &messages[start_idx..] {
            conversation_text.push_str(&msg.content.to_lowercase());
            conversation_text.push(' ');
            if let Some(calls) = &msg.tool_calls {
                for tc in calls {
                    used_tools.insert(tc.name.clone());
                }
            }
        }

        tools
            .into_iter()
            .filter(|tagged| {
                let name = tagged.schema["name"].as_str().unwrap_or("");
                let is_core = tagged.tags.iter().any(|t| {
                    t == "core" || t == "core_mcp" || t == "meta" || t == "native" || t == "cade"
                }) || is_essential_tool(name);

                if is_core {
                    return true;
                }

                if used_tools.contains(name) {
                    return true;
                }

                let name_lower = name.to_lowercase();
                let short_name = name_lower.split("__").last().unwrap_or(&name_lower);
                if conversation_text.contains(short_name) {
                    return true;
                }

                for tag in &tagged.tags {
                    if !tag.is_empty() && conversation_text.contains(&tag.to_lowercase()) {
                        return true;
                    }
                }

                false
            })
            .map(|tagged| tagged.schema)
            .collect()
    }
}

fn is_essential_tool(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "write_file"
            | "edit_file"
            | "replace_in_file"
            | "bash"
            | "glob"
            | "grep"
            | "update_memory"
            | "update_memory_typed"
            | "set_plan"
            | "UpdatePlan"
            | "finish_task"
            | "ask_user_question"
            | "run_subagent"
            | "cancel_subagent"
            | "list_agents"
            | "message_agent"
    )
}

// ── Resolver ─────────────────────────────────────────────────────────────────

/// Resolves the optimal tool selector based on the active model ID.
pub fn resolve_tool_selector(model_id: &str) -> Box<dyn IntelligentToolSelector> {
    if std::env::var("CADE_DISABLE_TOOL_PRUNING").is_ok() {
        Box::new(PassThroughToolSelector)
    } else if model_id.contains("needle") || std::env::var("CADE_USE_NEEDLE_ITS").is_ok() {
        Box::new(NeedleToolSelector::default())
    } else {
        Box::new(IntentToolSelector::default())
    }
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
    fn test_intent_selector_preserves_core_and_prunes_unrelated() {
        let selector = IntentToolSelector::default();
        let messages = vec![LlmMessage {
            role: "user".to_string(),
            content: "Please edit src/main.rs and run cargo test".to_string(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }];

        let tools = vec![
            TaggedToolSchema {
                schema: json!({"name": "write_file"}),
                tags: vec!["core".to_string()],
            },
            TaggedToolSchema {
                schema: json!({"name": "bash"}),
                tags: vec!["native".to_string()],
            },
            TaggedToolSchema {
                schema: json!({"name": "drawio__generate_diagram"}),
                tags: vec!["diagrams".to_string()],
            },
            TaggedToolSchema {
                schema: json!({"name": "pptx__generate_slides"}),
                tags: vec!["presentation".to_string()],
            },
        ];

        let selected = selector.select_tools(&messages, tools);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0]["name"], "write_file");
        assert_eq!(selected[1]["name"], "bash");
    }

    #[test]
    fn test_intent_selector_includes_matching_specialized_tool() {
        let selector = IntentToolSelector::default();
        let messages = vec![LlmMessage {
            role: "user".to_string(),
            content: "Create a presentation slides deck about architecture".to_string(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }];

        let tools = vec![
            TaggedToolSchema {
                schema: json!({"name": "bash"}),
                tags: vec!["core".to_string()],
            },
            TaggedToolSchema {
                schema: json!({"name": "pptx__generate_slides"}),
                tags: vec!["presentation".to_string()],
            },
            TaggedToolSchema {
                schema: json!({"name": "drawio__generate_diagram"}),
                tags: vec!["diagrams".to_string()],
            },
        ];

        let selected = selector.select_tools(&messages, tools);
        assert_eq!(selected.len(), 2);
        assert!(selected.iter().any(|s| s["name"] == "bash"));
        assert!(selected.iter().any(|s| s["name"] == "pptx__generate_slides"));
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

        let tools = vec![TaggedToolSchema {
            schema: json!({
                "name": "mcp_tool",
                "description": "This is a very long description that should be compressed"
            }),
            tags: vec!["mcp".to_string()],
        }];

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
            // Unused core_mcp tool (e.g. Serena) -> kept regardless of usage
            TaggedToolSchema {
                schema: json!({
                    "name": "serena__activate_project",
                    "description": "activate project in serena"
                }),
                tags: vec![
                    "cade".to_string(),
                    "mcp".to_string(),
                    "core_mcp".to_string(),
                ],
            },
        ];

        let selected = selector.select_tools(&messages, tools);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0]["name"], "used_mcp_tool");
        assert_eq!(selected[1]["name"], "bash");
        assert_eq!(selected[2]["name"], "serena__activate_project");
    }

    #[test]
    fn test_needle_tool_selector_pruning() {
        let selector = NeedleToolSelector::new(NeedleConfig {
            top_k_tools: 2,
            ..Default::default()
        });

        let messages = vec![LlmMessage {
            role: "user".to_string(),
            content: "read the config file from disk".to_string(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }];

        let tools = vec![
            TaggedToolSchema {
                schema: json!({ "name": "bash", "description": "execute command" }),
                tags: vec!["cade".to_string()],
            },
            TaggedToolSchema {
                schema: json!({ "name": "read_file", "description": "read file content" }),
                tags: vec!["mcp".to_string()],
            },
            TaggedToolSchema {
                schema: json!({ "name": "weather_api", "description": "get weather in city" }),
                tags: vec!["mcp".to_string()],
            },
            TaggedToolSchema {
                schema: json!({ "name": "music_player", "description": "play audio track" }),
                tags: vec!["mcp".to_string()],
            },
        ];

        let selected = selector.select_tools(&messages, tools);
        // bash is core (kept) + top-2 optional tools (read_file + 1 other)
        assert!(selected.iter().any(|t| t["name"] == "bash"));
        assert!(selected.iter().any(|t| t["name"] == "read_file"));
        assert!(selected.len() <= 3);
    }
}
