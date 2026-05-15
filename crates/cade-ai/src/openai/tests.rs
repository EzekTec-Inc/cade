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
fn needs_responses_api_check() {
    assert!(needs_responses_api("gpt-5"));
    assert!(needs_responses_api("o1-pro"));
    assert!(needs_responses_api("o3-pro"));
    assert!(!needs_responses_api("gpt-4o"));
    assert!(!needs_responses_api("o3-mini"));
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
fn parse_responses_api_text() {
    let body = json!({
        "output": [{
            "type": "message",
            "content": [{
                "type": "output_text",
                "text": "Response text"
            }]
        }]
    });
    let resp = OpenAiProvider::parse_responses_response(&body);
    assert_eq!(resp.content.as_deref(), Some("Response text"));
    assert!(resp.tool_calls.is_empty());
}

#[test]
fn parse_responses_api_function_call() {
    let body = json!({
        "output": [{
            "type": "function_call",
            "name": "read_file",
            "call_id": "fc_456",
            "arguments": "{\"path\":\"src/main.rs\"}"
        }]
    });
    let resp = OpenAiProvider::parse_responses_response(&body);
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "read_file");
    assert_eq!(resp.tool_calls[0].id, "fc_456");
    assert_eq!(resp.finish_reason, "tool_calls");
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

#[test]
fn responses_api_o_series_maps_system_to_developer() -> Result<()> {
    let req = CompletionRequest {
        model: "openai/o4-mini".into(),
        messages: vec![super::super::LlmMessage {
            role: "system".into(),
            content: "Instructions.".into(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
        }],
        tools: vec![],
        max_tokens: 4096,
        reasoning_effort: None,
    };
    let input = OpenAiProvider::to_responses_input(&req);
    let arr = input.as_array().ok_or("Should be an array")?;
    assert_eq!(arr[0]["role"], "developer");
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

    let resp_tools_val = OpenAiProvider::build_responses_tools(&req);
    let arr2 = resp_tools_val.as_array().ok_or("Should be an array")?;
    assert_eq!(
        arr2.len(),
        128,
        "build_responses_tools should truncate to 128"
    );

    Ok(())
}
