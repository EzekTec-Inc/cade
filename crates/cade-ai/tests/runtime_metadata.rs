use cade_ai::provider_registry::{ProviderDef, ProviderRegistry};
use cade_ai::runtime::{ModelMetadata, RegisteredModel, RuntimeRegistry, TokenizerKind};
use cade_ai::{ModelEntry, catalogue::CatalogueRow, count_tokens_with_registry};
use serde_json::json;

#[test]
fn configured_aliases_catalogue_and_tokenizer_share_exact_nested_model_identity() {
    let providers = ProviderRegistry::from_json(
        &json!([{
            "name":"office", "aliases":["office-alt"], "kind":"openai-compatible",
            "chat_url":"http://127.0.0.1:1234/v1"
        }])
        .to_string(),
    )
    .unwrap();
    let mut registry = RuntimeRegistry::from_json(
        &json!({
            "models":[{
                "id":"office/tenant/gpt-4o-clone-0125", "aliases":["short-name"],
                "tokenizer":"characters", "chars_per_token":2, "tools":false,
                "native_structured":false, "max_tokens":777, "context_window":9999
            }]
        })
        .to_string(),
    )
    .unwrap();
    registry.set_provider_registry(&providers);
    for id in [
        "short-name",
        "office/tenant/gpt-4o-clone-0125",
        "office-alt/tenant/gpt-4o-clone-0125",
    ] {
        let metadata = registry.metadata_for_id(id);
        assert_eq!(metadata.tokenizer, Some(TokenizerKind::Characters));
        assert_eq!(metadata.tools, Some(false));
        assert_eq!(metadata.native_structured, Some(false));
        assert_eq!(metadata.max_tokens, Some(777));
        assert_eq!(metadata.context_window, Some(9999));
        assert_eq!(count_tokens_with_registry(&registry, id, "😃😃😃"), 2);
        assert_eq!(count_tokens_with_registry(&registry, id, "x"), 1);
        assert!(
            registry
                .canonical_model_id(id)
                .ends_with("gpt-4o-clone-0125")
        );
    }
    let snapshot = registry.catalogue_snapshot();
    let row: &CatalogueRow = snapshot
        .iter()
        .find(|row| row.2 == "office/tenant/gpt-4o-clone-0125")
        .unwrap();
    let entry = ModelEntry::from_catalogue(row);
    assert_eq!(entry.max_tokens, 777);
    assert_eq!(entry.context_window, 9999);
    assert_eq!(entry.toolset, "none");
    // Unknown models containing familiar vendor/model names do not select BPE by substring.
    assert_eq!(
        registry
            .metadata_for_id("office/tenant/claude-gpt-4o-gemini")
            .tokenizer,
        Some(TokenizerKind::Cl100kBase)
    );
}

#[test]
fn capabilities_false_and_partial_rules_preserve_other_recipe_fields() {
    let registry = RuntimeRegistry::from_json(
        &json!({
            "rules":[{"providers":["openai"], "prefixes":["gpt-5"], "max_tokens":888}],
            "models":[{"id":"openai/gpt-5", "tools":false, "native_structured":false,
                "developer_role":false, "preview_gateway":false, "include_reasoning":false}]
        })
        .to_string(),
    )
    .unwrap();
    let metadata = registry.metadata_for_id("openai/gpt-5");
    assert_eq!(metadata.max_tokens, Some(888));
    assert_eq!(
        metadata.protocol,
        Some(cade_ai::openai::ApiProtocol::Responses)
    );
    assert_eq!(metadata.tokenizer, Some(TokenizerKind::O200kBase));
    assert_eq!(metadata.tools, Some(false));
    assert_eq!(metadata.native_structured, Some(false));
    assert_eq!(metadata.developer_role, Some(false));
    assert_eq!(metadata.preview_gateway, Some(false));
    assert_eq!(metadata.include_reasoning, Some(false));
    assert_eq!(metadata.toolset.as_deref(), Some("none"));
}

#[test]
fn fresh_catalogue_uses_discovery_and_partial_registered_limits_without_duplicates() {
    let mut registry = RuntimeRegistry::default();
    registry
        .try_register(RegisteredModel {
            id: "private/tenant/deployment-2026-09-30".into(),
            aliases: vec!["deployment".into()],
            metadata: ModelMetadata {
                max_tokens: Some(777),
                ..Default::default()
            },
        })
        .unwrap();
    let old = registry.catalogue_snapshot();
    registry.discovered.insert(
        "private/tenant/deployment-2026-09-30".into(),
        ModelMetadata {
            max_tokens: Some(888),
            context_window: Some(12345),
            display_name: Some("Actual deployment".into()),
            tools: Some(true),
            ..Default::default()
        },
    );
    assert!(registry.has_known_limits("private", "tenant/deployment-2026-09-30"));
    let fresh = registry.catalogue_snapshot();
    assert_eq!(
        fresh
            .iter()
            .filter(|row| row.2 == "private/tenant/deployment-2026-09-30")
            .count(),
        1
    );
    let entry = fresh
        .iter()
        .find(|row| row.2 == "private/tenant/deployment-2026-09-30")
        .unwrap();
    assert_eq!(entry.1, "Actual deployment");
    assert_eq!((entry.4, entry.5), (777, 12345));
    assert_ne!(old, fresh);
    assert_eq!(
        registry.canonical_model_id("deployment"),
        "private/tenant/deployment-2026-09-30"
    );
}

