#[allow(unused)]
type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

use super::*;

#[test]
fn clean_schema_adds_missing_properties() {
    let mut v = json!({"type": "object"});
    clean_openai_schema(&mut v);
    assert!(v.get("properties").is_some());
}

#[test]
fn clean_schema_does_not_overwrite_existing_properties() {
    let mut v = json!({"type": "object", "properties": {"foo": {"type": "string"}}});
    clean_openai_schema(&mut v);
    assert!(v["properties"]["foo"]["type"].as_str() == Some("string"));
}

#[test]
fn clean_schema_recurses_into_nested() {
    let mut v = json!({
        "type": "object",
        "properties": {
            "nested": {"type": "object"}
        }
    });
    clean_openai_schema(&mut v);
    // The nested object should also get an empty properties
    assert!(v["properties"]["nested"]["properties"].is_object());
}

#[test]
fn clean_schema_handles_arrays() {
    let mut v = json!([{"type": "object"}, {"type": "string"}]);
    clean_openai_schema(&mut v);
    assert!(v[0]["properties"].is_object());
}

#[test]
fn clean_schema_handles_type_array() {
    let mut v = json!({
        "type": "object",
        "properties": {
            "nullable_str": {
                "type": ["string", "null"]
            }
        }
    });
    clean_openai_schema(&mut v);
    assert_eq!(
        v["properties"]["nullable_str"]["type"],
        json!(["string", "null"])
    );
}

#[test]
fn clean_schema_ignores_null_schemas_in_anyof() {
    let mut v = json!({
        "type": "object",
        "properties": {
            "optional_str": {
                "anyOf": [
                    {"type": "null"},
                    {"type": "string", "maxLength": 10}
                ]
            }
        }
    });
    clean_openai_schema(&mut v);
    assert_eq!(v["properties"]["optional_str"]["type"].as_str(), Some("string"));
    assert_eq!(v["properties"]["optional_str"]["maxLength"].as_i64(), Some(10));
}

#[test]
fn needs_max_completion_tokens_reasoning_models() {
    assert!(needs_max_completion_tokens("o1-preview"));
    assert!(needs_max_completion_tokens("o3-mini"));
    assert!(needs_max_completion_tokens("o4-mini"));
    assert!(needs_max_completion_tokens("gpt-4.5"));
    assert!(needs_max_completion_tokens("gpt-5"));
    assert!(!needs_max_completion_tokens("gpt-4o"));
    assert!(!needs_max_completion_tokens("gpt-4o-mini"));
}



#[test]
fn parse_response_text_only() {
    let body = json!({
        "choices": [{
            "finish_reason": "stop",
            "message": {
                "content": "Hello, world!"
            }
        }]
    });
    let resp = OpenAiProvider::parse_response(&body);
    assert_eq!(resp.content.as_deref(), Some("Hello, world!"));
    assert!(resp.tool_calls.is_empty());
    assert_eq!(resp.finish_reason, "stop");
}

#[test]
fn parse_response_with_tool_calls() {
    let body = json!({
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "bash",
                        "arguments": "{\"command\":\"ls -la\"}"
                    }
                }]
            }
        }]
    });
    let resp = OpenAiProvider::parse_response(&body);
    assert!(resp.content.is_none());
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "bash");
    assert_eq!(resp.tool_calls[0].id, "call_123");
    assert_eq!(resp.tool_calls[0].arguments["command"], "ls -la");
}

#[test]
fn parse_response_empty_arguments_is_object() {
    let body = json!({
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "empty_tool",
                        "arguments": ""
                    }
                }]
            }
        }]
    });
    let resp = OpenAiProvider::parse_response(&body);
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "empty_tool");
    assert!(resp.tool_calls[0].arguments.is_object());
}







