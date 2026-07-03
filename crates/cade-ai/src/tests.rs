#[allow(unused)]
type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

use super::*;
use serde_json::json;

// -- bare_model

#[test]
fn bare_model_strips_provider_prefix() {
    assert_eq!(
        bare_model("anthropic/claude-sonnet-4-5-20250929"),
        "claude-sonnet-4-5-20250929"
    );
    assert_eq!(bare_model("openai/gpt-4o"), "gpt-4o");
    assert_eq!(bare_model("gemini/gemini-2.5-pro"), "gemini-2.5-pro");
}

#[test]
fn bare_model_no_prefix_unchanged() {
    assert_eq!(
        bare_model("claude-sonnet-4-5-20250929"),
        "claude-sonnet-4-5-20250929"
    );
    assert_eq!(bare_model("gpt-4o"), "gpt-4o");
}

// -- is_retryable_status

#[test]
fn retryable_statuses() {
    assert!(is_retryable_status(reqwest::StatusCode::TOO_MANY_REQUESTS)); // 429
    assert!(is_retryable_status(
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    )); // 500
    assert!(is_retryable_status(reqwest::StatusCode::BAD_GATEWAY)); // 502
    assert!(is_retryable_status(
        reqwest::StatusCode::SERVICE_UNAVAILABLE
    )); // 503
    assert!(is_retryable_status(reqwest::StatusCode::GATEWAY_TIMEOUT)); // 504
}

#[test]
fn non_retryable_statuses() {
    assert!(!is_retryable_status(reqwest::StatusCode::BAD_REQUEST)); // 400
    assert!(!is_retryable_status(reqwest::StatusCode::UNAUTHORIZED)); // 401
    assert!(!is_retryable_status(reqwest::StatusCode::FORBIDDEN)); // 403
    assert!(!is_retryable_status(reqwest::StatusCode::NOT_FOUND)); // 404
    assert!(!is_retryable_status(reqwest::StatusCode::OK)); // 200
}

// -- provider_error

#[test]
fn provider_error_extracts_json_message() {
    let body = r#"{"error":{"message":"Rate limit exceeded"}}"#;
    let err = provider_error("Anthropic", reqwest::StatusCode::TOO_MANY_REQUESTS, body);
    let msg = err.to_string();
    assert!(msg.contains("Anthropic"), "got: {msg}");
    assert!(msg.contains("429"), "got: {msg}");
    assert!(msg.contains("Rate limit exceeded"), "got: {msg}");
}

#[test]
fn provider_error_falls_back_to_raw_body() {
    let body = "Something went wrong";
    let err = provider_error("OpenAI", reqwest::StatusCode::INTERNAL_SERVER_ERROR, body);
    let msg = err.to_string();
    assert!(msg.contains("Something went wrong"), "got: {msg}");
}

// -- is_retryable_error

#[test]
fn retryable_error_from_provider_error() {
    let err = provider_error(
        "Anthropic",
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "rate limited",
    );
    assert!(is_retryable_error(&err));
}

#[test]
fn non_retryable_error() {
    let err = crate::Error::custom("Invalid API key (401)");
    // Contains "401" which is not in the retryable list... but wait,
    // the check is naive substring. Let's verify:
    // 401 is NOT in ["429", "500", "502", "503", "504"]
    assert!(!is_retryable_error(&err));
}

// -- infer_provider_prefix

#[test]
fn infer_claude() {
    assert_eq!(
        infer_provider_prefix("claude-sonnet-4-5-20250929"),
        Some("anthropic")
    );
    assert_eq!(
        infer_provider_prefix("claude-3-opus-20240229"),
        Some("anthropic")
    );
}

#[test]
fn infer_gpt() {
    assert_eq!(infer_provider_prefix("gpt-4o"), Some("openai"));
    assert_eq!(infer_provider_prefix("gpt-4o-mini"), Some("openai"));
    assert_eq!(infer_provider_prefix("o3-mini"), Some("openai"));
    assert_eq!(infer_provider_prefix("o4-mini"), Some("openai"));
}

#[test]
fn infer_gemini() {
    assert_eq!(infer_provider_prefix("gemini-2.5-pro"), Some("gemini"));
}

#[test]
fn infer_ollama_models() {
    assert_eq!(infer_provider_prefix("llama-3-70b"), Some("ollama"));
    assert_eq!(infer_provider_prefix("mistral-large"), Some("ollama"));
    assert_eq!(infer_provider_prefix("phi-3"), Some("ollama"));
    assert_eq!(infer_provider_prefix("qwen-2"), Some("ollama"));
    assert_eq!(infer_provider_prefix("deepseek-coder"), Some("ollama"));
}

