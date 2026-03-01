pub mod anthropic;
pub mod catalogue;
pub mod gemini;
pub mod ollama;
pub mod openai;

pub use catalogue::{ModelEntry, CATALOGUE};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio_stream::Stream;

use crate::server::config::ServerConfig;

// ── Request / Response types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,    // "system" | "user" | "assistant" | "tool"
    pub content: String, // text or JSON (for tool results)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<LlmToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<Value>, // JSON schemas
    pub max_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<LlmToolCall>,
    pub finish_reason: String,
}

/// A chunk from a streaming response
#[derive(Debug, Clone)]
pub enum StreamChunk {
    Text(String),
    ToolCall(LlmToolCall),
    Done,
}

// ── Provider trait ────────────────────────────────────────────────────────────

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse>;
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>>;
}

/// Strip optional `provider/` prefix from a model handle.
/// e.g. `"anthropic/claude-sonnet-4-5-20250929"` → `"claude-sonnet-4-5-20250929"`
pub fn bare_model(model: &str) -> &str {
    if let Some(pos) = model.find('/') {
        &model[pos + 1..]
    } else {
        model
    }
}

// ── LLM Router ────────────────────────────────────────────────────────────────
//
// Owns all configured providers and selects the right one at request time
// based on the `provider/model` prefix in `CompletionRequest.model`.
// This lets /model switching work transparently without a server restart.

/// A known OpenAI-compatible provider preset.
///
/// - `env_vars`: env var names to scan for an API key (first non-empty wins)
/// - `chat_url`: chat completions endpoint
/// - `models_url`: live model listing endpoint (`None` → not supported by this provider)
#[derive(Debug, Clone)]
pub struct PresetDef {
    pub name:       &'static str,
    pub env_vars:   &'static [&'static str],
    pub chat_url:   &'static str,
    pub models_url: Option<&'static str>,
}

/// All known OpenAI-compatible preset providers with their auto-detection env vars.
pub const PRESET_PROVIDERS: &[PresetDef] = &[
    PresetDef {
        name:       "openrouter",
        env_vars:   &["OPENROUTER_API_KEY"],
        chat_url:   "https://openrouter.ai/api/v1/chat/completions",
        models_url: Some("https://openrouter.ai/api/v1/models"),
    },
    PresetDef {
        name:       "groq",
        env_vars:   &["GROQ_API_KEY"],
        chat_url:   "https://api.groq.com/openai/v1/chat/completions",
        models_url: Some("https://api.groq.com/openai/v1/models"),
    },
    PresetDef {
        name:       "together",
        env_vars:   &["TOGETHER_API_KEY", "TOGETHER_AI_API_KEY"],
        chat_url:   "https://api.together.xyz/v1/chat/completions",
        models_url: Some("https://api.together.xyz/v1/models"),
    },
    PresetDef {
        name:       "fireworks",
        env_vars:   &["FIREWORKS_API_KEY"],
        chat_url:   "https://api.fireworks.ai/inference/v1/chat/completions",
        models_url: Some("https://api.fireworks.ai/inference/v1/models"),
    },
    PresetDef {
        name:       "deepinfra",
        env_vars:   &["DEEPINFRA_API_KEY"],
        chat_url:   "https://api.deepinfra.com/v1/openai/chat/completions",
        models_url: Some("https://api.deepinfra.com/v1/openai/models"),
    },
];

/// Backward-compat alias for providers.rs and repl.rs /connect preset lookup.
/// Derived from PRESET_PROVIDERS so there is a single source of truth.
pub fn openai_compat_presets() -> Vec<(&'static str, &'static str)> {
    PRESET_PROVIDERS.iter()
        .map(|p| (p.name, p.chat_url))
        .collect()
}

/// Deprecated constant kept for compile-time references — use `openai_compat_presets()` instead.
#[deprecated(note = "use PRESET_PROVIDERS or openai_compat_presets()")]
pub const OPENAI_COMPAT_PRESETS: &[(&str, &str)] = &[
    ("openrouter", "https://openrouter.ai/api/v1/chat/completions"),
    ("together",   "https://api.together.xyz/v1/chat/completions"),
    ("groq",       "https://api.groq.com/openai/v1/chat/completions"),
    ("fireworks",  "https://api.fireworks.ai/inference/v1/chat/completions"),
    ("deepinfra",  "https://api.deepinfra.com/v1/openai/chat/completions"),
];

