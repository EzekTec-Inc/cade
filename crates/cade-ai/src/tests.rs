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