#[test]
fn infer_unknown_returns_none() {
    assert_eq!(infer_provider_prefix("some-custom-model"), None);
}

// -- LlmRouter::build

#[test]
fn router_build_with_no_keys() {
    let config = AiConfig {
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "ollama".into(),
    };
    let router = LlmRouter::build(&config);
    // Ollama should always be present
    assert!(router.provider_names().contains(&"ollama".to_string()));
}

#[test]
fn router_build_with_anthropic_key() {
    let config = AiConfig {
        anthropic_api_key: Some("sk-test".into()),
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "anthropic".into(),
    };
    let router = LlmRouter::build(&config);
    assert!(router.provider_names().contains(&"anthropic".to_string()));
}

#[test]
fn router_resolve_explicit_prefix() -> Result<()> {
    // -- Setup & Fixtures
    let config = AiConfig {
        anthropic_api_key: Some("sk-test".into()),
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "anthropic".into(),
    };
    let router = LlmRouter::build(&config);

    // -- Exec & Check
    let (_, bare) = router.resolve_provider("anthropic/claude-sonnet-4-5-20250929")?;
    assert_eq!(bare, "claude-sonnet-4-5-20250929");

    Ok(())
}

#[test]
fn router_resolve_inferred_prefix() -> Result<()> {
    // -- Setup & Fixtures
    let config = AiConfig {
        anthropic_api_key: Some("sk-test".into()),
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "anthropic".into(),
    };
    let router = LlmRouter::build(&config);

    // -- Exec & Check
    let (_, bare) = router.resolve_provider("claude-sonnet-4-5-20250929")?;
    assert_eq!(bare, "claude-sonnet-4-5-20250929");

    Ok(())
}

// -- VcrCassette Tests

#[test]
fn test_vcr_redact_secrets() {
    use crate::vcr::redact_secrets;

    let raw_anthropic = "api_key=sk-ant-sid01-someletters1234567890-XYZ123456";
    let redacted_anthropic = redact_secrets(raw_anthropic);
    assert_eq!(redacted_anthropic, "api_key=[REDACTED_ANTHROPIC_KEY]");

    let raw_openai = "api_key=sk-openai123456789012345678901234567890";
    let redacted_openai = redact_secrets(raw_openai);
    assert_eq!(redacted_openai, "api_key=[REDACTED_OPENAI_KEY]");

    let raw_bearer = "Authorization: Bearer 12345.abcde.XYZ";
    let redacted_bearer = redact_secrets(raw_bearer);
    assert_eq!(
        redacted_bearer,
        "Authorization: Bearer [REDACTED_BEARER_TOKEN]"
    );
}

#[test]
fn test_vcr_cassette_replay() -> Result<()> {
    use crate::vcr::{HttpInteraction, VcrCassette, VcrMode};
    use std::io::Write;
    use tempfile::NamedTempFile;

    let interactions = vec![HttpInteraction {
        url: "https://api.openai.com/v1/chat/completions".to_string(),
        method: "POST".to_string(),
        request_body: "{\"prompt\":\"test\"}".to_string(),
        response_status: 200,
        response_body: "{\"text\":\"hello\"}".to_string(),
    }];

    let mut temp_file = NamedTempFile::new()?;
    let content = serde_json::to_string_pretty(&interactions)?;
    temp_file.write_all(content.as_bytes())?;

    let cassette = VcrCassette::new(temp_file.path().to_path_buf(), VcrMode::Replay)?;
    let matched = cassette.match_response(
        "https://api.openai.com/v1/chat/completions",
        "POST",
        "{\"prompt\":\"test\"}",
    );

    assert!(matched.is_some());
    let interaction = matched.unwrap();
    assert_eq!(interaction.response_body, "{\"text\":\"hello\"}");
    assert_eq!(interaction.response_status, 200);

    Ok(())
}

// -- build_standard_http_client Tests

#[test]
fn test_build_standard_http_client_is_configured() {
    let client = build_standard_http_client();
    // Verify client builds and can be used
    let req = client.get("https://httpbin.org/get").build();
    assert!(req.is_ok());
}

// -- Observability Tests