pub struct LlmRouter {
    providers:        std::collections::HashMap<String, Arc<dyn LlmProvider>>,
    /// API keys stored per provider name — used for live model listing calls.
    provider_keys:    std::collections::HashMap<String, String>,
    default_provider: String,
    /// Base URL for the Ollama instance (used by /v1/models to query /api/tags).
    pub ollama_base_url: String,
}

impl LlmRouter {
    pub fn build(config: &ServerConfig) -> Self {
        let mut providers: std::collections::HashMap<String, Arc<dyn LlmProvider>> =
            std::collections::HashMap::new();
        let mut provider_keys: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut default_provider = config.llm_provider.to_string();

        // ── Core providers (from ServerConfig) ────────────────────────────────
        if let Some(key) = &config.anthropic_api_key {
            providers.insert(
                "anthropic".to_string(),
                Arc::new(anthropic::AnthropicProvider::new(key.clone())),
            );
            provider_keys.insert("anthropic".to_string(), key.clone());
        }
        if let Some(key) = &config.openai_api_key {
            providers.insert(
                "openai".to_string(),
                Arc::new(openai::OpenAiProvider::new(key.clone(), None)),
            );
            provider_keys.insert("openai".to_string(), key.clone());
        }
        if let Some(key) = &config.google_api_key {
            providers.insert(
                "gemini".to_string(),
                Arc::new(gemini::GeminiProvider::new(key.clone())),
            );
            providers.insert(
                "google".to_string(),
                Arc::new(gemini::GeminiProvider::new(key.clone())),
            );
            provider_keys.insert("gemini".to_string(), key.clone());
            provider_keys.insert("google".to_string(), key.clone());
        }
        // Ollama is always available as a local fallback
        providers.insert(
            "ollama".to_string(),
            Arc::new(ollama::OllamaProvider::new(config.ollama_base_url.clone())),
        );

        // ── Preset providers auto-detected from env vars ───────────────────────
        for preset in PRESET_PROVIDERS {
            // Skip if already registered (avoid overwriting a core provider)
            if providers.contains_key(preset.name) { continue; }
            let key = preset.env_vars.iter()
                .find_map(|var| std::env::var(var).ok().filter(|k| !k.is_empty()));
            if let Some(key) = key {
                tracing::info!(
                    "Auto-detected provider '{}' from env var '{}'",
                    preset.name,
                    preset.env_vars.iter().find(|v| std::env::var(v).is_ok()).unwrap_or(&"?")
                );
                providers.insert(
                    preset.name.to_string(),
                    Arc::new(openai::OpenAiProvider::new(key.clone(), Some(preset.chat_url.to_string()))),
                );
                provider_keys.insert(preset.name.to_string(), key);
            }
        }

        // Ensure the configured default is actually available; fall back gracefully
        if !providers.contains_key(&default_provider) {
            if let Some(first) = providers.keys().next() {
                default_provider = first.clone();
            }
        }

        Self {
            providers,
            provider_keys,
            default_provider,
            ollama_base_url: config.ollama_base_url.clone(),
        }
    }

    /// Add or replace a provider at runtime (hot-reload via /connect).
    /// `api_key` is stored for live model listing; pass `None` if not applicable.
    pub fn add_provider(&mut self, name: String, provider: Arc<dyn LlmProvider>) {
        tracing::info!("Provider hot-loaded: {name}");
        self.providers.insert(name, provider);
    }

    /// Add a provider with its API key (used by /connect when key is known).
    pub fn add_provider_with_key(&mut self, name: String, provider: Arc<dyn LlmProvider>, key: String) {
        tracing::info!("Provider hot-loaded: {name}");
        self.providers.insert(name.clone(), provider);
        if !key.is_empty() {
            self.provider_keys.insert(name, key);
        }
    }

    /// Remove a provider at runtime (via /disconnect).
    /// Returns false if the name was not found.
    pub fn remove_provider(&mut self, name: &str) -> bool {
        if self.providers.remove(name).is_some() {
            self.provider_keys.remove(name);
            tracing::info!("Provider removed: {name}");
            // Reset default if we just removed it
            if self.default_provider == name {
                self.default_provider = self.providers.keys()
                    .next().cloned().unwrap_or_default();
            }
            true
        } else {
            false
        }
    }

