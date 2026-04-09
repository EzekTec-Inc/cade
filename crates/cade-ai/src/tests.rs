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