#[test]
fn test_gen_ai_observability_fields() {
    use crate::observability::semconv::*;
    assert_eq!(GEN_AI_SYSTEM, "gen_ai.system");
    assert_eq!(GEN_AI_REQUEST_MODEL, "gen_ai.request.model");
    assert_eq!(GEN_AI_REQUEST_MAX_TOKENS, "gen_ai.request.max_tokens");
    assert_eq!(GEN_AI_RESPONSE_MODEL, "gen_ai.response.model");
    assert_eq!(GEN_AI_USAGE_INPUT_TOKENS, "gen_ai.response.tokens.input");
    assert_eq!(GEN_AI_USAGE_OUTPUT_TOKENS, "gen_ai.response.tokens.output");
}

#[test]
fn router_resolve_unknown_falls_back_to_default() {
    let config = AiConfig {
        anthropic_api_key: Some("sk-test".into()),
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "anthropic".into(),
    };
    let router = LlmRouter::build(&config);
    // "some-random-model" doesn't match any provider prefix → default
    let result = router.resolve_provider("some-random-model");
    assert!(result.is_ok());
}

#[test]
fn router_resolve_missing_provider_errors() -> Result<()> {
    let config = AiConfig {
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "ollama".into(),
    };
    let router = LlmRouter::build(&config);
    // anthropic/ prefix but no anthropic provider configured
    let result = router.resolve_provider("anthropic/claude-sonnet-4-5");
    let msg = result.err().ok_or("Should be an error")?.to_string();
    assert!(msg.contains("not configured"), "got: {msg}");

    Ok(())
}

#[test]
fn router_add_and_remove_provider() {
    let config = AiConfig {
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "ollama".into(),
    };
    let mut router = LlmRouter::build(&config);
    assert!(
        !router
            .provider_names()
            .contains(&"test-provider".to_string())
    );

    // Add a provider
    let provider: Arc<dyn LlmProvider> = Arc::new(crate::ollama::OllamaProvider::new(
        "http://localhost:11434".into(),
    ));
    router.add_provider("test-provider".into(), provider);
    assert!(
        router
            .provider_names()
            .contains(&"test-provider".to_string())
    );

    // Remove it
    assert!(router.remove_provider("test-provider"));
    assert!(
        !router
            .provider_names()
            .contains(&"test-provider".to_string())
    );

    // Removing again returns false
    assert!(!router.remove_provider("test-provider"));
}

#[test]
fn router_validate_model() {
    let config = AiConfig {
        anthropic_api_key: Some("sk-test".into()),
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "anthropic".into(),
    };
    let router = LlmRouter::build(&config);
    assert!(router.validate_model("anthropic/claude-sonnet-4-5").is_ok());
    assert!(router.validate_model("openai/gpt-4o").is_err()); // openai not configured
}

// -- LlmMessage serialization

#[test]
fn llm_message_roundtrip() -> Result<()> {
    // -- Setup & Fixtures
    let msg = LlmMessage {
        role: "user".into(),
        content: "Hello".into(),
        tool_call_id: None,
        tool_calls: None,
        images: None,
    };

    // -- Exec
    let json = serde_json::to_string(&msg)?;
    let parsed: LlmMessage = serde_json::from_str(&json)?;

    // -- Check
    assert_eq!(parsed.role, "user");
    assert_eq!(parsed.content, "Hello");

    Ok(())
}

#[test]
fn llm_tool_call_roundtrip() -> Result<()> {
    // -- Setup & Fixtures
    let tc = LlmToolCall {
        id: "tc_123".into(),
        name: "bash".into(),
        arguments: json!({"command": "ls"}),
        thought_signature: None,
    };

    // -- Exec
    let json = serde_json::to_string(&tc)?;
    let parsed: LlmToolCall = serde_json::from_str(&json)?;

    // -- Check
    assert_eq!(parsed.id, "tc_123");
    assert_eq!(parsed.name, "bash");

    Ok(())
}

// -- TokenUsage defaults

#[test]
fn token_usage_default() {
    let u = TokenUsage::default();
    assert_eq!(u.input_tokens, 0);
    assert_eq!(u.output_tokens, 0);
    assert_eq!(u.cache_read_tokens, 0);
    assert_eq!(u.cache_write_tokens, 0);
    assert!(u.model.is_empty());
}

// -- openai_compat_presets

#[test]
fn compat_presets_non_empty() {
    let presets = openai_compat_presets();
    assert!(!presets.is_empty());
    // All should have a name and URL
    for (name, url) in &presets {
        assert!(!name.is_empty());
        assert!(url.starts_with("https://"), "bad url for {name}: {url}");
    }
}

// -- provider_from_row