    /// Names of all currently registered providers.
    pub fn provider_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Fetch live model lists from all providers that support it (Ollama + preset providers
    /// with a `models_url`). Queries are run **concurrently** — one task per provider.
    /// Results are returned as `ModelEntry { dynamic: true }`.
    ///
    /// Called by `GET /v1/models` to populate the dynamic section of the model picker.
    pub async fn list_dynamic_models(&self) -> Vec<catalogue::ModelEntry> {
        use catalogue::ModelEntry;
        use futures::future::join_all;

        // Build one future per provider that supports live model listing
        type ModelFut = std::pin::Pin<Box<dyn std::future::Future<Output = Vec<ModelEntry>> + Send>>;
        let mut tasks: Vec<ModelFut> = Vec::new();

        for name in self.providers.keys() {
            let key = self.provider_keys.get(name.as_str()).cloned().unwrap_or_default();

            match name.as_str() {
                // ── Local Ollama ─────────────────────────────────────────────
                "ollama" => {
                    let url = self.ollama_base_url.clone();
                    tasks.push(Box::pin(async move {
                        let ol = ollama::OllamaProvider::new(url);
                        ol.list_models().await
                            .into_iter()
                            .map(|m| ModelEntry {
                                provider:     "ollama".into(),
                                id:           format!("ollama/{m}"),
                                display_name: m,
                                toolset:      "default".into(),
                                dynamic:      true,
                            })
                            .collect()
                    }));
                }

                // ── Anthropic — live /v1/models, fallback to catalogue ───────
                "anthropic" => {
                    tasks.push(Box::pin(async move {
                        let live = anthropic::fetch_anthropic_models(&key).await;
                        if live.is_empty() {
                            // Provider is configured but endpoint unreachable — use catalogue
                            CATALOGUE.iter()
                                .filter(|(p, ..)| *p == "anthropic")
                                .map(catalogue::ModelEntry::from_catalogue)
                                .collect()
                        } else {
                            live.into_iter().map(|(id, display)| ModelEntry {
                                provider:     "anthropic".into(),
                                id:           format!("anthropic/{id}"),
                                display_name: display,
                                toolset:      "default".into(),
                                dynamic:      true,
                            }).collect()
                        }
                    }));
                }

                // ── OpenAI — live /v1/models (chat only), fallback to catalogue
                "openai" => {
                    tasks.push(Box::pin(async move {
                        let ids = openai::fetch_openai_chat_models(&key).await;
                        if ids.is_empty() {
                            CATALOGUE.iter()
                                .filter(|(p, ..)| *p == "openai")
                                .map(catalogue::ModelEntry::from_catalogue)
                                .collect()
                        } else {
                            ids.into_iter().map(|id| ModelEntry {
                                provider:     "openai".into(),
                                id:           format!("openai/{id}"),
                                display_name: id.clone(),
                                toolset:      "codex".into(),
                                dynamic:      true,
                            }).collect()
                        }
                    }));
                }

                // ── Gemini — live models list, fallback to catalogue ─────────
                "gemini" | "google" => {
                    let n = name.clone();
                    tasks.push(Box::pin(async move {
                        let live = gemini::fetch_gemini_models(&key).await;
                        if live.is_empty() {
                            CATALOGUE.iter()
                                .filter(|(p, ..)| *p == "gemini")
                                .map(catalogue::ModelEntry::from_catalogue)
                                .collect()
                        } else {
                            live.into_iter().map(|(id, display)| ModelEntry {
                                provider:     n.clone(),
                                id:           format!("{n}/{id}"),
                                display_name: display,
                                toolset:      "gemini".into(),
                                dynamic:      true,
                            }).collect()
                        }
                    }));
                }

                // ── Preset providers (Groq, OpenRouter, etc.) ───────────────
                _ => {
                    if let Some(preset) = PRESET_PROVIDERS.iter().find(|p| p.name == name.as_str()) {
                        if let Some(models_url) = preset.models_url {
                            let n   = name.clone();
                            let url = models_url.to_string();
                            tasks.push(Box::pin(async move {
                                openai::fetch_model_ids(&url, &key).await
                                    .into_iter()
                                    .map(|id| ModelEntry {
                                        provider:     n.clone(),
                                        id:           format!("{n}/{id}"),
                                        display_name: id,
                                        toolset:      "default".into(),
                                        dynamic:      true,
                                    })
                                    .collect()
                            }));
                        }
                    }
                }
            }
        }

        let mut out: Vec<ModelEntry> = join_all(tasks).await.into_iter().flatten().collect();
        out.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.id.cmp(&b.id)));
        out
    }

    /// Build an `Arc<dyn LlmProvider>` from a DB `ProviderRow`.
    pub fn provider_from_row(
        row: &crate::server::storage::sqlite::ProviderRow,
        config: &ServerConfig,
    ) -> Option<Arc<dyn LlmProvider>> {
        match row.kind.as_str() {
            "anthropic" => {
                let key = row.api_key.clone().or_else(|| config.anthropic_api_key.clone())?;
                Some(Arc::new(anthropic::AnthropicProvider::new(key)))
            }
            "openai" => {
                let key = row.api_key.clone().or_else(|| config.openai_api_key.clone())?;
                Some(Arc::new(openai::OpenAiProvider::new(key, row.base_url.clone())))
            }
            "gemini" => {
                let key = row.api_key.clone().or_else(|| config.google_api_key.clone())?;
                Some(Arc::new(gemini::GeminiProvider::new(key)))
            }
            "ollama" => {
                let base = row.base_url.clone()
                    .unwrap_or_else(|| config.ollama_base_url.clone());
                Some(Arc::new(ollama::OllamaProvider::new(base)))
            }
            "openai-compatible" => {
                let key = row.api_key.clone().unwrap_or_default();
                let url = row.base_url.clone()?;
                Some(Arc::new(openai::OpenAiProvider::new(key, Some(url))))
            }
            _ => None,
        }
    }

    /// Select provider and bare model name for a `provider/model` or bare `model` string.
    ///
    /// Resolution order:
    ///   1. Explicit `provider/model` prefix — error if prefix unknown
    ///   2. Auto-detect provider from well-known model name patterns — error if provider not configured
    ///   3. Fall back to the configured default provider (only for truly unknown model names)
    fn pick(&self, model: &str) -> anyhow::Result<(Arc<dyn LlmProvider>, String)> {
        // 1. Explicit prefix: `gemini/gemini-2.5-pro`
        if let Some(slash) = model.find('/') {
            let prefix = &model[..slash];
            let bare   = model[slash + 1..].to_string();
            return self.providers
                .get(prefix)
                .map(|p| (Arc::clone(p), bare))
                .ok_or_else(|| anyhow::anyhow!(
                    "Provider '{}' is not configured. Run /connect {} to add it.",
                    prefix, prefix
                ));
        }

        // 2. Infer provider from model name pattern
        if let Some(prefix) = infer_provider_prefix(model) {
            return self.providers
                .get(prefix)
                .map(|p| (Arc::clone(p), model.to_string()))
                .ok_or_else(|| anyhow::anyhow!(
                    "Model '{}' requires the '{}' provider. Run /connect {} to add it.",
                    model, prefix, prefix
                ));
        }

        // 3. Truly unknown model — use the default provider
        self.providers
            .get(&self.default_provider)
            .map(|p| (Arc::clone(p), model.to_string()))
            .ok_or_else(|| anyhow::anyhow!("No LLM provider available"))
    }

    /// Validate that the given model string can be routed.
    pub fn validate_model(&self, model: &str) -> anyhow::Result<()> {
        self.pick(model).map(|_| ())
    }
}

