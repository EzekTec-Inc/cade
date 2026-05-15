/// Static model catalogue — known-good models per provider.
///
/// Format: (provider_key, display_name, full_model_id, toolset_label, max_tokens, context_window_tokens)
///   toolset_label: "default" | "codex" | "gemini"  (maps to Toolset::from_str)
///   context_window_tokens: model's input context length in tokens
pub const CATALOGUE: &[(&str, &str, &str, &str, u32, u32)] = &[
    // -- Anthropic
    (
        "anthropic",
        "Claude Opus 4.7",
        "anthropic/claude-opus-4-7",
        "default",
        128_000,
        1_048_576,
    ),
    (
        "anthropic",
        "Claude Opus 4.5",
        "anthropic/claude-opus-4-5",
        "default",
        128_000,
        1_048_576,
    ),
    (
        "anthropic",
        "Claude Sonnet 4.6",
        "anthropic/claude-sonnet-4-6",
        "default",
        128_000,
        1_048_576,
    ),
    (
        "anthropic",
        "Claude Sonnet 4.5",
        "anthropic/claude-sonnet-4-5-20250929",
        "default",
        128_000,
        1_048_576,
    ),
    (
        "anthropic",
        "Claude Haiku 4.5",
        "anthropic/claude-haiku-4-5",
        "default",
        128_000,
        200_000,
    ),
    (
        "anthropic",
        "Claude Sonnet 3.7",
        "anthropic/claude-3-7-sonnet-20250219",
        "default",
        128_000,
        1_048_576,
    ),
    (
        "anthropic",
        "Claude Haiku 3.5",
        "anthropic/claude-3-5-haiku-20241022",
        "default",
        8192,
        200_000,
    ),
    (
        "anthropic",
        "Claude Opus 3",
        "anthropic/claude-3-opus-20240229",
        "default",
        4096,
        200_000,
    ),
    // -- OpenAI
    (
        "openai",
        "GPT-4.1",
        "openai/gpt-4.1",
        "codex",
        16384,
        1_048_576,
    ),
    ("openai", "GPT-4o", "openai/gpt-4o", "codex", 16384, 128_000),
    (
        "openai",
        "GPT-4o Mini",
        "openai/gpt-4o-mini",
        "codex",
        16384,
        128_000,
    ),
    (
        "openai",
        "o4 Mini",
        "openai/o4-mini",
        "codex",
        100_000,
        200_000,
    ),
    ("openai", "o3", "openai/o3", "codex", 100_000, 200_000),
    (
        "openai",
        "o3 Mini",
        "openai/o3-mini",
        "codex",
        100_000,
        200_000,
    ),
    // -- Google Gemini
    (
        "gemini",
        "Gemini 2.5 Pro",
        "gemini/gemini-2.5-pro",
        "gemini",
        8192,
        1_048_576,
    ),
    (
        "gemini",
        "Gemini 2.0 Flash",
        "gemini/gemini-2.0-flash",
        "gemini",
        8192,
        1_048_576,
    ),
    (
        "gemini",
        "Gemini 1.5 Pro",
        "gemini/gemini-1.5-pro",
        "gemini",
        8192,
        2_097_152,
    ),
    (
        "gemini",
        "Gemini 1.5 Flash",
        "gemini/gemini-1.5-flash",
        "gemini",
        8192,
        1_048_576,
    ),
];

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
    pub fn from_catalogue(e: &(&str, &str, &str, &str, u32, u32)) -> Self {
        Self {
            provider: e.0.to_string(),
            id: e.2.to_string(),
            display_name: e.1.to_string(),
            toolset: e.3.to_string(),
            max_tokens: e.4,
            context_window: e.5,
            dynamic: false,
        }
    }
}

/// Determine the toolset for a specific model ID. Defaults to "default" if unknown.
pub fn toolset_for_model(model_id: &str) -> String {
    if let Some(m) = CATALOGUE.iter().find(|(_, _, id, _, _, _)| *id == model_id) {
        m.3.to_string()
    } else if model_id.starts_with("gemini/") {
        "gemini".to_string()
    } else {
        "default".to_string() // Groq, OpenRouter, Ollama default to generic openai/anthropic style
    }
}

/// Determine the max output tokens for a specific model ID. Defaults to 4096 if unknown.
pub fn max_tokens_for_model(model_id: &str) -> u32 {
    if let Some(m) = CATALOGUE.iter().find(|(_, _, id, _, _, _)| *id == model_id) {
        m.4
    } else if model_id.starts_with("anthropic/claude-") {
        128_000
    } else if model_id.starts_with("openai/o")
        || model_id.starts_with("openai/gpt-5")
        || model_id.starts_with("gpt-5")
    {
        100_000
    } else if model_id.starts_with("gemini/")
        || model_id.starts_with("google/gemini")
        || model_id.starts_with("openai/")
    {
        8192
    } else {
        4096 // Safe default for older models / unknown providers
    }
}