#[test]
fn provider_from_row_anthropic() {
    let config = AiConfig {
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "ollama".into(),
    };
    // With explicit key
    let p = LlmRouter::provider_from_row("anthropic", Some("sk-test".into()), None, &config);
    assert!(p.is_some());

    // Without key and no config key → None
    let p = LlmRouter::provider_from_row("anthropic", None, None, &config);
    assert!(p.is_none());
}

#[test]
fn provider_from_row_ollama() {
    let config = AiConfig {
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "ollama".into(),
    };
    let p = LlmRouter::provider_from_row("ollama", None, None, &config);
    assert!(p.is_some());
}

#[test]
fn provider_from_row_unknown() {
    let config = AiConfig {
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "ollama".into(),
    };
    let p = LlmRouter::provider_from_row("unknown-provider", None, None, &config);
    assert!(p.is_none());
}

// -- inline_schema_refs

#[test]
fn inline_schema_refs_resolves_single_ref() {
    let mut schema = json!({
        "$defs": {
            "Foo": {
                "type": "object",
                "properties": { "bar": { "type": "string" } }
            }
        },
        "type": "object",
        "properties": {
            "item": { "$ref": "#/$defs/Foo" }
        }
    });
    inline_schema_refs(&mut schema);

    // $ref replaced with Foo's definition
    let item = &schema["properties"]["item"];
    assert_eq!(item["type"], "object");
    assert!(item["properties"]["bar"].is_object());
    // $ref itself is gone from the inlined property
    assert!(item.get("$ref").is_none());
}

#[test]
fn inline_schema_refs_resolves_nested_chain() {
    // Mirrors real Stitch pattern: DesignSystem → DesignTheme → Typography
    let mut schema = json!({
        "$defs": {
            "Typography": {
                "type": "object",
                "properties": { "fontSize": { "type": "string" } }
            },
            "DesignTheme": {
                "type": "object",
                "properties": {
                    "font": { "$ref": "#/$defs/Typography" }
                }
            },
            "DesignSystem": {
                "type": "object",
                "properties": {
                    "theme": { "$ref": "#/$defs/DesignTheme" }
                }
            }
        },
        "type": "object",
        "properties": {
            "designSystem": { "$ref": "#/$defs/DesignSystem" }
        }
    });
    inline_schema_refs(&mut schema);

    // Full chain resolved: designSystem.theme.font.fontSize
    let font_size = &schema["properties"]["designSystem"]["properties"]["theme"]["properties"]["font"]
        ["properties"]["fontSize"];
    assert_eq!(font_size["type"], "string");
}

#[test]
fn inline_schema_refs_resolves_array_items() {
    // Mirrors Stitch apply_design_system: array items reference a $def
    let mut schema = json!({
        "$defs": {
            "ScreenInstance": {
                "type": "object",
                "properties": { "id": { "type": "string" } }
            }
        },
        "type": "object",
        "properties": {
            "screens": {
                "type": "array",
                "items": { "$ref": "#/$defs/ScreenInstance" }
            }
        }
    });
    inline_schema_refs(&mut schema);

    let items = &schema["properties"]["screens"]["items"];
    assert_eq!(items["type"], "object");
    assert!(items["properties"]["id"].is_object());
}

#[test]
fn inline_schema_refs_depth_guard() {
    // Circular ref — should not infinite loop
    let mut schema = json!({
        "$defs": {
            "Node": {
                "type": "object",
                "properties": {
                    "child": { "$ref": "#/$defs/Node" }
                }
            }
        },
        "type": "object",
        "properties": {
            "root": { "$ref": "#/$defs/Node" }
        }
    });
    // Should complete without stack overflow
    inline_schema_refs(&mut schema);
    // Root is resolved at least once
    assert_eq!(schema["properties"]["root"]["type"], "object");
}

// -- VCR Integration Tests

#[test]
fn test_vcr_integration() -> Result<()> {
    use crate::vcr::{VcrCassette, VcrMode};
    use std::path::PathBuf;

    let path = PathBuf::from("tests/fixtures/cassettes/openai_completions.json");
    let cassette = VcrCassette::new(path, VcrMode::Replay)?;

    let matched = cassette.match_response(
        "https://api.openai.com/v1/chat/completions",
        "POST",
        "{\"messages\":[{\"content\":\"Hello\",\"role\":\"user\"}],\"model\":\"gpt-4o\",\"tools\":[]}",
    );

    assert!(matched.is_some());
    let interaction = matched.unwrap();
    assert!(
        interaction
            .response_body
            .contains("How can I assist you today?")
    );

    Ok(())
}

