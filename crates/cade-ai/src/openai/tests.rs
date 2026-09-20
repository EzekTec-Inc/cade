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
    assert_eq!(
        v["properties"]["optional_str"]["type"].as_str(),
        Some("string")
    );
    assert_eq!(
        v["properties"]["optional_str"]["maxLength"].as_i64(),
        Some(10)
    );
}

#[test]
fn needs_max_completion_tokens_reasoning_models() {
    assert!(needs_max_completion_tokens("o1-preview"));
    assert!(needs_max_completion_tokens("o3-mini"));
    assert!(needs_max_completion_tokens("o4-mini"));
    assert!(needs_max_completion_tokens("gpt-4.5"));
    assert!(needs_max_completion_tokens("gpt-5"));
    assert!(needs_max_completion_tokens("gpt-5.5-pro"));
    assert!(needs_max_completion_tokens("gpt-5.6"));
    assert!(!needs_max_completion_tokens("gpt-4o"));
    assert!(!needs_max_completion_tokens("gpt-4o-mini"));
}

#[test]
fn parse_token_usage_supports_chat_completions_shape() {
    let usage = json!({
        "prompt_tokens": 100,
        "completion_tokens": 25,
        "prompt_tokens_details": { "cached_tokens": 10 }
    });

    let parsed = parse_token_usage(&usage, "gpt-4o").expect("usage should parse");
    assert_eq!(parsed.input_tokens, 90);
    assert_eq!(parsed.cache_read_tokens, 10);
    assert_eq!(parsed.output_tokens, 25);
    assert_eq!(parsed.model, "openai/gpt-4o");
}

#[test]
fn parse_token_usage_supports_responses_api_shape() {
    let usage = json!({
        "input_tokens": 100,
        "output_tokens": 25,
        "input_tokens_details": { "cached_tokens": 10 }
    });

    let parsed = parse_token_usage(&usage, "o3-mini").expect("usage should parse");
    assert_eq!(parsed.input_tokens, 90);
    assert_eq!(parsed.cache_read_tokens, 10);
    assert_eq!(parsed.output_tokens, 25);
    assert_eq!(parsed.model, "openai/o3-mini");
}

#[test]
fn is_o_series_identifies_frontier_reasoning_models() {
    assert!(is_o_series("openai/o1-mini"));
    assert!(is_o_series("openai/o3-mini"));
    assert!(is_o_series("openai/gpt-5.5-pro"));
    assert!(is_o_series("openai/gpt-5.6"));
    assert!(is_o_series("gpt-5"));
    assert!(!is_o_series("openai/gpt-4o"));
    assert!(!is_o_series("deepseek/deepseek-chat"));
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
                cache_control: None,
            },
            super::super::LlmMessage {
                role: "user".into(),
                content: "Hello".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
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
                cache_control: None,
            },
            super::super::LlmMessage {
                role: "user".into(),
                content: "Hello".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
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
            cache_control: None,
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

    for _name in ["update_memory", "update_memory_typed", "memory_apply_patch"] {}

    Ok(())
}

#[test]
fn build_tools_preserves_mixed_priority_and_core_mcp_tools_when_truncating() -> Result<()> {
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
        "name": "update_memory",
        "description": "Core memory tool",
        "parameters": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));

    tools.push(json!({
        "name": "serena__find_symbol",
        "description": "Configured core MCP tool",
        "parameters": {
            "type": "object",
            "properties": {},
            "required": []
        },
        "x-cade": {
            "kind": "mcp",
            "server_key": "serena",
            "core_server": true
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
            == Some("update_memory")),
        "should preserve update_memory"
    );
    assert!(
        arr.iter().any(|tool| tool
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|name| name.as_str())
            == Some("serena__find_symbol")),
        "should preserve metadata-marked serena__find_symbol"
    );

    Ok(())
}

