/// Owned compatibility row: (provider, display name, routing ID, toolset,
/// output budget, context budget). No model records are duplicated in Rust.
pub type CatalogueRow = (String, String, String, String, u32, u32);

/// First-use compatibility snapshot from configured JSON and runtime observations.
/// `.iter()`, indexing and slice methods remain available. Use `catalogue_snapshot`
/// for a fresh view after discovery or SDK registration.
pub static CATALOGUE: std::sync::LazyLock<Vec<CatalogueRow>> =
    std::sync::LazyLock::new(catalogue_snapshot);

pub fn catalogue_snapshot() -> Vec<CatalogueRow> {
    crate::runtime::shared_registry()
        .read()
        .catalogue_snapshot()
}

/// A model entry returned by `GET /v1/models`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelEntry {
    pub provider: String,
    pub id: String,
    pub display_name: String,
    pub toolset: String,
    pub max_tokens: u32,
    /// Model's input context window size in tokens.
    pub context_window: u32,
    /// `true` if discovered at runtime (e.g. Ollama `/api/tags`), `false` if from static catalogue.
    #[serde(default)]
    pub dynamic: bool,
}

impl ModelEntry {
    pub fn from_catalogue<S: AsRef<str>>(e: &(S, S, S, S, u32, u32)) -> Self {
        Self {
            provider: e.0.as_ref().into(),
            id: e.2.as_ref().into(),
            display_name: e.1.as_ref().into(),
            toolset: e.3.as_ref().into(),
            max_tokens: e.4,
            context_window: e.5,
            dynamic: false,
        }
    }
}

pub(crate) fn strip_model_snapshot_suffix(id: &str) -> &str {
    // Check -YYYY-MM-DD (11 chars: '-' + 4 digits + '-' + 2 digits + '-' + 2 digits)
    if id.len() > 11 {
        let Some(suffix) = id.get(id.len() - 11..) else {
            return id;
        };
        let bytes = suffix.as_bytes();
        if bytes[0] == b'-'
            && bytes[1..5].iter().all(u8::is_ascii_digit)
            && bytes[5] == b'-'
            && bytes[6..8].iter().all(u8::is_ascii_digit)
            && bytes[8] == b'-'
            && bytes[9..11].iter().all(u8::is_ascii_digit)
        {
            return &id[..id.len() - 11];
        }
    }
    // Check -MMDD (5 chars: '-' + 4 digits, e.g. -0613, -0125)
    if id.len() > 5 {
        let Some(suffix) = id.get(id.len() - 5..) else {
            return id;
        };
        let bytes = suffix.as_bytes();
        if bytes[0] == b'-' && bytes[1..5].iter().all(u8::is_ascii_digit) {
            return &id[..id.len() - 5];
        }
    }
    id
}

pub fn normalize_model_id_for_lookup(model_id: &str) -> String {
    crate::runtime::shared_registry()
        .read()
        .canonical_model_id(model_id)
}

/// Determine the toolset for a specific model ID. Defaults to "default" if unknown.
pub fn toolset_for_model(model_id: &str) -> String {
    metadata_for_model(model_id)
        .toolset
        .unwrap_or_else(|| "default".into())
}

pub(crate) fn metadata_for_model(model_id: &str) -> crate::runtime::ModelMetadata {
    crate::runtime::shared_registry()
        .read()
        .metadata_for_id(model_id)
}

/// Returns false if the model explicitly does not support tool calling (e.g. deepseek-reasoner).
pub fn supports_tools_for_model(model_id: &str) -> bool {
    metadata_for_model(model_id).tools != Some(false)
}

/// Determine the max output tokens for a specific model ID. Defaults to 4096 if unknown.
pub fn max_tokens_for_model(model_id: &str) -> u32 {
    metadata_for_model(model_id).max_tokens.unwrap_or(0)
}

/// Determine the context window (input tokens) for a specific model ID.
///
/// Used to compute the character budget for message history trimming.
/// Falls back to editable conservative budgeting defaults for unknown models.
///
/// The env var `CADE_CONTEXT_BUDGET` (in chars) overrides everything when set.
pub fn context_window_for_model(model_id: &str) -> u32 {
    // Env var hard-override (useful for testing or unusual deployments)
    if let Ok(val) = std::env::var("CADE_CONTEXT_BUDGET")
        && let Ok(n) = val.parse::<u32>()
        && n > 0
    {
        return n;
    }

    metadata_for_model(model_id).context_window.unwrap_or(0)
}