/// Infer the provider key from well-known model name prefixes.
/// Returns e.g. "anthropic", "openai", "gemini", "ollama", or None.
fn infer_provider_prefix(model: &str) -> Option<&'static str> {
    let m = model.to_lowercase();
    if m.starts_with("claude") {
        Some("anthropic")
    } else if m.starts_with("gemini") {
        Some("gemini")
    } else if m.starts_with("gpt-")
        || m.starts_with("o1-")
        || m.starts_with("o3-")
        || m.starts_with("o4-")
        || m == "gpt-4o"
        || m == "gpt-4o-mini"
    {
        Some("openai")
    } else if m.starts_with("llama")
        || m.starts_with("mistral")
        || m.starts_with("phi")
        || m.starts_with("qwen")
        || m.starts_with("deepseek")
    {
        Some("ollama")
    } else {
        None
    }
}

#[async_trait::async_trait]
impl LlmProvider for LlmRouter {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let (provider, bare_model) = self.pick(&req.model)?;
        let routed = CompletionRequest { model: bare_model, ..req.clone() };
        provider.complete(&routed).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>> {
        let (provider, bare_model) = self.pick(&req.model)?;
        let routed = CompletionRequest { model: bare_model, ..req.clone() };
        provider.stream(&routed).await
    }
}

// ── Factory (kept for compatibility) ──────────────────────────────────────────

pub fn make_provider(config: &ServerConfig) -> Result<Arc<dyn LlmProvider>> {
    Ok(Arc::new(LlmRouter::build(config)))
}