#[test]
fn invalid_limits_and_ambiguous_aliases_fail_atomically_and_file_loading_recovers() {
    for config in [
        json!({"fallback":{"max_tokens":0}}),
        json!({"fallback":{"context_window":0}}),
        json!({"fallback":{"chars_per_token":0}}),
        json!({"models":[{"id":"private/model", "tokenizer":"unknown-encoder"}]}),
        json!({"models":[{"id":"private/model-a","aliases":["same"]},{"id":"private/model-b","aliases":["same"]}]}),
    ] {
        assert!(RuntimeRegistry::from_json(&config.to_string()).is_err());
    }
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("models.json");
    std::fs::write(&file, r#"{"fallback":{"chars_per_token":0}}"#).unwrap();
    let loaded = RuntimeRegistry::load(Some(&file));
    assert_eq!(
        loaded.fallback.chars_per_token,
        RuntimeRegistry::default().fallback.chars_per_token
    );
    assert!(count_tokens_with_registry(&loaded, "unknown/model", "x") > 0);
    assert!(
        RuntimeRegistry::from_json(r#"{"failover_providers":[]}"#)
            .unwrap()
            .failover_providers
            .is_empty()
    );
}

#[test]
fn provider_canonical_names_win_over_aliases_and_invalid_environment_names_are_safe() {
    let providers = ProviderRegistry::from_json(&json!([
        {"name":"custom", "aliases":["openai","custom-alt"], "chat_url":"http://127.0.0.1:1234/v1"},
        {"name":"google", "kind":"openai-compatible", "chat_url":"http://127.0.0.1:1235/v1"}
    ]).to_string()).unwrap();
    assert_eq!(providers.get("openai").unwrap().name, "openai");
    assert_eq!(providers.get("google").unwrap().kind, "openai-compatible");
    assert_eq!(providers.get("custom-alt").unwrap().name, "custom");
    assert!(!providers.alias_map().contains_key("google"));
    let mut registry = RuntimeRegistry::default();
    registry.set_provider_registry(&providers);
    assert_eq!(
        registry.metadata_for_id("google/gemini-2.5-pro").thinking,
        None
    );
    assert_eq!(
        registry
            .metadata_for_id("gemini/gemini-2.5-pro")
            .thinking
            .as_deref(),
        Some("budget")
    );
    assert!(
        ProviderRegistry::from_json(
            &json!([
                {"name":"a","aliases":["same"],"chat_url":"http://127.0.0.1:1234/v1"},
                {"name":"b","aliases":["same"],"chat_url":"http://127.0.0.1:1235/v1"}
            ])
            .to_string()
        )
        .is_err()
    );
    let raw: ProviderDef = serde_json::from_value(json!({
        "name":"custom", "chat_url":"http://127.0.0.1:1234/v1",
        "env_vars":["","BAD=NAME","BAD\u{0000}NAME"],
        "base_url_env":["","BAD=NAME","BAD\u{0000}NAME"]
    }))
    .unwrap();
    assert_eq!(raw.env_key(), None);
    assert_eq!(raw.endpoint(), raw.chat_url);
}

#[test]
fn configured_budget_ratios_saturate_without_overflow_or_zero_token_messages() {
    let registry = RuntimeRegistry::from_json(
        &json!({"models":[{
            "id":"private/huge-ratio", "tokenizer":"characters", "chars_per_token":usize::MAX
        }]})
        .to_string(),
    )
    .unwrap();
    let turns = vec![vec![cade_ai::LlmMessage {
        role: "user".into(),
        content: "x".into(),
        tool_call_id: None,
        tool_calls: None,
        images: None,
        cache_control: None,
    }]];
    let budget = cade_ai::PromptBudgetManager::new().calculate_budget_with_registry(
        &registry,
        "private/huge-ratio",
        &turns,
        usize::MAX,
        usize::MAX,
    );
    assert_eq!(budget.selected_turns.len(), 1);
    assert_eq!(budget.total_chars_used, usize::MAX);
    assert_eq!(budget.total_tokens_used, usize::MAX);
    assert_eq!(
        count_tokens_with_registry(&registry, "private/huge-ratio", "x"),
        1
    );
}

#[test]
fn configured_background_models_and_suffix_policy_preserve_nested_namespaces() {
    let providers = ProviderRegistry::from_json(&json!([
        {"name":"office","aliases":["office-alt"],"kind":"openai-compatible","chat_url":"http://127.0.0.1:1/v1",
            "default_model":"tenant/default", "fast_model":"tenant/fast", "background_model":"tenant/background"},
        {"name":"default-only","kind":"openai-compatible","chat_url":"http://127.0.0.1:1/v1", "default_model":"tenant/default"}
    ]).to_string()).unwrap();
    let mut metadata = RuntimeRegistry::from_json(
        &json!({"rules":[{
            "providers":["office"],"suffixes":[":keep"],"prefer_primary_for_background":true
        }]})
        .to_string(),
    )
    .unwrap();
    metadata.set_provider_registry(&providers);
    assert_eq!(
        providers.fast_model_for("office-alt/tenant/primary", &metadata),
        "office/tenant/fast"
    );
    assert_eq!(
        providers.background_model_for("office-alt/tenant/primary", &metadata),
        "office/tenant/background"
    );
    assert_eq!(
        providers.background_model_for("office/tenant/primary:keep", &metadata),
        "office/tenant/primary:keep"
    );
    assert_eq!(
        providers.background_model_for("office/tenant/background", &metadata),
        "office/tenant/background"
    );
    assert_eq!(
        providers.background_model_for("default-only/tenant/primary", &metadata),
        "default-only/tenant/default"
    );
    assert_eq!(
        providers.background_model_for("unregistered/tenant/primary", &metadata),
        "unregistered/tenant/primary"
    );
    let explicit = "office/tenant/user-picked";
    assert_ne!(
        providers.background_model_for(explicit, &metadata),
        explicit,
        "only implicit callers should invoke background selection; explicit compaction choices bypass it"
    );
}
