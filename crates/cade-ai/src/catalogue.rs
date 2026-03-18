/// Static model catalogue — known-good models per provider.
///
/// Format: (provider_key, display_name, full_model_id, toolset_label, max_tokens, context_window_tokens)
///   toolset_label: "default" | "codex" | "gemini"  (maps to Toolset::from_str)
///   context_window_tokens: model's input context length in tokens
pub const CATALOGUE: &[(&str, &str, &str, &str, u32, u32)] = &[
    // ── Anthropic ─────────────────────────────────────────────────────────────────────────────────────
    // All Claude 3+ models: 200 K token context window
    ("anthropic", "Claude Opus 4.5",   "anthropic/claude-opus-4-5",            "default", 8192,  200_000),
    ("anthropic", "Claude Sonnet 4.5", "anthropic/claude-sonnet-4-5-20250929", "default", 8192,  200_000),
    ("anthropic", "Claude Haiku 4.5",  "anthropic/claude-haiku-4-5",           "default", 8192,  200_000),
    ("anthropic", "Claude Sonnet 3.7", "anthropic/claude-3-7-sonnet-20250219", "default", 8192,  200_000),
    ("anthropic", "Claude Haiku 3.5",  "anthropic/claude-3-5-haiku-20241022",  "default", 8192,  200_000),
    ("anthropic", "Claude Opus 3",     "anthropic/claude-3-opus-20240229",     "default", 4096,  200_000),

    // ── OpenAI ────────────────────────────────────────────────────────────────────────────────────────
    ("openai",    "GPT-4.1",           "openai/gpt-4.1",                       "codex",  16384, 1_047_576),
    ("openai",    "GPT-4o",            "openai/gpt-4o",                        "codex",  16384,   128_000),
    ("openai",    "GPT-4o Mini",       "openai/gpt-4o-mini",                   "codex",  16384,   128_000),
    ("openai",    "o4 Mini",           "openai/o4-mini",                       "codex",  16384,   200_000),
    ("openai",    "o3",                "openai/o3",                            "codex",  16384,   200_000),
    ("openai",    "o3 Mini",           "openai/o3-mini",                       "codex",  16384,   200_000),

    // ── Google Gemini ─────────────────────────────────────────────────────────────────────────────────
    ("gemini",    "Gemini 2.5 Pro",    "gemini/gemini-2.5-pro",                "gemini",  8192, 1_048_576),
    ("gemini",    "Gemini 2.0 Flash",  "gemini/gemini-2.0-flash",              "gemini",  8192, 1_048_576),
    ("gemini",    "Gemini 1.5 Pro",    "gemini/gemini-1.5-pro",                "gemini",  8192, 2_097_152),
    ("gemini",    "Gemini 1.5 Flash",  "gemini/gemini-1.5-flash",              "gemini",  8192, 1_048_576),
];

/// A model entry returned by `GET /v1/models`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelEntry {
    pub provider:       String,
    pub id:             String,
    pub display_name:   String,
    pub toolset:        String,
    pub max_tokens:     u32,
    /// Model's input context window size in tokens.
    pub context_window: u32,
    /// `true` if discovered at runtime (e.g. Ollama `/api/tags`), `false` if from static catalogue.
    #[serde(default)]
    pub dynamic:        bool,
}