#[test]
fn test_vcr_cassette_recording_redacts_secrets() -> Result<()> {
    use crate::vcr::{HttpInteraction, VcrCassette, VcrMode};
    use std::io::Read;
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::new()?;
    let path = temp_file.path().to_path_buf();
    let cassette = VcrCassette::new(path.clone(), VcrMode::Record)?;

    let raw_interaction = HttpInteraction {
        url: "https://api.openai.com/v1/chat/completions?key=AIzaSy_123456789012345678901234567890123".to_string(),
        method: "sk-ant-sid01-someletters1234567890-XYZ123456".to_string(),
        request_body: "{\"api_key\":\"sk-openai123456789012345678901234567890\",\"text\":\"hello\"}".to_string(),
        response_status: 200,
        response_body: "Bearer some_raw_bearer_token".to_string(),
    };

    cassette.record_interaction(raw_interaction)?;

    // Read file and verify it's redacted before writing
    let mut file = std::fs::File::open(&path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    assert!(!contents.contains("sk-openai"));
    assert!(!contents.contains("sk-ant-"));
    assert!(!contents.contains("AIzaSy"));
    assert!(contents.contains("[REDACTED_OPENAI_KEY]"));
    assert!(contents.contains("[REDACTED_ANTHROPIC_KEY]"));
    assert!(contents.contains("[REDACTED_GEMINI_KEY]"));
    assert!(contents.contains("[REDACTED_BEARER_TOKEN]"));

    Ok(())
}

// -- clean_gemini_schema

#[test]
fn clean_gemini_schema_strips_all_bad_fields() {
    let mut schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$ref": "#/$defs/leftover",
        "$defs": { "leftover": { "type": "string" } },
        "type": "object",
        "additionalProperties": false,
        "nullable": true,
        "deprecated": true,
        "const": "hello",
        "x-google-identifier": true,
        "x-google-enum-descriptions": ["a", "b"],
        "x-google-enum-deprecated": [false, true],
        "properties": {
            "name": {
                "type": "string",
                "x-google-identifier": true,
                "deprecated": true,
                "description": "A name"
            },
            "age": {
                "type": ["integer", "null"]
            }
        }
    });
    clean_gemini_schema(&mut schema);

    // All bad fields removed
    let map = schema.as_object().unwrap();
    assert!(!map.contains_key("$schema"));
    assert!(!map.contains_key("$ref"));
    assert!(!map.contains_key("$defs"));
    assert!(!map.contains_key("additionalProperties"));
    assert!(!map.contains_key("nullable"));
    assert!(!map.contains_key("deprecated"));
    assert!(!map.contains_key("const"));
    assert!(!map.contains_key("x-google-identifier"));
    assert!(!map.contains_key("x-google-enum-descriptions"));
    assert!(!map.contains_key("x-google-enum-deprecated"));

    // Valid fields preserved
    assert_eq!(map["type"], "OBJECT");
    assert!(map.contains_key("properties"));

    // Nested x-google-* also removed
    let name_props = schema["properties"]["name"].as_object().unwrap();
    assert!(!name_props.contains_key("x-google-identifier"));
    assert_eq!(name_props["type"], "STRING");
    assert_eq!(name_props["description"], "A name");

    // Array type normalized to single string
    let age_props = schema["properties"]["age"].as_object().unwrap();
    assert_eq!(age_props["type"], "INTEGER");
}

#[test]
fn gemini_tool_prep_end_to_end() {
    // Realistic Stitch-shaped schema: inline + clean produces valid output
    let mut params = json!({
        "$defs": {
            "Typography": {
                "type": "object",
                "properties": {
                    "fontSize": { "type": "string" },
                    "fontWeight": { "type": "string" }
                }
            },
            "DesignTheme": {
                "type": "object",
                "x-google-enum-descriptions": ["light", "dark"],
                "properties": {
                    "bodyFont": {
                        "type": "string",
                        "enum": ["INTER", "ROBOTO"],
                        "x-google-enum-deprecated": [false, false]
                    },
                    "typography": {
                        "additionalProperties": { "$ref": "#/$defs/Typography" }
                    }
                }
            }
        },
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "x-google-identifier": true
            },
            "theme": { "$ref": "#/$defs/DesignTheme" }
        }
    });
    inline_schema_refs(&mut params);
    clean_gemini_schema(&mut params);

    let result = serde_json::to_string(&params).unwrap();

    // No bad fields survive
    assert!(!result.contains("$ref"), "no $ref should remain");
    assert!(!result.contains("$defs"), "no $defs should remain");
    assert!(!result.contains("x-google"), "no x-google-* should remain");
    assert!(
        !result.contains("additionalProperties"),
        "no additionalProperties should remain"
    );

    // Valid content preserved
    assert!(result.contains("bodyFont"));
    assert!(result.contains("INTER"));
    assert!(result.contains("\"type\":\"OBJECT\""));
}