#[test]
fn build_tools_preserves_configured_core_mcp_tools_when_truncating() -> Result<()> {
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

    for name in [
        "any-core-server__compress",
        "any-core-server__retrieve",
        "any-core-server__stats",
    ] {
        tools.push(json!({
            "name": name,
            "description": "Core MCP tool",
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            },
            "x-cade": {
                "kind": "mcp",
                "server_key": "any-core-server",
                "core_server": true
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
    for name in [
        "any-core-server__compress",
        "any-core-server__retrieve",
        "any-core-server__stats",
    ] {
        assert!(
            arr.iter().any(|tool| tool
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|name| name.as_str())
                == Some(name)),
            "build_tools should preserve {name} inside the 128-tool cap"
        );
    }

    Ok(())
}

#[test]
fn build_tools_preserves_tagged_and_metadata_tools_when_truncating() -> Result<()> {
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
        "name": "dynamic_metadata_tool",
        "description": "Tool with dynamic x-cade core_server metadata",
        "parameters": {
            "type": "object",
            "properties": {},
            "required": []
        },
        "x-cade": {
            "kind": "mcp",
            "core_server": true
        }
    }));

    tools.push(json!({
        "name": "legacy_db_tool",
        "description": "Legacy tool with DB core_mcp tag",
        "parameters": {
            "type": "object",
            "properties": {},
            "required": []
        },
        "tags": ["cade", "mcp", "core_mcp"]
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
            == Some("dynamic_metadata_tool")),
        "should preserve dynamic_metadata_tool via x-cade.core_server"
    );

    assert!(
        arr.iter().any(|tool| tool
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|name| name.as_str())
            == Some("legacy_db_tool")),
        "should preserve legacy tool with core_mcp tag"
    );

    Ok(())
}

#[test]
fn build_tools_preserves_finish_task_meta_tool_when_truncating() -> Result<()> {
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
        "name": "finish_task",
        "description": "Call this tool when you have completed a task.",
        "parameters": {
            "type": "object",
            "properties": {
                "summary": { "type": "string" },
                "reason": { "type": "string" }
            },
            "required": ["summary", "reason"]
        },
        "tags": ["cade", "meta"]
    }));

    let req = CompletionRequest {
        model: "gpt-5".into(),
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
            == Some("finish_task")),
        "build_tools should preserve finish_task because it is tagged as a meta tool"
    );

    Ok(())
}

#[test]
fn test_github_create_issue_openai_tool() {
    let raw_tool = json!({
        "description": "Create an issue",
        "name": "github-mcp-server__create_issue",
        "parameters": {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "properties": {
                "assignees": {
                    "items": { "type": "string" },
                    "nullable": true,
                    "type": "array"
                },
                "body": {
                    "nullable": true,
                    "type": "string"
                },
                "labels": {
                    "items": { "type": "string" },
                    "nullable": true,
                    "type": "array"
                },
                "owner": { "type": "string" },
                "repo": { "type": "string" },
                "title": { "type": "string" }
            },
            "required": [ "owner", "repo", "title" ],
            "title": "CreateIssueParams",
            "type": "object"
        }
    });

    let tool = OpenAiProvider::openai_tool_from_schema(&raw_tool);
    let params = &tool["function"]["parameters"];

    // 1. Top-level type MUST be "object"
    assert_eq!(params["type"], "object");

    // 2. Top-level MUST NOT have oneOf, anyOf, allOf, enum, const, not
    for key in ["oneOf", "anyOf", "allOf", "enum", "const", "not"] {
        assert!(params.get(key).is_none(), "top-level should not have {key}");
    }

    // 3. Property named "title" MUST be preserved
    assert!(
        params["properties"]["title"].is_object(),
        "title property must not be deleted"
    );
    assert_eq!(params["properties"]["title"]["type"], "string");

    // 4. Schema-level title "CreateIssueParams" MUST be stripped
    assert!(
        params.get("title").is_none(),
        "schema-level title must be stripped"
    );

    // 5. All required fields must exist in properties
    let props = params["properties"].as_object().unwrap();
    let req = params["required"].as_array().unwrap();
    for r in req {
        let name = r.as_str().unwrap();
        assert!(
            props.contains_key(name),
            "required field {name} must exist in properties"
        );
    }

    // 6. additionalProperties must be false
    assert_eq!(params["additionalProperties"], false);
}

// ── Responses API & Preview Gateway Routing Tests ─────────────────────────