impl ModelEntry {
    pub fn from_catalogue(e: &(&str, &str, &str, &str, u32, u32)) -> Self {
        Self {
            provider:       e.0.to_string(),
            id:             e.2.to_string(),
            display_name:   e.1.to_string(),
            toolset:        e.3.to_string(),
            max_tokens:     e.4,
            context_window: e.5,
            dynamic:        false,
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
    } else if model_id.starts_with("gemini/") || model_id.starts_with("google/gemini") || model_id.starts_with("openai/") {
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
    if let Ok(val) = std::env::var("CADE_CONTEXT_BUDGET") {
        if let Ok(n) = val.parse::<u32>() {
            return n;
        }
    }
    // Exact catalogue match
    if let Some(m) = CATALOGUE.iter().find(|(_, _, id, _, _, _)| *id == model_id) {
        return m.5;
    }
    // Provider-prefix heuristics for dynamic / uncatalogued models
    if model_id.starts_with("anthropic/") { return 200_000; }
    if model_id.starts_with("gemini/") || model_id.starts_with("google/gemini") { return 1_048_576; }
    if model_id.starts_with("openai/")    { return 128_000; }
    // Groq models (fast inference, smaller windows)
    if model_id.contains("llama")         { return 128_000; }
    if model_id.contains("mixtral")       { return 32_000; }
    // Conservative fallback for anything else (Ollama local models, unknown)
    32_000
}

// ── Pricing ───────────────────────────────────────────────────────────────────

/// Per-1M-token USD rates (approximate — check provider docs for current prices).
pub struct ModelPricing {
    pub input:       f64,  // $/1M input tokens
    pub output:      f64,  // $/1M output tokens
    pub cache_read:  f64,  // $/1M cache-read tokens
    pub cache_write: f64,  // $/1M cache-write tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CATALOGUE ─────────────────────────────────────────────────────────

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

    // ── ModelEntry::from_catalogue ────────────────────────────────────────

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

    // ── toolset_for_model ─────────────────────────────────────────────────

    #[test]
    fn toolset_known_models() {
        assert_eq!(toolset_for_model("anthropic/claude-sonnet-4-5-20250929"), "default");
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

    // ── max_tokens_for_model ──────────────────────────────────────────────

    #[test]
    fn max_tokens_known_models() {
        assert_eq!(max_tokens_for_model("anthropic/claude-sonnet-4-5-20250929"), 8192);
        assert_eq!(max_tokens_for_model("openai/gpt-4o"), 16384);
    }

    #[test]
    fn max_tokens_unknown_gemini() {
        assert_eq!(max_tokens_for_model("gemini/future-model"), 8192);
    }

    #[test]
    fn max_tokens_unknown_openai() {
        assert_eq!(max_tokens_for_model("openai/future-model"), 8192);
    }

    #[test]
    fn max_tokens_completely_unknown() {
        assert_eq!(max_tokens_for_model("random/model"), 4096);
    }

    // ── context_window_for_model ──────────────────────────────────────────

    #[test]
    fn context_window_known_models() {
        assert_eq!(context_window_for_model("anthropic/claude-sonnet-4-5-20250929"), 200_000);
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

    // ── pricing_for_model ─────────────────────────────────────────────────

    #[test]
    fn pricing_claude_sonnet() {
        let p = pricing_for_model("anthropic/claude-sonnet-4-5-20250929");
        assert!(p.input > 0.0);
        assert!(p.output > 0.0);
        assert!(p.cache_read > 0.0);
        assert!(p.cache_write > 0.0);
    }

    #[test]
    fn pricing_gpt_4o() {
        let p = pricing_for_model("openai/gpt-4o");
        assert!(p.input > 0.0);
        assert!(p.output > 0.0);
    }

    #[test]
    fn pricing_gemini_25_pro() {
        let p = pricing_for_model("gemini/gemini-2.5-pro");
        assert!(p.input > 0.0);
        assert!(p.output > 0.0);
    }

    #[test]
    fn pricing_unknown_model_zero() {
        let p = pricing_for_model("random/model-xyz");
        assert_eq!(p.input, 0.0);
        assert_eq!(p.output, 0.0);
        assert_eq!(p.cache_read, 0.0);
        assert_eq!(p.cache_write, 0.0);
    }

    #[test]
    fn pricing_provider_prefix_fallback() {
        // Unknown anthropic model should get the fallback anthropic pricing
        let p = pricing_for_model("anthropic/future-model");
        assert!(p.input > 0.0);
    }
}

/// Returns approximate per-token pricing for a model.
/// Uses pattern matching on model ID; unknown models get zero rates.
pub fn pricing_for_model(model_id: &str) -> ModelPricing {
    match model_id {
        // ── Anthropic ─────────────────────────────────────────────────────────
        m if m.contains("claude-sonnet-4") || m.contains("claude-3-7-sonnet")
            || m.contains("claude-sonnet-4-6") =>
            ModelPricing { input:  3.00, output: 15.00, cache_read: 0.30,  cache_write:  3.75 },
        m if m.contains("claude-haiku-4") || m.contains("claude-3-5-haiku") =>
            ModelPricing { input:  0.80, output:  4.00, cache_read: 0.08,  cache_write:  1.00 },
        m if m.contains("claude-opus-4") || m.contains("claude-3-opus") =>
            ModelPricing { input: 15.00, output: 75.00, cache_read: 1.50,  cache_write: 18.75 },
        // ── OpenAI ────────────────────────────────────────────────────────────
        m if m.contains("gpt-4.1") && !m.contains("mini") =>
            ModelPricing { input:  2.00, output:  8.00, cache_read: 0.50,  cache_write:  0.0 },
        m if m.contains("gpt-4o-mini") =>
            ModelPricing { input:  0.15, output:  0.60, cache_read: 0.075, cache_write:  0.0 },
        m if m.contains("gpt-4o") =>
            ModelPricing { input:  2.50, output: 10.00, cache_read: 1.25,  cache_write:  0.0 },
        m if m.contains("o4-mini") || m.contains("o3-mini") =>
            ModelPricing { input:  1.10, output:  4.40, cache_read: 0.275, cache_write:  0.0 },
        m if m.contains("/o3") && !m.contains("mini") =>
            ModelPricing { input: 10.00, output: 40.00, cache_read: 2.50,  cache_write:  0.0 },
        // ── Google Gemini ─────────────────────────────────────────────────────
        m if m.contains("gemini-2.5-pro") =>
            ModelPricing { input:  1.25, output: 10.00, cache_read: 0.31,  cache_write:  0.0 },
        m if m.contains("gemini-2.0-flash") || m.contains("gemini-1.5-flash") =>
            ModelPricing { input:  0.10, output:  0.40, cache_read: 0.025, cache_write:  0.0 },
        m if m.contains("gemini-1.5-pro") =>
            ModelPricing { input:  1.25, output:  5.00, cache_read: 0.31,  cache_write:  0.0 },
        // ── Provider-prefix fallbacks ─────────────────────────────────────────
        m if m.starts_with("anthropic/") =>
            ModelPricing { input:  3.00, output: 15.00, cache_read: 0.30,  cache_write:  3.75 },
        m if m.starts_with("openai/") =>
            ModelPricing { input:  2.50, output: 10.00, cache_read: 1.25,  cache_write:  0.0 },
        m if m.starts_with("gemini/") || m.starts_with("gemini-") =>
            ModelPricing { input:  1.25, output: 10.00, cache_read: 0.31,  cache_write:  0.0 },
        _ =>
            ModelPricing { input:  0.0,  output:  0.0,  cache_read: 0.0,   cache_write:  0.0 },
    }
}