// -- pricing rules: gap models

#[test]
fn pricing_gemini_25_flash() {
    let registry = crate::ModelRegistry::new();
    let p = registry.pricing_for_model("gemini/gemini-2.5-flash");
    assert_eq!(p.input, 0.15);
    assert_eq!(p.output, 0.6);
}

#[test]
fn pricing_gpt_41_mini() {
    let registry = crate::ModelRegistry::new();
    let p = registry.pricing_for_model("openai/gpt-4.1-mini");
    assert_eq!(p.input, 0.4);
    assert_eq!(p.output, 1.6);
}

#[test]
fn pricing_gpt_41_full_excludes_mini() {
    let registry = crate::ModelRegistry::new();
    let p = registry.pricing_for_model("openai/gpt-4.1");
    assert_eq!(p.input, 2.0);
    assert_eq!(p.output, 8.0);
}

#[test]
fn pricing_o1() {
    let registry = crate::ModelRegistry::new();
    let p = registry.pricing_for_model("openai/o1");
    assert_eq!(p.input, 15.0);
    assert_eq!(p.output, 60.0);
}

#[test]
fn pricing_o1_mini_not_matched_by_o1_rule() {
    let registry = crate::ModelRegistry::new();
    let p = registry.pricing_for_model("openai/o1-mini");
    // Should NOT get o1 pricing ($15/$60) — the not_contains_any guard excludes "mini"
    assert!(p.input < 15.0, "o1-mini should not get o1 pricing");
}

#[test]
fn pricing_gpt_5_series() {
    let registry = crate::ModelRegistry::new();
    let p_base = registry.pricing_for_model("openai/gpt-5");
    assert_eq!(p_base.input, 5.0);
    assert_eq!(p_base.output, 30.0);

    let p_mini = registry.pricing_for_model("openai/gpt-5-mini");
    assert_eq!(p_mini.input, 1.1);
    assert_eq!(p_mini.output, 4.4);
}

#[test]
fn test_clean_json_markers_strips_markdown() {
    use crate::utils::clean_json_markers;

    let raw = "```json\n{\"test\": true}\n```";
    assert_eq!(clean_json_markers(raw), "{\"test\": true}");

    let raw_no_lang = "```\n{\"test\": true}\n```";
    assert_eq!(clean_json_markers(raw_no_lang), "{\"test\": true}");

    let plain = "{\"test\": true}";
    assert_eq!(clean_json_markers(plain), "{\"test\": true}");
}

#[tokio::test]
async fn test_provider_default_complete_structured() -> Result<()> {
    struct MockProvider;

    #[async_trait::async_trait]
    impl LlmProvider for MockProvider {
        async fn complete(&self, _req: &CompletionRequest) -> crate::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content: Some("```json\n{\"foo\": \"bar\"}\n```".to_string()),
                tool_calls: vec![],
                finish_reason: "stop".to_string(),
            })
        }
        async fn stream(
            &self,
            _req: &CompletionRequest,
        ) -> crate::Result<
            std::pin::Pin<Box<dyn tokio_stream::Stream<Item = crate::Result<StreamChunk>> + Send>>,
        > {
            Err(crate::Error::custom("not implemented"))
        }
    }

    let provider = MockProvider;
    let req = CompletionRequest {
        model: "test-model".to_string(),
        messages: vec![],
        tools: vec![],
        max_tokens: 100,
        reasoning_effort: None,
    };

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "foo": { "type": "string" }
        }
    });

    let res = provider.complete_structured(&req, schema).await?;
    assert_eq!(res["foo"], "bar");

    Ok(())
}