/// Determine the context window (input tokens) for a specific model ID.
///
/// Used to compute the character budget for message history trimming.
/// Falls back by provider prefix, then to a conservative 32 K default.
///
/// The env var `CADE_CONTEXT_BUDGET` (in chars) overrides everything when set.
pub fn context_window_for_model(model_id: &str) -> u32 {
    // Env var hard-override (useful for testing or unusual deployments)
    if let Ok(val) = std::env::var("CADE_CONTEXT_BUDGET")
        && let Ok(n) = val.parse::<u32>()
    {
        return n;
    }
    // Exact catalogue match
    if let Some(m) = CATALOGUE.iter().find(|(_, _, id, _, _, _)| *id == model_id) {
        return m.5;
    }
    // Provider-prefix heuristics for dynamic / uncatalogued models
    if model_id.starts_with("anthropic/") {
        if model_id.contains("opus") || model_id.contains("sonnet") {
            return 1_048_576;
        }
        return 200_000;
    }
    if model_id.starts_with("gemini/") || model_id.starts_with("google/gemini") {
        return 1_048_576;
    }
    if model_id.starts_with("openai/") {
        return 128_000;
    }
    // Groq models (fast inference, smaller windows)
    if model_id.contains("llama") {
        return 128_000;
    }
    if model_id.contains("mixtral") {
        return 32_000;
    }
    // Conservative fallback for anything else (Ollama local models, unknown)
    32_000
}

/// Returns a fast, cost-effective reasoning model from the same provider as the main model.
/// Ideal for subagents (like heuristic evaluators) that run frequently and synchronously.
pub fn fast_model_for_main_model(main_model: &str) -> String {
    let provider = main_model.split('/').next().unwrap_or(main_model);
    match provider {
        // Bug 6 fix: updated stale defaults.
        // - Haiku 4.5 (current generation, supports adaptive thinking).
        // - o4-mini (better tool-calling than gpt-4o-mini).
        // - gemini-2.0-flash (actually fast — 2.5-pro is the large reasoning model).
        "anthropic" => "anthropic/claude-haiku-4-5".to_string(),
        "openai" => "openai/o4-mini".to_string(),
        "gemini" => "gemini/gemini-2.0-flash".to_string(),
        _ => main_model.to_string(), // Fallback: use exactly what the user is using
    }
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
        for (provider, display, id, toolset, max_tok, ctx) in CATALOGUE {
            assert!(!provider.is_empty(), "empty provider for {id}");
            assert!(!display.is_empty(), "empty display for {id}");
            assert!(!id.is_empty(), "empty id");
            assert!(
                ["default", "codex", "gemini"].contains(toolset),
                "invalid toolset '{toolset}' for {id}"
            );
            assert!(*max_tok > 0, "zero max_tokens for {id}");
            assert!(*ctx > 0, "zero context_window for {id}");
        }
    }

    #[test]
    fn catalogue_ids_are_prefixed_with_provider() {
        for (provider, _, id, _, _, _) in CATALOGUE {
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
        assert_eq!(toolset_for_model("gemini/gemini-999"), "gemini");
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
        assert_eq!(max_tokens_for_model("gemini/future-model"), 8192);
    }

    #[test]
    fn max_tokens_unknown_gpt5() {
        assert_eq!(max_tokens_for_model("openai/gpt-5.5-preview"), 100_000);
    }

    #[test]
    fn max_tokens_unknown_openai() {
        assert_eq!(max_tokens_for_model("openai/future-model"), 8192);
    }

    #[test]
    fn max_tokens_completely_unknown() {
        assert_eq!(max_tokens_for_model("random/model"), 4096);
    }

    #[test]
    fn max_tokens_bare_gpt5_defaults_to_high_budget() {
        // Bare OpenAI model IDs should still align with gpt-5 defaults to avoid truncation.
        assert_eq!(max_tokens_for_model("gpt-5"), 100_000);
        assert_eq!(max_tokens_for_model("gpt-5.1-preview"), 100_000);
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
        assert_eq!(context_window_for_model("anthropic/future-claude"), 200_000);
        assert_eq!(context_window_for_model("gemini/future-gemini"), 1_048_576);
        assert_eq!(context_window_for_model("openai/future-gpt"), 128_000);
    }

    #[test]
    fn context_window_llama_model() {
        assert_eq!(context_window_for_model("groq/llama-3-70b"), 128_000);
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
}
