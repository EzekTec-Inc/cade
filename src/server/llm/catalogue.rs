/// Static model catalogue — known-good models per provider.
///
/// Format: (provider_key, display_name, full_model_id, toolset_label, max_tokens)
///   toolset_label: "default" | "codex" | "gemini"  (maps to Toolset::from_str)
pub const CATALOGUE: &[(&str, &str, &str, &str, u32)] = &[
    // ── Anthropic ────────────────────────────────────────────────────────────
    ("anthropic", "Claude Opus 4.5",    "anthropic/claude-opus-4-5",              "default", 8192),
    ("anthropic", "Claude Sonnet 4.5",  "anthropic/claude-sonnet-4-5-20250929",   "default", 8192),
    ("anthropic", "Claude Haiku 4.5",   "anthropic/claude-haiku-4-5",             "default", 8192),
    ("anthropic", "Claude Sonnet 3.7",  "anthropic/claude-3-7-sonnet-20250219",   "default", 8192),
    ("anthropic", "Claude Haiku 3.5",   "anthropic/claude-3-5-haiku-20241022",    "default", 8192),
    ("anthropic", "Claude Opus 3",      "anthropic/claude-3-opus-20240229",       "default", 4096),

    // ── OpenAI ───────────────────────────────────────────────────────────────
    ("openai",    "GPT-4.1",            "openai/gpt-4.1",                         "codex", 16384),
    ("openai",    "GPT-4o",             "openai/gpt-4o",                          "codex", 16384),
    ("openai",    "GPT-4o Mini",        "openai/gpt-4o-mini",                     "codex", 16384),
    ("openai",    "o4 Mini",            "openai/o4-mini",                         "codex", 16384),
    ("openai",    "o3",                 "openai/o3",                              "codex", 16384),
    ("openai",    "o3 Mini",            "openai/o3-mini",                         "codex", 16384),

    // ── Google Gemini ─────────────────────────────────────────────────────────
    ("gemini",    "Gemini 2.5 Pro",     "gemini/gemini-2.5-pro",                  "gemini", 8192),
    ("gemini",    "Gemini 2.0 Flash",   "gemini/gemini-2.0-flash",                "gemini", 8192),
    ("gemini",    "Gemini 1.5 Pro",     "gemini/gemini-1.5-pro",                  "gemini", 8192),
    ("gemini",    "Gemini 1.5 Flash",   "gemini/gemini-1.5-flash",                "gemini", 8192),
];

/// A model entry returned by `GET /v1/models`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelEntry {
    pub provider:     String,
    pub id:           String,
    pub display_name: String,
    pub toolset:      String,
    pub max_tokens:   u32,
    /// `true` if discovered at runtime (e.g. Ollama `/api/tags`), `false` if from static catalogue.
    #[serde(default)]
    pub dynamic:      bool,
}

impl ModelEntry {
    pub fn from_catalogue(e: &(&str, &str, &str, &str, u32)) -> Self {
        Self {
            provider:     e.0.to_string(),
            id:           e.2.to_string(),
            display_name: e.1.to_string(),
            toolset:      e.3.to_string(),
            max_tokens:   e.4,
            dynamic:      false,
        }
    }
}

/// Determine the toolset for a specific model ID. Defaults to "default" if unknown.
pub fn toolset_for_model(model_id: &str) -> String {
    if let Some(m) = CATALOGUE.iter().find(|(_, _, id, _, _)| *id == model_id) {
        m.3.to_string()
    } else if model_id.starts_with("gemini/") {
        "gemini".to_string()
    } else {
        "default".to_string() // Groq, OpenRouter, Ollama default to generic openai/anthropic style
    }
}

/// Determine the max tokens for a specific model ID. Defaults to 4096 if unknown.
pub fn max_tokens_for_model(model_id: &str) -> u32 {
    if let Some(m) = CATALOGUE.iter().find(|(_, _, id, _, _)| *id == model_id) {
        m.4
    } else if model_id.starts_with("gemini/") || model_id.starts_with("openai/") {
        8192
    } else {
        4096 // Safe default for older models / unknown providers
    }
}