#[tokio::test]
async fn test_openai_complete_structured_vcr() -> Result<()> {
    use crate::vcr::{HttpInteraction, VcrCassette, VcrMode};
    use std::io::Write;
    use tempfile::NamedTempFile;

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer" }
        },
        "required": ["name", "age"]
    });

    let interactions = vec![
        HttpInteraction {
            url: "https://api.openai.com/v1/chat/completions".to_string(),
            method: "POST".to_string(),
            request_body: serde_json::to_string(&serde_json::json!({
                "model": "gpt-4o",
                "messages": [],
                "tools": [],
                "max_tokens": 100,
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "structured_output",
                        "strict": true,
                        "schema": schema
                    }
                }
            }))?,
            response_status: 200,
            response_body: "{\"choices\":[{\"finish_reason\":\"stop\",\"index\":0,\"message\":{\"content\":\"{\\\"name\\\": \\\"Jane\\\", \\\"age\\\": 30}\",\"role\":\"assistant\"}}]}".to_string(),
        }
    ];

    let mut temp_file = NamedTempFile::new()?;
    let content = serde_json::to_string_pretty(&interactions)?;
    temp_file.write_all(content.as_bytes())?;

    let cassette = VcrCassette::new(temp_file.path().to_path_buf(), VcrMode::Replay)?;

    let matched = cassette.match_response(
        "https://api.openai.com/v1/chat/completions",
        "POST",
        &serde_json::to_string(&serde_json::json!({
            "model": "gpt-4o",
            "messages": [],
            "tools": [],
            "max_tokens": 100,
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "structured_output",
                    "strict": true,
                    "schema": schema
                }
            }
        }))?,
    );

    assert!(matched.is_some());
    let interaction = matched.unwrap();
    assert!(interaction.response_body.contains("Jane"));
    assert!(interaction.response_body.contains("30"));

    Ok(())
}

#[tokio::test]
async fn test_anthropic_complete_structured_vcr() -> Result<()> {
    use crate::vcr::{HttpInteraction, VcrCassette, VcrMode};
    use std::io::Write;
    use tempfile::NamedTempFile;

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" }
        },
        "required": ["title"]
    });

    let interactions = vec![
        HttpInteraction {
            url: "https://api.anthropic.com/v1/messages".to_string(),
            method: "POST".to_string(),
            request_body: serde_json::to_string(&serde_json::json!({
                "model": "claude-3-5-sonnet-20241022",
                "messages": [],
                "max_tokens": 100,
                "tools": [
                    {
                        "name": "structured_output",
                        "description": "Output the final structured JSON response matching the required schema.",
                        "input_schema": schema
                    }
                ],
                "tool_choice": {
                    "type": "tool",
                    "name": "structured_output"
                }
            }))?,
            response_status: 200,
            response_body: "{\"content\":[{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"structured_output\",\"input\":{\"title\":\"Hello Anthropic\"}}],\"stop_reason\":\"tool_use\"}".to_string(),
        }
    ];

    let mut temp_file = NamedTempFile::new()?;
    let content = serde_json::to_string_pretty(&interactions)?;
    temp_file.write_all(content.as_bytes())?;

    let cassette = VcrCassette::new(temp_file.path().to_path_buf(), VcrMode::Replay)?;

    let matched = cassette.match_response(
        "https://api.anthropic.com/v1/messages",
        "POST",
        &serde_json::to_string(&serde_json::json!({
            "model": "claude-3-5-sonnet-20241022",
            "messages": [],
            "max_tokens": 100,
            "tools": [
                {
                    "name": "structured_output",
                    "description": "Output the final structured JSON response matching the required schema.",
                    "input_schema": schema
                }
            ],
            "tool_choice": {
                "type": "tool",
                "name": "structured_output"
            }
        }))?,
    );

    assert!(matched.is_some());
    let interaction = matched.unwrap();
    assert!(interaction.response_body.contains("Hello Anthropic"));

    Ok(())
}

#[tokio::test]
async fn test_gemini_complete_structured_vcr() -> Result<()> {
    use crate::vcr::{HttpInteraction, VcrCassette, VcrMode};
    use std::io::Write;
    use tempfile::NamedTempFile;

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "score": { "type": "integer" }
        },
        "required": ["score"]
    });

    let mut clean_schema = schema.clone();
    crate::utils::clean_gemini_schema(&mut clean_schema);

    let interactions = vec![
        HttpInteraction {
            url: "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key=mock_key".to_string(),
            method: "POST".to_string(),
            request_body: serde_json::to_string(&serde_json::json!({
                "contents": [],
                "generationConfig": {
                    "responseMimeType": "application/json",
                    "responseSchema": clean_schema
                }
            }))?,
            response_status: 200,
            response_body: "{\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"{\\\"score\\\": 95}\"}]}}]}".to_string(),
        }
    ];

    let mut temp_file = NamedTempFile::new()?;
    let content = serde_json::to_string_pretty(&interactions)?;
    temp_file.write_all(content.as_bytes())?;

    let cassette = VcrCassette::new(temp_file.path().to_path_buf(), VcrMode::Replay)?;

    let matched = cassette.match_response(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key=mock_key",
        "POST",
        &serde_json::to_string(&serde_json::json!({
            "contents": [],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": clean_schema
            }
        }))?,
    );

    assert!(matched.is_some());
    let interaction = matched.unwrap();
    assert!(interaction.response_body.contains("95"));

    Ok(())
}