/// Returns a fast, cost-effective reasoning model from the same provider as the main model.
/// Ideal for subagents (like heuristic evaluators) that run frequently and synchronously.
pub fn fast_model_for_main_model(main_model: &str) -> String {
    let providers = crate::provider_registry::ProviderRegistry::configured();
    providers.fast_model_for(main_model, &crate::runtime::shared_registry().read())
}

/// Configured background selection, with any per-model passthrough policy.
pub fn background_model_for_main_model(main_model: &str) -> String {
    let providers = crate::provider_registry::ProviderRegistry::configured();
    providers.background_model_for(main_model, &crate::runtime::shared_registry().read())
}

/// Inspect environment variables to detect which LLM providers have keys configured.
pub fn available_env_providers() -> Vec<String> {
    crate::provider_registry::ProviderRegistry::configured()
        .get_all_providers()
        .iter()
        .filter(|p| p.env_key().is_some())
        .map(|p| p.name.clone())
        .collect()
}

/// Select a fast subagent model given a parent model and optionally a list of available/authenticated providers.
/// If `available_providers` is None, detects available providers from the environment.
/// If the preferred fast model's provider is available (or if no providers can be detected),
/// returns that provider's fast model. If the preferred provider is unconfigured,
/// falls back to the best available configured provider's fast model.
pub fn select_fast_subagent_model(
    parent_model: &str,
    available_providers: Option<&[String]>,
) -> String {
    let fast_default = fast_model_for_main_model(parent_model);
    let env_providers;
    let providers = match available_providers {
        Some(p) => p,
        None => {
            env_providers = available_env_providers();
            &env_providers[..]
        }
    };
    if providers.is_empty() {
        return fast_default;
    }

    let default_provider = fast_default.split('/').next().unwrap_or(&fast_default);
    if providers
        .iter()
        .any(|p| p.eq_ignore_ascii_case(default_provider))
    {
        return fast_default;
    }

    crate::provider_registry::ProviderRegistry::configured()
        .get_all_providers()
        .iter()
        .filter(|p| {
            p.fast_model.is_some()
                && providers
                    .iter()
                    .any(|avail| avail == &p.name || p.aliases.contains(avail))
        })
        .min_by_key(|p| (p.fast_priority, &p.name))
        .and_then(|p| {
            p.fast_model
                .as_ref()
                .map(|model| format!("{}/{model}", p.name))
        })
        .unwrap_or(fast_default)
}