#[test]
fn resolve_endpoint_uses_preview_base_url_for_frontier_models() {
    let provider = OpenAiProvider::new("test-key".into(), None);

    // Without override, routes to standard OPENAI_URL
    assert_eq!(
        provider.resolve_endpoint_with_preview("openai/gpt-4o", false, None),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        provider.resolve_endpoint_with_preview("openai/gpt-5.6", false, None),
        "https://api.openai.com/v1/chat/completions"
    );

    // With override, frontier models route to preview gateway, while gpt-4o stays on public
    let preview_gw = Some("https://preview-gateway.corp/v1");

    assert_eq!(
        provider.resolve_endpoint_with_preview("openai/gpt-4o", false, preview_gw),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        provider.resolve_endpoint_with_preview("openai/gpt-5.6", false, preview_gw),
        "https://preview-gateway.corp/v1/chat/completions"
    );
    assert_eq!(
        provider.resolve_endpoint_with_preview("openai/gpt-5.5-pro", true, preview_gw),
        "https://preview-gateway.corp/v1/responses"
    );
}

#[test]
fn resolve_endpoint_with_custom_proxy_base_url() {
    let proxy_provider =
        OpenAiProvider::new("test-key".into(), Some("http://127.0.0.1:8787/v1".into()));

    assert_eq!(
        proxy_provider.resolve_endpoint_with_preview("openai/gpt-4o", false, None),
        "http://127.0.0.1:8787/v1/chat/completions"
    );
    assert_eq!(
        proxy_provider.resolve_endpoint_with_preview("openai/gpt-5.6-terra", true, None),
        "http://127.0.0.1:8787/v1/responses"
    );
}

#[test]
fn gpt56_sol_with_tools_and_reasoning_uses_responses_api_shape() -> Result<()> {
    let provider = OpenAiProvider::new("test-key".into(), None);
    let req = CompletionRequest {
        model: "openai/gpt-5.6-sol".into(),
        messages: vec![crate::LlmMessage {
            role: "user".into(),
            content: "Use the tool".into(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }],
        tools: vec![json!({
            "name": "sample_tool",
            "description": "Sample tool",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }
        })],
        max_tokens: 4096,
        reasoning_effort: Some("high".into()),
    };

    assert!(requires_responses_api_for_tools_with_reasoning(&req));
    assert_eq!(
        provider.resolve_endpoint_for_request(&req),
        "https://api.openai.com/v1/responses"
    );

    let body = provider.build_body(&req, true);
    assert_eq!(body["model"], "gpt-5.6-sol");
    assert!(
        body.get("messages").is_none(),
        "Responses API should use input, not messages"
    );
    assert!(
        body.get("max_completion_tokens").is_none(),
        "Responses API should use max_output_tokens"
    );
    assert_eq!(
        body["input"]
            .as_array()
            .ok_or("input should be an array")?
            .len(),
        1
    );
    assert_eq!(body["max_output_tokens"], 4096);
    assert_eq!(body["reasoning"]["effort"], "high");
    assert!(
        body.get("reasoning_effort").is_none(),
        "Responses API should not send chat-completions reasoning_effort"
    );
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["name"], "sample_tool");
    assert!(
        body["tools"][0].get("function").is_none(),
        "Responses API uses flat function tools"
    );

    Ok(())
}

#[test]
fn gpt56_terra_with_tools_and_no_explicit_reasoning_uses_responses_api_shape() -> Result<()> {
    let provider = OpenAiProvider::new("test-key".into(), None);
    let req = CompletionRequest {
        model: "openai/gpt-5.6-terra".into(),
        messages: vec![crate::LlmMessage {
            role: "user".into(),
            content: "Use the tool".into(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }],
        tools: vec![json!({
            "name": "sample_tool",
            "description": "Sample tool",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }
        })],
        max_tokens: 4096,
        reasoning_effort: None,
    };

    assert!(requires_responses_api_for_tools_with_reasoning(&req));
    assert_eq!(
        provider.resolve_endpoint_for_request(&req),
        "https://api.openai.com/v1/responses"
    );

    let body = provider.build_body(&req, true);
    assert_eq!(body["model"], "gpt-5.6-terra");
    assert!(
        body.get("messages").is_none(),
        "Responses API should use input, not messages"
    );
    assert!(
        body.get("max_completion_tokens").is_none(),
        "Responses API should use max_output_tokens"
    );
    assert_eq!(
        body["input"]
            .as_array()
            .ok_or("input should be an array")?
            .len(),
        1
    );
    assert_eq!(body["max_output_tokens"], 4096);
    assert!(
        body.get("reasoning").is_none(),
        "Responses API should omit reasoning when reasoning_effort is None"
    );
    assert!(
        body.get("reasoning_effort").is_none(),
        "Responses API should not send chat-completions reasoning_effort"
    );
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["name"], "sample_tool");
    assert!(
        body["tools"][0].get("function").is_none(),
        "Responses API uses flat function tools"
    );

    Ok(())
}