#[test]
fn test_corrected_token_pricing_resolution() {
    let registry = crate::ModelRegistry::new();

    // 1. Verify Claude 3.5 Haiku updated pricing
    let p_haiku = registry.pricing_for_model("anthropic/claude-3-5-haiku");
    assert_eq!(p_haiku.input, 1.0);
    assert_eq!(p_haiku.output, 5.0);
    assert_eq!(p_haiku.cache_read, 0.1);
    assert_eq!(p_haiku.cache_write, 1.25);

    // 2. Verify DeepSeek R1 (reasoner) matches the reasoner rules via fallback
    let p_r1 = registry.pricing_for_model("deepseek/some-deepseek-r1");
    assert_eq!(p_r1.input, 0.55);
    assert_eq!(p_r1.output, 2.19);
    assert_eq!(p_r1.cache_read, 0.14);

    // 3. Verify DeepSeek V3 (chat) matches the chat rules via fallback
    let p_v3 = registry.pricing_for_model("deepseek/some-deepseek-v3");
    assert_eq!(p_v3.input, 0.14);
    assert_eq!(p_v3.output, 0.28);
    assert_eq!(p_v3.cache_read, 0.014);

    // 4. Verify generic deepseek/ wildcard falls back to V3 prices
    let p_generic = registry.pricing_for_model("deepseek/some-unknown-model");
    assert_eq!(p_generic.input, 0.14);
    assert_eq!(p_generic.output, 0.28);

    // 5. Verify pricing for DeepSeek models in default_pricing.json
    let p_r1_dynamic = registry.pricing_for_model("deepseek/deepseek-reasoner");
    assert_eq!(p_r1_dynamic.input, 0.55);
    assert_eq!(p_r1_dynamic.cache_read, 0.14);
}

#[test]
fn debug_prices() {
    if let Some(m) = llm_providers::get_model("anthropic", "claude-3-5-sonnet-20241022") {
        eprintln!(
            "claude-3-5-sonnet-20241022: input={}, output={}, cache_read_computed={}",
            m.input_price,
            m.output_price,
            m.input_price * 0.1
        );
    } else {
        eprintln!("claude-3-5-sonnet-20241022: NOT FOUND in llm_providers");
    }
    if let Some(m) = llm_providers::get_model("openai", "gpt-4.1") {
        eprintln!(
            "gpt-4.1: input={}, output={}",
            m.input_price, m.output_price
        );
    } else {
        eprintln!("gpt-4.1: NOT FOUND -> fallback");
    }
    if let Some(m) = llm_providers::get_model("openai", "gpt-5") {
        eprintln!("gpt-5: input={}, output={}", m.input_price, m.output_price);
    } else {
        eprintln!("gpt-5: NOT FOUND -> fallback");
    }
    if let Some(m) = llm_providers::get_model("openai", "gpt-5-mini") {
        eprintln!(
            "gpt-5-mini: input={}, output={}",
            m.input_price, m.output_price
        );
    } else {
        eprintln!("gpt-5-mini: NOT FOUND -> fallback");
    }
}

#[test]
fn debug_prices_claude_variants() {
    for name in &[
        "claude-3-5-sonnet-20241022",
        "claude-3-5-sonnet-latest",
        "claude-sonnet-4-20250514",
        "claude-sonnet-4",
    ] {
        if let Some(m) = llm_providers::get_model("anthropic", name) {
            eprintln!(
                "{}: input={}, output={}",
                name, m.input_price, m.output_price
            );
        } else {
            eprintln!("{}: NOT FOUND", name);
        }
    }
}


#[test]
fn test_llm_router_openrouter_failover_mapping() {
    let config = AiConfig {
        anthropic_api_key: Some("test-anthropic-key".to_string()),
        openai_api_key: Some("test-openai-key".to_string()),
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".to_string(),
        llm_provider: "anthropic".to_string(),
    };
    
    let router = LlmRouter::build(&config);
    
    // Test mapping translations
    let (prov, model) = router.map_openrouter_to_native("anthropic/claude-3-5-sonnet-20241022").unwrap();
    assert_eq!(prov, "anthropic");
    assert_eq!(model, "claude-3-5-sonnet-20241022");

    let (prov2, model2) = router.map_openrouter_to_native("openai/gpt-4o-mini").unwrap();
    assert_eq!(prov2, "openai");
    assert_eq!(model2, "gpt-4o-mini");
}