#[test]
fn to_openai_messages_basic() -> Result<()> {
    // -- Setup & Fixtures
    let req = CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![
            super::super::LlmMessage {
                role: "system".into(),
                content: "You are helpful.".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            },
            super::super::LlmMessage {
                role: "user".into(),
                content: "Hello".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            },
        ],
        tools: vec![],
        max_tokens: 4096,
        reasoning_effort: None,
    };
    // -- Exec
    let messages = OpenAiProvider::to_openai_messages(&req);

    // -- Check
    let arr = messages.as_array().ok_or("Should be an array")?;
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["role"], "system");
    assert_eq!(arr[1]["role"], "user");

    Ok(())
}

#[test]
fn build_tools_wraps_in_function_type() -> Result<()> {
    let req = CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![],
        tools: vec![json!({
            "name": "bash",
            "description": "Run a command",
            "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}
        })],
        max_tokens: 4096,
        reasoning_effort: None,
    };
    // -- Exec
    let tools = OpenAiProvider::build_tools(&req);

    // -- Check
    let arr = tools.as_array().ok_or("Should be an array")?;
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["type"], "function");
    assert_eq!(arr[0]["function"]["name"], "bash");

    Ok(())
}

#[test]
fn build_tools_disables_strict_structured_outputs() -> Result<()> {
    let req = CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![],
        tools: vec![json!({
            "name": "bash",
            "description": "Run a command",
            "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}
        })],
        max_tokens: 4096,
        reasoning_effort: None,
    };
    let tools = OpenAiProvider::build_tools(&req);
    let arr = tools.as_array().ok_or("Should be an array")?;
    assert_eq!(arr[0]["function"]["strict"], false);
    Ok(())
}



// ── o-series developer role remapping ─────────────────────────────────────

#[test]
fn o_series_system_maps_to_developer_role() -> Result<()> {
    let req = CompletionRequest {
        model: "openai/o3-mini".into(),
        messages: vec![
            super::super::LlmMessage {
                role: "system".into(),
                content: "You are helpful.".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            },
            super::super::LlmMessage {
                role: "user".into(),
                content: "Hello".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            },
        ],
        tools: vec![],
        max_tokens: 4096,
        reasoning_effort: None,
    };
    let messages = OpenAiProvider::to_openai_messages(&req);
    let arr = messages.as_array().ok_or("Should be an array")?;
    assert_eq!(
        arr[0]["role"], "developer",
        "system should map to developer for o-series"
    );
    assert_eq!(arr[1]["role"], "user");
    Ok(())
}

#[test]
fn non_o_series_preserves_system_role() -> Result<()> {
    let req = CompletionRequest {
        model: "gpt-4.1".into(),
        messages: vec![super::super::LlmMessage {
            role: "system".into(),
            content: "You are helpful.".into(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
        }],
        tools: vec![],
        max_tokens: 4096,
        reasoning_effort: None,
    };
    let messages = OpenAiProvider::to_openai_messages(&req);
    let arr = messages.as_array().ok_or("Should be an array")?;
    assert_eq!(arr[0]["role"], "system");
    Ok(())
}



// ── Tool param sealing (top-level additionalProperties only) ──────────────

#[test]
fn build_tools_seals_top_level_only() -> Result<()> {
    let req = CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![],
        tools: vec![json!({
            "name": "test_tool",
            "description": "test",
            "parameters": {
                "type": "object",
                "properties": {
                    "nested": {
                        "type": "object",
                        "properties": {
                            "inner": {"type": "string"}
                        }
                    },
                    "name": {"type": "string"}
                },
                "required": ["name"]
            }
        })],
        max_tokens: 4096,
        reasoning_effort: None,
    };
    let tools = OpenAiProvider::build_tools(&req);
    let arr = tools.as_array().ok_or("Should be an array")?;
    let params = &arr[0]["function"]["parameters"];
    // Top-level sealed
    assert_eq!(
        params["additionalProperties"],
        json!(false),
        "top-level params must have additionalProperties: false"
    );
    // Nested object NOT sealed — preserves loose MCP tool shape
    assert!(
        params["properties"]["nested"]
            .get("additionalProperties")
            .is_none(),
        "nested object must NOT have additionalProperties set"
    );
    Ok(())
}