#[test]
fn format_upstream_error_provides_diagnostic_guidance_for_preview_models() {
    let err_404 = OpenAiProvider::format_upstream_error(
        "OpenAI",
        reqwest::StatusCode::NOT_FOUND,
        "model_not_found: The model 'gpt-5.6' does not exist",
        "openai/gpt-5.6",
    );
    let msg = err_404.to_string();
    assert!(msg.contains("404 Not Found"));
    assert!(msg.contains("OPENAI_PREVIEW_BASE_URL"));

    let err_standard = OpenAiProvider::format_upstream_error(
        "OpenAI",
        reqwest::StatusCode::NOT_FOUND,
        "model_not_found: The model 'gpt-4o' does not exist",
        "openai/gpt-4o",
    );
    let standard_msg = err_standard.to_string();
    assert!(!standard_msg.contains("OPENAI_PREVIEW_BASE_URL"));
}

#[test]
fn to_openai_messages_never_emits_null_content() -> Result<()> {
    let req = CompletionRequest {
        model: "openai/gpt-5.5-2026-04-23".into(),
        messages: vec![
            crate::LlmMessage {
                role: "system".into(),
                content: "System prompt".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            crate::LlmMessage {
                role: "user".into(),
                content: "Run a command".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            // Assistant message after tool invocation with empty content (input[2])
            crate::LlmMessage {
                role: "assistant".into(),
                content: "".into(),
                tool_call_id: None,
                tool_calls: Some(vec![crate::LlmToolCall {
                    id: "call_123".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "ls"}),
                    thought_signature: None,
                }]),
                images: None,
                cache_control: None,
            },
            // Tool output message
            crate::LlmMessage {
                role: "tool".into(),
                content: "file1.txt\nfile2.txt".into(),
                tool_call_id: Some("call_123".into()),
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            // Assistant message with empty content and no tool calls
            crate::LlmMessage {
                role: "assistant".into(),
                content: "".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
        ],
        tools: vec![],
        max_tokens: 1000,
        reasoning_effort: None,
    };

    let messages = OpenAiProvider::to_openai_messages(&req);
    let arr = messages.as_array().ok_or("Should be an array")?;

    for (idx, msg) in arr.iter().enumerate() {
        let content = &msg["content"];
        assert!(
            !content.is_null(),
            "Message at index {idx} has null content, which OpenAI rejects with: 'Invalid type for input[{idx}].content: expected one of an array of objects or string, but got null instead.'"
        );
        assert!(
            content.is_string() || content.is_array(),
            "Message at index {idx} content must be string or array of objects, got {content:?}"
        );
    }

    // Also verify when serialized in Responses API body
    let provider = OpenAiProvider::new("test-key".into(), None);
    let body = provider.build_body(&req, false);
    if let Some(input) = body.get("input").and_then(|v| v.as_array()) {
        for (idx, item) in input.iter().enumerate() {
            let content = &item["content"];
            assert!(
                !content.is_null(),
                "Responses API input[{idx}].content must not be null"
            );
        }
    }

    Ok(())
}

#[test]
fn to_responses_input_serializes_valid_responses_api_schema() -> Result<()> {
    let req = CompletionRequest {
        model: "openai/gpt-5.5-2026-04-23".into(),
        messages: vec![
            crate::LlmMessage {
                role: "system".into(),
                content: "System prompt".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            crate::LlmMessage {
                role: "user".into(),
                content: "Run a command".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            // Assistant message that invoked tools
            crate::LlmMessage {
                role: "assistant".into(),
                content: "".into(),
                tool_call_id: None,
                tool_calls: Some(vec![crate::LlmToolCall {
                    id: "call_123".into(),
                    name: "bash".into(),
                    arguments: json!({"command": "ls"}),
                    thought_signature: None,
                }]),
                images: None,
                cache_control: None,
            },
            // Tool output message
            crate::LlmMessage {
                role: "tool".into(),
                content: "file1.txt\nfile2.txt".into(),
                tool_call_id: Some("call_123".into()),
                tool_calls: None,
                images: None,
                cache_control: None,
            },
            // User follow-up
            crate::LlmMessage {
                role: "user".into(),
                content: "Now analyze the files".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            },
        ],
        tools: vec![],
        max_tokens: 1000,
        reasoning_effort: None,
    };

    let input_val = OpenAiProvider::to_responses_input(&req);
    let items = input_val.as_array().ok_or("input should be an array")?;

    // Verify item 0: Developer/system message
    assert_eq!(items[0]["role"], "developer");
    assert_eq!(items[0]["content"], "System prompt");

    // Verify item 1: User message
    assert_eq!(items[1]["role"], "user");
    assert_eq!(items[1]["content"], "Run a command");

    // Verify item 2: Function call item (NOT assistant with tool_calls!)
    assert_eq!(items[2]["type"], "function_call");
    assert_eq!(items[2]["call_id"], "call_123");
    assert_eq!(items[2]["name"], "bash");
    assert_eq!(items[2]["arguments"], "{\"command\":\"ls\"}");
    assert!(
        items[2].get("tool_calls").is_none(),
        "Responses API items must not contain tool_calls"
    );

    // Verify item 3: Function call output item
    assert_eq!(items[3]["type"], "function_call_output");
    assert_eq!(items[3]["call_id"], "call_123");
    assert_eq!(items[3]["output"], "file1.txt\nfile2.txt");

    // Verify item 4: User follow-up
    assert_eq!(items[4]["role"], "user");
    assert_eq!(items[4]["content"], "Now analyze the files");

    // Crucial check: verify that NO item in input has a 'tool_calls' field
    for (idx, item) in items.iter().enumerate() {
        assert!(
            item.get("tool_calls").is_none(),
            "input[{idx}] contains 'tool_calls', which OpenAI /v1/responses rejects with 400 Bad Request"
        );
    }

    Ok(())
}

#[test]
fn build_tools_fair_round_robin_core_mcp_servers_under_cap() -> Result<()> {
    let mut tools = Vec::new();
    // 3 core servers with 50 tools each = 150 tools total (> 128)
    for i in 0..50 {
        tools.push(json!({
            "name": format!("serena__tool_{i}"),
            "description": "Serena AST tool",
            "parameters": { "type": "object", "properties": {}, "required": [] },
            "x-cade": {
                "kind": "mcp",
                "server_key": "serena",
                "core_server": true
            }
        }));
        tools.push(json!({
            "name": format!("desktop-commander__tool_{i}"),
            "description": "Desktop Commander tool",
            "parameters": { "type": "object", "properties": {}, "required": [] },
            "x-cade": {
                "kind": "mcp",
                "server_key": "desktop-commander",
                "core_server": true
            }
        }));
        tools.push(json!({
            "name": format!("github__tool_{i}"),
            "description": "GitHub tool",
            "parameters": { "type": "object", "properties": {}, "required": [] },
            "x-cade": {
                "kind": "mcp",
                "server_key": "github",
                "core_server": true
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
    assert_eq!(arr.len(), 128, "Total tools must be capped at 128");

    // Count tools per server
    let count_serena = arr
        .iter()
        .filter(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n.starts_with("serena__"))
                .unwrap_or(false)
        })
        .count();

    let count_desktop = arr
        .iter()
        .filter(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n.starts_with("desktop-commander__"))
                .unwrap_or(false)
        })
        .count();

    let count_github = arr
        .iter()
        .filter(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n.starts_with("github__"))
                .unwrap_or(false)
        })
        .count();

    // With 128 cap divided among 3 servers, each should get at least 42 tools (43 + 43 + 42 = 128)
    assert!(
        count_serena >= 42,
        "Serena must receive fair allocation, got {count_serena}"
    );
    assert!(
        count_desktop >= 42,
        "Desktop Commander must receive fair allocation, got {count_desktop}"
    );
    assert!(
        count_github >= 42,
        "GitHub must receive fair allocation, got {count_github}"
    );

    Ok(())
}

#[test]
fn build_tools_deterministic_invariance_under_shuffling() -> Result<()> {
    let mut tools = Vec::new();

    // Add meta tools
    for name in [
        "load_skill",
        "set_plan",
        "UpdatePlan",
        "search_memory",
        "update_memory",
    ] {
        tools.push(json!({
            "name": name,
            "description": "Meta tool",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }));
    }

    // Add core MCP tools
    for i in 0..60 {
        tools.push(json!({
            "name": format!("serena__cmd_{i:02}"),
            "description": "Serena command",
            "parameters": { "type": "object", "properties": {}, "required": [] },
            "x-cade": {
                "kind": "mcp",
                "server_key": "serena",
                "core_server": true
            }
        }));
        tools.push(json!({
            "name": format!("github__cmd_{i:02}"),
            "description": "GitHub command",
            "parameters": { "type": "object", "properties": {}, "required": [] },
            "x-cade": {
                "kind": "mcp",
                "server_key": "github",
                "core_server": true
            }
        }));
    }

    // Add non-core tools
    for i in 0..40 {
        tools.push(json!({
            "name": format!("misc_tool_{i:02}"),
            "description": "Misc non-core tool",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }));
    }

    let make_req = |t: Vec<Value>| CompletionRequest {
        model: "gpt-4o".into(),
        messages: vec![],
        tools: t,
        max_tokens: 4096,
        reasoning_effort: None,
    };

    let base_tools = OpenAiProvider::build_tools(&make_req(tools.clone()));
    let base_names: Vec<String> = base_tools
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(String::from)
        })
        .collect();

    // Permutation 1: Reversed order
    let mut reversed = tools.clone();
    reversed.reverse();
    let rev_tools = OpenAiProvider::build_tools(&make_req(reversed));
    let rev_names: Vec<String> = rev_tools
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(String::from)
        })
        .collect();
    assert_eq!(
        base_names, rev_names,
        "Reversed schema order must produce identical selection and ordering"
    );

    // Permutation 2: Interleaved odd/even order
    let mut interleaved = Vec::new();
    let mid = tools.len() / 2;
    for i in 0..mid {
        interleaved.push(tools[mid + i].clone());
        interleaved.push(tools[i].clone());
    }
    let int_tools = OpenAiProvider::build_tools(&make_req(interleaved));
    let int_names: Vec<String> = int_tools
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(String::from)
        })
        .collect();
    assert_eq!(
        base_names, int_names,
        "Interleaved schema order must produce identical selection and ordering"
    );

    Ok(())
}

