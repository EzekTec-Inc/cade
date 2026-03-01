/// Static model catalogue — known-good models per provider.
///
/// Format: (provider_key, display_name, full_model_id, toolset_label)
///   toolset_label: "default" | "codex" | "gemini"  (maps to Toolset::from_str)
pub const CATALOGUE: &[(&str, &str, &str, &str)] = &[
    // ── Anthropic ────────────────────────────────────────────────────────────
    ("anthropic", "Claude Opus 4.5",    "anthropic/claude-opus-4-5",              "default"),
    ("anthropic", "Claude Sonnet 4.5",  "anthropic/claude-sonnet-4-5-20250929",   "default"),
    ("anthropic", "Claude Haiku 4.5",   "anthropic/claude-haiku-4-5",             "default"),
    ("anthropic", "Claude Sonnet 3.7",  "anthropic/claude-3-7-sonnet-20250219",   "default"),
    ("anthropic", "Claude Haiku 3.5",   "anthropic/claude-3-5-haiku-20241022",    "default"),
    ("anthropic", "Claude Opus 3",      "anthropic/claude-3-opus-20240229",       "default"),

    // ── OpenAI ───────────────────────────────────────────────────────────────
    ("openai",    "GPT-4.1",            "openai/gpt-4.1",                         "codex"),
    ("openai",    "GPT-4o",             "openai/gpt-4o",                          "codex"),
    ("openai",    "GPT-4o Mini",        "openai/gpt-4o-mini",                     "codex"),
    ("openai",    "o4 Mini",            "openai/o4-mini",                         "codex"),
    ("openai",    "o3",                 "openai/o3",                              "codex"),
    ("openai",    "o3 Mini",            "openai/o3-mini",                         "codex"),

    // ── Google Gemini ─────────────────────────────────────────────────────────
    ("gemini",    "Gemini 2.5 Pro",     "gemini/gemini-2.5-pro",                  "gemini"),
    ("gemini",    "Gemini 2.0 Flash",   "gemini/gemini-2.0-flash",                "gemini"),
    ("gemini",    "Gemini 1.5 Pro",     "gemini/gemini-1.5-pro",                  "gemini"),
    ("gemini",    "Gemini 1.5 Flash",   "gemini/gemini-1.5-flash",                "gemini"),
];

/// A model entry returned by `GET /v1/models`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelEntry {
    pub provider:     String,
    pub id:           String,
    pub display_name: String,
    pub toolset:      String,
    /// `true` if discovered at runtime (e.g. Ollama `/api/tags`), `false` if from static catalogue.
    #[serde(default)]
    pub dynamic:      bool,
}

impl ModelEntry {
    pub fn from_catalogue(e: &(&str, &str, &str, &str)) -> Self {
        Self {
            provider:     e.0.to_string(),
            id:           e.2.to_string(),
            display_name: e.1.to_string(),
            toolset:      e.3.to_string(),
            dynamic:      false,
        }
    }
}