#[test]
fn build_tools_truncates_to_128() -> Result<()> {
    let mut tools = Vec::new();
    for i in 0..200 {
        tools.push(json!({
            "name": format!("tool_{}", i),
            "description": "test",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }));
    }
    let req = CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![],
        tools,
        max_tokens: 4096,
        reasoning_effort: None,
    };

    let tools_val = OpenAiProvider::build_tools(&req);
    let arr = tools_val.as_array().ok_or("Should be an array")?;
    assert_eq!(arr.len(), 128, "build_tools should truncate to 128");

    
    

    Ok(())
}

#[test]
fn build_tools_preserves_load_skill_when_truncating() -> Result<()> {
    let mut tools = Vec::new();
    for i in 0..160 {
        tools.push(json!({
            "name": format!("tool_{}", i),
            "description": "test",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }));
    }
    tools.push(json!({
        "name": "load_skill",
        "description": "Load a skill",
        "parameters": {
            "type": "object",
            "properties": {
                "id": { "type": "string" }
            },
            "required": ["id"]
        }
    }));

    let req = CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![],
        tools,
        max_tokens: 4096,
        reasoning_effort: None,
    };

    let tools_val = OpenAiProvider::build_tools(&req);
    let arr = tools_val.as_array().ok_or("Should be an array")?;
    assert_eq!(arr.len(), 128, "build_tools should still cap at 128");
    assert!(
        arr.iter().any(|tool| tool
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|name| name.as_str())
            == Some("load_skill")),
        "build_tools should preserve load_skill inside the 128-tool cap"
    );

    
    
    

    Ok(())
}

#[test]
fn build_tools_preserves_memory_writing_tools_when_truncating() -> Result<()> {
    let mut tools = Vec::new();
    for i in 0..160 {
        tools.push(json!({
            "name": format!("tool_{}", i),
            "description": "test",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }));
    }

    for name in ["update_memory", "update_memory_typed", "memory_apply_patch"] {
        tools.push(json!({
            "name": name,
            "description": "Core memory tool",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }));
    }

    let req = CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![],
        tools,
        max_tokens: 4096,
        reasoning_effort: None,
    };

    let tools_val = OpenAiProvider::build_tools(&req);
    let arr = tools_val.as_array().ok_or("Should be an array")?;
    assert_eq!(arr.len(), 128, "build_tools should still cap at 128");
    for name in ["update_memory", "update_memory_typed", "memory_apply_patch"] {
        assert!(
            arr.iter().any(|tool| tool
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|name| name.as_str())
                == Some(name)),
            "build_tools should preserve {name} inside the 128-tool cap"
        );
    }

    
    
    for _name in ["update_memory", "update_memory_typed", "memory_apply_patch"] {
        
    }

    Ok(())
}

#[test]
fn build_tools_preserves_mixed_priority_and_prefixed_tools_when_truncating() -> Result<()> {
    let mut tools = Vec::new();
    for i in 0..160 {
        tools.push(json!({
            "name": format!("tool_{}", i),
            "description": "test",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }));
    }

    // Add a priority memory tool (flat schema)
    tools.push(json!({
        "name": "update_memory",
        "description": "Core memory tool",
        "parameters": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    // Add a priority prefix tool (flat schema)
    tools.push(json!({
        "name": "serena__find_symbol",
        "description": "Priority prefix tool",
        "parameters": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    let req = CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![],
        tools,
        max_tokens: 4096,
        reasoning_effort: None,
    };

    let tools_val = OpenAiProvider::build_tools(&req);
    let arr = tools_val.as_array().ok_or("Should be an array")?;
    assert_eq!(arr.len(), 128, "build_tools should still cap at 128");

    // Both "update_memory" and "serena__find_symbol" should be preserved!
    assert!(
        arr.iter().any(|tool| tool
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|name| name.as_str())
            == Some("update_memory")),
        "should preserve update_memory"
    );
    assert!(
        arr.iter().any(|tool| tool
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|name| name.as_str())
            == Some("serena__find_symbol")),
        "should preserve serena__find_symbol"
    );

    Ok(())
}