// endregion: --- Tests

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;

    // -- CATALOGUE

    #[test]
    fn catalogue_non_empty() {
        assert!(!CATALOGUE.is_empty());
    }

    #[test]
    fn catalogue_all_entries_have_valid_fields() {
        for (provider, display, id, toolset, max_tok, ctx) in CATALOGUE.iter() {
            assert!(!provider.is_empty(), "empty provider for {id}");
            assert!(!display.is_empty(), "empty display for {id}");
            assert!(!id.is_empty(), "empty id");
            assert!(
                ["default", "codex", "gemini", "none"].contains(&toolset.as_str()),
                "invalid toolset '{toolset}' for {id}"
            );
            assert!(*max_tok > 0, "zero max_tokens for {id}");
            assert!(*ctx > 0, "zero context_window for {id}");
        }
    }

    #[test]
    fn catalogue_ids_are_prefixed_with_provider() {
        for (provider, _, id, _, _, _) in CATALOGUE.iter() {
            assert!(
                id.starts_with(&format!("{provider}/")),
                "id '{id}' should start with '{provider}/'"
            );
        }
    }

    // -- ModelEntry::from_catalogue

    #[test]
    fn model_entry_from_catalogue() {
        let entry = &CATALOGUE[0];
        let me = ModelEntry::from_catalogue(entry);
        assert_eq!(me.provider, entry.0);
        assert_eq!(me.display_name, entry.1);
        assert_eq!(me.id, entry.2);
        assert_eq!(me.toolset, entry.3);
        assert_eq!(me.max_tokens, entry.4);
        assert_eq!(me.context_window, entry.5);
        assert!(!me.dynamic);
    }

    #[test]
    fn normalize_model_id_for_lookup_prefixes_bare_openai_models() {
        assert_eq!(normalize_model_id_for_lookup("gpt-4o"), "openai/gpt-4o");
        assert_eq!(
            normalize_model_id_for_lookup("chatgpt-4o-latest"),
            "openai/chatgpt-4o-latest"
        );
        assert_eq!(normalize_model_id_for_lookup("o3-mini"), "openai/o3-mini");
        assert_eq!(
            normalize_model_id_for_lookup("openai/gpt-4o"),
            "openai/gpt-4o"
        );
        assert_eq!(
            normalize_model_id_for_lookup("gemini/gemini-2.5-pro"),
            "gemini/gemini-2.5-pro"
        );
    }

    #[test]
    fn context_window_resolves_bare_openai_models() {
        assert_eq!(
            context_window_for_model("gpt-4o"),
            context_window_for_model("openai/gpt-4o")
        );
        assert_eq!(
            context_window_for_model("o3-mini"),
            context_window_for_model("openai/o3-mini")
        );
        assert_eq!(
            context_window_for_model("gpt-5"),
            context_window_for_model("openai/gpt-5")
        );
    }

    // -- toolset_for_model

    #[test]
    fn toolset_known_models() {
        assert_eq!(
            toolset_for_model("anthropic/claude-sonnet-4-5-20250929"),
            "default"
        );
        assert_eq!(toolset_for_model("openai/gpt-4o"), "codex");
        assert_eq!(toolset_for_model("gemini/gemini-2.5-pro"), "gemini");
    }

    #[test]
    fn toolset_unknown_gemini_prefix() {
        assert_eq!(toolset_for_model("gemini/gemini-999"), "default");
    }

    #[test]
    fn toolset_unknown_model() {
        assert_eq!(toolset_for_model("groq/llama-3-70b"), "default");
    }

    // -- max_tokens_for_model

    #[test]
    fn max_tokens_known_models() {
        assert_eq!(
            max_tokens_for_model("anthropic/claude-sonnet-4-5-20250929"),
            128_000
        );
        assert_eq!(max_tokens_for_model("openai/gpt-4o"), 16384);
    }

    #[test]
    fn max_tokens_unknown_gemini() {
        assert_eq!(max_tokens_for_model("gemini/future-model"), 4096);
    }

    #[test]
    fn max_tokens_unknown_gpt5() {
        assert_eq!(max_tokens_for_model("openai/gpt-5.5-preview"), 4096);
        assert_eq!(max_tokens_for_model("openai/gpt-5.6-luna"), 16384);
        assert_eq!(max_tokens_for_model("openai/gpt-5.5-pro"), 16384);
    }

    #[test]
    fn gpt56_models_are_catalogued_as_codex() {
        assert_eq!(toolset_for_model("openai/gpt-5.6"), "codex");
        assert_eq!(toolset_for_model("openai/gpt-5.6-luna"), "codex");
        assert_eq!(toolset_for_model("openai/gpt-5.5-pro"), "codex");
        assert_eq!(context_window_for_model("openai/gpt-5.6-luna"), 200_000);
        assert_eq!(context_window_for_model("openai/gpt-5.5-pro"), 200_000);
    }

    #[test]
    fn deepseek_models_are_catalogued_correctly() {
        assert_eq!(toolset_for_model("deepseek/deepseek-chat"), "codex");
        assert_eq!(toolset_for_model("deepseek/deepseek-reasoner"), "none");
        assert!(supports_tools_for_model("deepseek/deepseek-chat"));
        assert!(!supports_tools_for_model("deepseek/deepseek-reasoner"));
        assert!(!supports_tools_for_model("deepseek-reasoner"));
        assert_eq!(context_window_for_model("deepseek/deepseek-chat"), 64_000);
        assert_eq!(
            context_window_for_model("deepseek/deepseek-reasoner"),
            64_000
        );
        assert_eq!(max_tokens_for_model("deepseek/deepseek-chat"), 8192);
        assert_eq!(max_tokens_for_model("deepseek/deepseek-reasoner"), 8192);
    }

    #[test]
    fn max_tokens_unknown_openai() {
        assert_eq!(max_tokens_for_model("openai/future-model"), 4096);
    }

    #[test]
    fn max_tokens_completely_unknown() {
        assert_eq!(max_tokens_for_model("random/model"), 4096);
    }

    #[test]
    fn max_tokens_bare_gpt5_defaults_to_safe_budget() {
        // Exact offline metadata wins; unknown names use configurable fallback budgets.
        assert_eq!(max_tokens_for_model("gpt-5"), 16384);
        assert_eq!(max_tokens_for_model("gpt-5.1-preview"), 4096);
        assert_eq!(max_tokens_for_model("gpt-5.5-pro"), 16384);
    }
    // -- context_window_for_model

    #[test]
    fn context_window_known_models() {
        assert_eq!(
            context_window_for_model("anthropic/claude-sonnet-4-5-20250929"),
            1_048_576
        );
        assert_eq!(context_window_for_model("openai/gpt-4o"), 128_000);
        assert_eq!(context_window_for_model("gemini/gemini-2.5-pro"), 1_048_576);
    }

    #[test]
    fn context_window_provider_prefix_fallback() {
        assert_eq!(context_window_for_model("anthropic/future-claude"), 32_000);
        assert_eq!(context_window_for_model("gemini/future-gemini"), 32_000);
        assert_eq!(context_window_for_model("openai/future-gpt"), 32_000);
    }

    #[test]
    fn context_window_legacy_claude_heuristic_caps_at_200k() {
        // Uncatalogued claude-3 IDs hit the provider heuristic; they must not be
        // granted the modern 1M-token window.
        assert_eq!(
            context_window_for_model("anthropic/claude-3-5-sonnet-20241022"),
            200_000
        );
        // A future version has no known context length merely because it is named Sonnet.
        assert_eq!(
            context_window_for_model("anthropic/claude-sonnet-4-9"),
            32_000
        );
    }

    #[test]
    fn context_window_llama_model() {
        assert_eq!(context_window_for_model("groq/llama-3-70b"), 32_000);
    }

    #[test]
    fn context_window_completely_unknown() {
        assert_eq!(context_window_for_model("random/model-xyz"), 32_000);
    }

    // -- Bug 6: fast_model_for_main_model returns current-gen models

    #[test]
    fn fast_model_anthropic_returns_haiku_4_5() {
        let result = super::fast_model_for_main_model("anthropic/claude-sonnet-4-20250514");
        assert_eq!(result, "anthropic/claude-haiku-4-5");
    }

    #[test]
    fn fast_model_openai_returns_o4_mini() {
        let result = super::fast_model_for_main_model("openai/gpt-4.1");
        assert_eq!(result, "openai/o4-mini");
    }

    #[test]
    fn fast_model_gemini_returns_2_0_flash() {
        let result = super::fast_model_for_main_model("gemini/gemini-2.5-pro");
        assert_eq!(result, "gemini/gemini-2.0-flash");
    }

    #[test]
    fn fast_model_unknown_provider_echoes_input() {
        let result = super::fast_model_for_main_model("ollama/llama3");
        assert_eq!(result, "ollama/llama3");
    }

    #[test]
    fn select_fast_subagent_model_prefers_matching_provider_if_available() {
        let providers = vec!["gemini".to_string(), "openai".to_string()];
        let model = select_fast_subagent_model("gemini/gemini-2.5-pro", Some(&providers));
        assert_eq!(model, "gemini/gemini-2.0-flash");
    }

    #[test]
    fn select_fast_subagent_model_falls_back_when_preferred_provider_missing() {
        // Parent model is Anthropic, but only Gemini and OpenAI keys are available
        let providers = vec!["gemini".to_string(), "openai".to_string()];
        let model = select_fast_subagent_model("anthropic/claude-sonnet-4", Some(&providers));
        assert_eq!(model, "gemini/gemini-2.0-flash");
    }
}