#[test]
fn build_tools_preserves_plan_and_meta_tools_under_heavy_load() -> Result<()> {
    let mut tools = Vec::new();
    for i in 0..160 {
        tools.push(json!({
            "name": format!("competing_tool_{i}"),
            "description": "random",
            "parameters": { "type": "object", "properties": {}, "required": [] }
        }));
    }

    for name in [
        "set_plan",
        "UpdatePlan",
        "load_skill",
        "finish_task",
        "ask_user_question",
    ] {
        tools.push(json!({
            "name": name,
            "description": "Critical meta tool",
            "parameters": { "type": "object", "properties": {}, "required": [] }
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
    assert_eq!(arr.len(), 128);

    for name in [
        "set_plan",
        "UpdatePlan",
        "load_skill",
        "finish_task",
        "ask_user_question",
    ] {
        assert!(
            arr.iter().any(|t| t
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                == Some(name)),
            "{name} must survive 160 competing tools"
        );
    }

    Ok(())
}

#[test]
fn test_parse_token_usage_standard_and_responses_api() {
    // 1. Standard Chat Completions usage payload
    let chat_usage = json!({
        "prompt_tokens": 150,
        "completion_tokens": 75,
        "total_tokens": 225,
        "prompt_tokens_details": {
            "cached_tokens": 50
        }
    });
    let tu = parse_token_usage(&chat_usage, "openai/gpt-4o-2024-08-06")
        .expect("should parse chat usage");
    assert_eq!(tu.input_tokens, 100); // 150 - 50 cached
    assert_eq!(tu.output_tokens, 75);
    assert_eq!(tu.cache_read_tokens, 50);
    assert_eq!(tu.cache_write_tokens, 0);
    // Verified snapshot stripping in model name:
    assert_eq!(tu.model, "openai/gpt-4o");

    // 2. Responses API usage payload (uses input_tokens, output_tokens, and input_token_details)
    let responses_usage = json!({
        "input_tokens": 200,
        "output_tokens": 90,
        "input_token_details": {
            "cached_tokens": 80
        }
    });
    let tu_resp = parse_token_usage(&responses_usage, "gpt-5-2025-01-01")
        .expect("should parse responses API usage");
    assert_eq!(tu_resp.input_tokens, 120); // 200 - 80 cached
    assert_eq!(tu_resp.output_tokens, 90);
    assert_eq!(tu_resp.cache_read_tokens, 80);
    assert_eq!(tu_resp.model, "openai/gpt-5");
}

#[test]
fn test_parse_responses_api_multi_tool_stream() {
    use std::collections::BTreeMap;

    let mut tool_map: BTreeMap<usize, (String, String, String)> = BTreeMap::new();

    // 1. Tool 0 added: read_file
    let item_0 = json!({
        "type": "response.output_item.added",
        "output_index": 0,
        "item": {
            "type": "function_call",
            "call_id": "call_read_1",
            "name": "read_file",
            "arguments": ""
        }
    });
    let c1 = parse_responses_api_chunk(&item_0, &mut tool_map, "gpt-5");
    assert!(c1.is_empty());
    assert_eq!(tool_map.len(), 1);

    // 2. Tool 0 delta: arguments chunk 1
    let delta_1 = json!({
        "type": "response.function_call_arguments.delta",
        "output_index": 0,
        "delta": "{\"path\": \""
    });
    let c2 = parse_responses_api_chunk(&delta_1, &mut tool_map, "gpt-5");
    assert!(c2.is_empty());

    // 3. Tool 0 delta: arguments chunk 2
    let delta_2 = json!({
        "type": "response.function_call_arguments.delta",
        "output_index": 0,
        "delta": "src/lib.rs\"}"
    });
    let c3 = parse_responses_api_chunk(&delta_2, &mut tool_map, "gpt-5");
    assert!(c3.is_empty());

    // 4. Tool 0 done: yields StreamChunk::ToolCall with parsed arguments
    let done_0 = json!({
        "type": "response.output_item.done",
        "output_index": 0,
        "item": {
            "type": "function_call",
            "call_id": "call_read_1"
        }
    });
    let c4 = parse_responses_api_chunk(&done_0, &mut tool_map, "gpt-5");
    assert_eq!(c4.len(), 1);
    match &c4[0] {
        StreamChunk::ToolCall(tc) => {
            assert_eq!(tc.id, "call_read_1");
            assert_eq!(tc.name, "read_file");
            assert_eq!(tc.arguments["path"], "src/lib.rs");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    assert!(tool_map.is_empty());

    // 5. Tool 1 added: bash
    let item_1 = json!({
        "type": "response.output_item.added",
        "output_index": 1,
        "item": {
            "type": "function_call",
            "call_id": "call_bash_2",
            "name": "bash",
            "arguments": "{\"command\":\"cargo check\"}"
        }
    });
    let _ = parse_responses_api_chunk(&item_1, &mut tool_map, "gpt-5");

    let done_1 = json!({
        "type": "response.output_item.done",
        "output_index": 1,
        "item": {
            "type": "function_call",
            "call_id": "call_bash_2"
        }
    });
    let c5 = parse_responses_api_chunk(&done_1, &mut tool_map, "gpt-5");
    assert_eq!(c5.len(), 1);
    match &c5[0] {
        StreamChunk::ToolCall(tc) => {
            assert_eq!(tc.id, "call_bash_2");
            assert_eq!(tc.name, "bash");
            assert_eq!(tc.arguments["command"], "cargo check");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }

    // 6. response.done with status and usage
    let resp_done = json!({
        "type": "response.done",
        "response": {
            "status": "completed",
            "usage": {
                "input_tokens": 300,
                "output_tokens": 120,
                "input_token_details": {
                    "cached_tokens": 50
                }
            }
        }
    });
    let c6 = parse_responses_api_chunk(&resp_done, &mut tool_map, "gpt-5");
    assert_eq!(c6.len(), 2);
    assert!(matches!(&c6[0], StreamChunk::FinishReason(r) if r == "completed"));
    match &c6[1] {
        StreamChunk::Usage(u) => {
            assert_eq!(u.input_tokens, 250);
            assert_eq!(u.output_tokens, 120);
            assert_eq!(u.cache_read_tokens, 50);
            assert_eq!(u.model, "openai/gpt-5");
        }
        other => panic!("expected Usage, got {other:?}"),
    }
}
