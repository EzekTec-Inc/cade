// region:    --- Modules

mod error;

pub use error::{Error, Result};

pub mod anthropic;
pub mod catalogue;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod registry;
pub mod provider_registry;

pub use catalogue::{CATALOGUE, ModelEntry};
pub use registry::{ModelPricing, ModelRegistry};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio_stream::Stream;

// endregion: --- Modules

#[derive(Debug, Clone)]
pub struct AiConfig {
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub ollama_base_url: String,
    pub llm_provider: String,
}

// -- Request / Response types

/// A base64-encoded image attached to a user message.
///
/// Stored as JSON in the SQLite `content` column alongside the text so that
/// the full conversation history — including past images — is available when
/// building LLM context for subsequent turns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageImage {
    /// IANA media type: `"image/png"`, `"image/jpeg"`, `"image/gif"`, `"image/webp"`.
    pub media_type: String,
    /// Base64-encoded image bytes (standard alphabet, no line-breaks).
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,    // "system" | "user" | "assistant" | "tool"
    pub content: String, // text or JSON (for tool results)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<LlmToolCall>>,
    /// Inline images attached to this message (user messages only).
    /// When present the provider serialises a multi-part content array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<MessageImage>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    /// Gemini-specific opaque token that must be echoed back verbatim in the
    /// conversation history when the model used thinking/reasoning.  Absent
    /// for all other providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<Value>, // JSON schemas
    pub max_tokens: u32,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<LlmToolCall>,
    pub finish_reason: String,
}

/// Token usage reported by the LLM at the end of a completion.
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    /// Tokens written into the prompt cache on this request (first cache miss).
    /// Non-zero only on Anthropic; billed at 1.25× normal input rate.
    pub cache_write_tokens: u32,
    /// The model that produced this usage (e.g. "gemini/gemini-2.5-pro").
    pub model: String,
}

/// A chunk from a streaming response
#[derive(Debug, Clone)]
pub enum StreamChunk {
    Text(String),
    /// Reasoning/thinking content emitted before the assistant response.
    Reasoning(String),
    ToolCall(LlmToolCall),
    /// Token usage reported at end of stream (before Done).
    Usage(TokenUsage),
    /// Provider-specific finish reason (e.g. "max_tokens", "length", "SAFETY").
    FinishReason(String),
    Done,
}

// -- Provider trait

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse>;
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>>;
}

// -- Retry helper

/// Which HTTP status codes are worth retrying (transient / rate-limit errors).
/// 400, 401, 403, 404 are permanent — fail fast.
pub fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

/// Retry an async fallible operation with exponential backoff.
///
/// - `max_attempts`: total tries (1 = no retry)
/// - `base_delay`:   wait before 2nd attempt
/// - Multiplier:     2× per attempt, capped at 8 s
/// - Retries on:     connection errors OR `is_retryable_status()`
///
/// The closure receives the attempt number (1-based) and returns a Future.
pub async fn retry_with_backoff<F, Fut, T>(
    op_name: &str,
    max_attempts: u32,
    base_delay: std::time::Duration,
    mut f: F,
) -> Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_err = crate::Error::custom("retry_with_backoff: no attempts made");
    for attempt in 1..=max_attempts {
        match f(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                // Check if this is a retryable HTTP status embedded in the error message.
                // providers call provider_error() which formats as "Provider 429: ..."
                let retryable = is_retryable_error(&e);
                if attempt < max_attempts && retryable {
                    let delay = std::cmp::min(
                        base_delay * 2u32.pow(attempt - 1),
                        std::time::Duration::from_secs(8),
                    );
                    tracing::warn!(
                        "{op_name}: attempt {attempt}/{max_attempts} failed ({e:#}), \
                         retrying in {}ms…",
                        delay.as_millis()
                    );
                    tokio::time::sleep(delay).await;
                    last_err = e;
                } else {
                    return Err(e);
                }
            }
        }
    }
    Err(last_err)
}

/// Returns true if the error looks like a transient / rate-limit failure.
/// Checks for connection errors (reqwest) and for retryable status codes
/// in Provider errors.
fn is_retryable_error(e: &Error) -> bool {
    // Check embedded reqwest errors
    if let Error::Reqwest(re) = e {
        if re.is_connect() || re.is_timeout() || re.is_request() {
            return true;
        }
        if let Some(status) = re.status() {
            return is_retryable_status(status);
        }
    }
    // Check structured provider errors
    if let Error::Provider { status, .. } = e
        && let Ok(status_code) = reqwest::StatusCode::from_u16(*status)
    {
        return is_retryable_status(status_code);
    }
    false
}

/// Extract a human-readable error message from a provider's JSON error body.
///
/// All major providers (Anthropic, OpenAI, Gemini) use `{"error":{"message":"..."}}`.
/// Falls back to the raw text if the body isn't JSON or lacks that field.
///
/// Returns a `crate::Error` ready to propagate with `return Err(...)`.
pub fn provider_error(provider: &str, status: reqwest::StatusCode, body: &str) -> Error {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or_else(|| body.trim().to_string());
    Error::Provider {
        status: status.as_u16(),
        msg: format!("{provider} {status}: {msg}"),
    }
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

// -- LLM Router
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
    pub name: &'static str,
    pub env_vars: &'static [&'static str],
    pub chat_url: &'static str,
    pub models_url: Option<&'static str>,
}

/// All known OpenAI-compatible preset providers with their auto-detection env vars.
pub const PRESET_PROVIDERS: &[PresetDef] = &[
    PresetDef {
        name: "openrouter",
        env_vars: &["OPENROUTER_API_KEY"],
        chat_url: "https://openrouter.ai/api/v1/chat/completions",
        models_url: Some("https://openrouter.ai/api/v1/models"),
    },
    PresetDef {
        name: "groq",
        env_vars: &["GROQ_API_KEY"],
        chat_url: "https://api.groq.com/openai/v1/chat/completions",
        models_url: Some("https://api.groq.com/openai/v1/models"),
    },
    PresetDef {
        name: "together",
        env_vars: &["TOGETHER_API_KEY", "TOGETHER_AI_API_KEY"],
        chat_url: "https://api.together.xyz/v1/chat/completions",
        models_url: Some("https://api.together.xyz/v1/models"),
    },
    PresetDef {
        name: "fireworks",
        env_vars: &["FIREWORKS_API_KEY"],
        chat_url: "https://api.fireworks.ai/inference/v1/chat/completions",
        models_url: Some("https://api.fireworks.ai/inference/v1/models"),
    },
    PresetDef {
        name: "deepinfra",
        env_vars: &["DEEPINFRA_API_KEY"],
        chat_url: "https://api.deepinfra.com/v1/openai/chat/completions",
        models_url: Some("https://api.deepinfra.com/v1/openai/models"),
    },
];

/// Backward-compat alias for providers.rs and repl.rs /connect preset lookup.
/// Derived from PRESET_PROVIDERS so there is a single source of truth.
pub fn openai_compat_presets() -> Vec<(&'static str, &'static str)> {
    PRESET_PROVIDERS
        .iter()
        .map(|p| (p.name, p.chat_url))
        .collect()
}


pub struct LlmRouter {
    providers: std::collections::HashMap<String, Arc<dyn LlmProvider>>,
    /// API keys stored per provider name — used for live model listing calls.
    provider_keys: std::collections::HashMap<String, String>,
    default_provider: String,
    /// Base URL for the Ollama instance (used by /v1/models to query /api/tags).
    pub ollama_base_url: String,
}

impl LlmRouter {
    pub fn build(config: &AiConfig) -> Self {
        let mut providers: std::collections::HashMap<String, Arc<dyn LlmProvider>> =
            std::collections::HashMap::new();
        let mut provider_keys: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut default_provider = config.llm_provider.to_string();

        // -- Core providers (from AiConfig)
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

        // -- Preset providers auto-detected from env vars
        for preset in PRESET_PROVIDERS {
            // Skip if already registered (avoid overwriting a core provider)
            if providers.contains_key(preset.name) {
                continue;
            }
            let key = preset
                .env_vars
                .iter()
                .find_map(|var| std::env::var(var).ok().filter(|k| !k.is_empty()));
            if let Some(key) = key {
                tracing::info!(
                    "Auto-detected provider '{}' from env var '{}'",
                    preset.name,
                    preset
                        .env_vars
                        .iter()
                        .find(|v| std::env::var(v).is_ok())
                        .unwrap_or(&"?")
                );
                providers.insert(
                    preset.name.to_string(),
                    Arc::new(openai::OpenAiProvider::new(
                        key.clone(),
                        Some(preset.chat_url.to_string()),
                    )),
                );
                provider_keys.insert(preset.name.to_string(), key);
            }
        }

        // Ensure the configured default is actually available; fall back gracefully
        if !providers.contains_key(&default_provider)
            && let Some(first) = providers.keys().next()
        {
            default_provider = first.clone();
        }

        Self {
            providers,
            provider_keys,
            default_provider,
            ollama_base_url: config.ollama_base_url.clone(),
        }
    }

    /// Add or replace a provider at runtime (hot-reload via /connect).
    /// Add or replace a provider at runtime (hot-reload via /connect or DB load).
    /// `key` is stored in provider_keys for live model listing; pass empty if not applicable.
    ///
    /// Note: always prefer `add_provider_with_key` when the API key is known — the key
    /// must be in `provider_keys` for `list_dynamic_models` to fetch live model lists.
    pub fn add_provider(&mut self, name: String, provider: Arc<dyn LlmProvider>) {
        tracing::info!("Provider hot-loaded: {name}");
        self.providers.insert(name, provider);
        // NOTE: provider_keys is NOT updated here — caller must use add_provider_with_key
        // when the API key is known, or live model listing will fall back to catalogue.
    }

    /// Add a provider with its API key. Prefer this over add_provider whenever the key
    /// is available so that list_dynamic_models() can fetch live model lists.
    pub fn add_provider_with_key(
        &mut self,
        name: String,
        provider: Arc<dyn LlmProvider>,
        key: String,
    ) {
        tracing::info!("Provider hot-loaded with key: {name}");
        self.providers.insert(name.clone(), provider);
        if !key.is_empty() {
            self.provider_keys.insert(name, key);
        }
    }

    /// Re-scan current shell env vars and hot-register any newly available providers.
    ///
    /// Scan the server's environment and register/update any newly available providers.
    ///
    /// Called from `GET /v1/models` so the picker reflects current env state.
    /// Now handles two cases:
    ///   1. Provider not yet registered → create it from env key
    ///   2. Provider registered but `provider_keys` is missing its key → fill it in
    ///      (happens when provider was loaded from DB without the key being stored)
    pub fn hot_sync_env_providers(&mut self) {
        // Helper: check if provider is missing from provider_keys (key not stored)
        let needs_key = |p: &std::collections::HashMap<String, String>, name: &str| -> bool {
            p.get(name).map(|k| k.is_empty()).unwrap_or(true)
        };

        // -- Core providers
        let missing_anthropic = !self.providers.contains_key("anthropic")
            || needs_key(&self.provider_keys, "anthropic");
        if missing_anthropic {
            let key = std::env::var("ANTHROPIC_API_KEY")
                .or_else(|_| std::env::var("CLAUDE_API_KEY"))
                .ok()
                .filter(|k| !k.is_empty());
            if let Some(key) = key {
                tracing::info!("hot_sync: registering/updating anthropic from env");
                self.providers.insert(
                    "anthropic".into(),
                    Arc::new(anthropic::AnthropicProvider::new(key.clone())),
                );
                self.provider_keys.insert("anthropic".into(), key);
            }
        }

        let missing_openai =
            !self.providers.contains_key("openai") || needs_key(&self.provider_keys, "openai");
        if missing_openai {
            let key = std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|k| !k.is_empty());
            if let Some(key) = key {
                tracing::info!("hot_sync: registering/updating openai from env");
                self.providers.insert(
                    "openai".into(),
                    Arc::new(openai::OpenAiProvider::new(key.clone(), None)),
                );
                self.provider_keys.insert("openai".into(), key);
            }
        }

        let missing_gemini =
            !self.providers.contains_key("gemini") || needs_key(&self.provider_keys, "gemini");
        if missing_gemini {
            let key = std::env::var("GOOGLE_API_KEY")
                .or_else(|_| std::env::var("GEMINI_API_KEY"))
                .ok()
                .filter(|k| !k.is_empty());
            if let Some(key) = key {
                tracing::info!("hot_sync: registering/updating gemini/google from env");
                self.providers.insert(
                    "gemini".into(),
                    Arc::new(gemini::GeminiProvider::new(key.clone())),
                );
                self.providers.insert(
                    "google".into(),
                    Arc::new(gemini::GeminiProvider::new(key.clone())),
                );
                self.provider_keys.insert("gemini".into(), key.clone());
                self.provider_keys.insert("google".into(), key);
            }
        }

        // -- Preset providers (Groq, OpenRouter, Together, etc.)
        for preset in PRESET_PROVIDERS {
            let missing = !self.providers.contains_key(preset.name)
                || needs_key(&self.provider_keys, preset.name);
            if !missing {
                continue;
            }
            let key = preset
                .env_vars
                .iter()
                .find_map(|var| std::env::var(var).ok().filter(|k| !k.is_empty()));
            if let Some(key) = key {
                tracing::info!("hot_sync: registering/updating {} from env", preset.name);
                self.providers.insert(
                    preset.name.into(),
                    Arc::new(openai::OpenAiProvider::new(
                        key.clone(),
                        Some(preset.chat_url.to_string()),
                    )),
                );
                self.provider_keys.insert(preset.name.into(), key);
            }
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
                self.default_provider = self.providers.keys().next().cloned().unwrap_or_default();
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
        type ModelFut =
            std::pin::Pin<Box<dyn std::future::Future<Output = Vec<ModelEntry>> + Send>>;
        let mut tasks: Vec<ModelFut> = Vec::new();

        for name in self.providers.keys() {
            let key = self
                .provider_keys
                .get(name.as_str())
                .cloned()
                .unwrap_or_default();

            match name.as_str() {
                // -- Local Ollama
                "ollama" => {
                    let url = self.ollama_base_url.clone();
                    tasks.push(Box::pin(async move {
                        let ol = ollama::OllamaProvider::new(url);
                        ol.list_models()
                            .await
                            .into_iter()
                            .map(|m| {
                                let id = format!("ollama/{m}");
                                ModelEntry {
                                    provider: "ollama".into(),
                                    id: id.clone(),
                                    display_name: m,
                                    toolset: "default".into(),
                                    max_tokens: catalogue::max_tokens_for_model(&id),
                                    context_window: catalogue::context_window_for_model(&id),
                                    dynamic: true,
                                }
                            })
                            .collect()
                    }));
                }

                // -- Anthropic — live /v1/models, fallback to catalogue
                "anthropic" => {
                    tasks.push(Box::pin(async move {
                        let live = anthropic::fetch_anthropic_models(&key).await;
                        if live.is_empty() {
                            // Provider is configured but endpoint unreachable — use catalogue
                            CATALOGUE
                                .iter()
                                .filter(|(p, ..)| *p == "anthropic")
                                .map(catalogue::ModelEntry::from_catalogue)
                                .collect()
                        } else {
                            live.into_iter()
                                .map(|(id, display)| {
                                    let full_id = format!("anthropic/{id}");
                                    ModelEntry {
                                        provider: "anthropic".into(),
                                        id: full_id.clone(),
                                        display_name: display,
                                        toolset: "default".into(),
                                        max_tokens: catalogue::max_tokens_for_model(&full_id),
                                        context_window: catalogue::context_window_for_model(
                                            &full_id,
                                        ),
                                        dynamic: true,
                                    }
                                })
                                .collect()
                        }
                    }));
                }

                // ── OpenAI — live /v1/models (chat only), fallback to catalogue
                "openai" => {
                    tasks.push(Box::pin(async move {
                        let ids = openai::fetch_openai_chat_models(&key).await;
                        if ids.is_empty() {
                            CATALOGUE
                                .iter()
                                .filter(|(p, ..)| *p == "openai")
                                .map(catalogue::ModelEntry::from_catalogue)
                                .collect()
                        } else {
                            ids.into_iter()
                                .map(|id| {
                                    let full_id = format!("openai/{id}");
                                    ModelEntry {
                                        provider: "openai".into(),
                                        id: full_id.clone(),
                                        display_name: id.clone(),
                                        toolset: "codex".into(),
                                        max_tokens: catalogue::max_tokens_for_model(&full_id),
                                        context_window: catalogue::context_window_for_model(
                                            &full_id,
                                        ),
                                        dynamic: true,
                                    }
                                })
                                .collect()
                        }
                    }));
                }

                // -- Gemini — live models list, fallback to catalogue
                "gemini" | "google" => {
                    let n = name.clone();
                    tasks.push(Box::pin(async move {
                        let live = gemini::fetch_gemini_models(&key).await;
                        if live.is_empty() {
                            CATALOGUE
                                .iter()
                                .filter(|(p, ..)| *p == "gemini")
                                .map(catalogue::ModelEntry::from_catalogue)
                                .collect()
                        } else {
                            live.into_iter()
                                .map(|(id, display)| {
                                    let full_id = format!("{n}/{id}");
                                    ModelEntry {
                                        provider: n.clone(),
                                        id: full_id.clone(),
                                        display_name: display,
                                        toolset: "gemini".into(),
                                        max_tokens: catalogue::max_tokens_for_model(&full_id),
                                        context_window: catalogue::context_window_for_model(
                                            &full_id,
                                        ),
                                        dynamic: true,
                                    }
                                })
                                .collect()
                        }
                    }));
                }

                // -- Preset providers (Groq, OpenRouter, etc.)
                _ => {
                    if let Some(preset) = PRESET_PROVIDERS.iter().find(|p| p.name == name.as_str())
                        && let Some(models_url) = preset.models_url
                    {
                        let n = name.clone();
                        let url = models_url.to_string();
                        tasks.push(Box::pin(async move {
                            openai::fetch_model_ids(&url, &key)
                                .await
                                .into_iter()
                                .map(|id| {
                                    let full_id = format!("{n}/{id}");
                                    ModelEntry {
                                        provider: n.clone(),
                                        id: full_id.clone(),
                                        display_name: id,
                                        toolset: "default".into(),
                                        max_tokens: catalogue::max_tokens_for_model(&full_id),
                                        context_window: catalogue::context_window_for_model(
                                            &full_id,
                                        ),
                                        dynamic: true,
                                    }
                                })
                                .collect()
                        }));
                    }
                }
            }
        }

        let mut out: Vec<ModelEntry> = join_all(tasks).await.into_iter().flatten().collect();
        out.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.id.cmp(&b.id)));
        out
    }

    /// Build an `Arc<dyn LlmProvider>` from DB `ProviderRow` fields.
    pub fn provider_from_row(
        kind: &str,
        api_key: Option<String>,
        base_url: Option<String>,
        config: &AiConfig,
    ) -> Option<Arc<dyn LlmProvider>> {
        match kind {
            "anthropic" => {
                let key = api_key
                    .clone()
                    .or_else(|| config.anthropic_api_key.clone())?;
                Some(Arc::new(anthropic::AnthropicProvider::new(key)))
            }
            "openai" => {
                let key = api_key.clone().or_else(|| config.openai_api_key.clone())?;
                Some(Arc::new(openai::OpenAiProvider::new(key, base_url.clone())))
            }
            "gemini" => {
                let key = api_key.clone().or_else(|| config.google_api_key.clone())?;
                Some(Arc::new(gemini::GeminiProvider::new(key)))
            }
            "ollama" => {
                let base = base_url
                    .clone()
                    .unwrap_or_else(|| config.ollama_base_url.clone());
                Some(Arc::new(ollama::OllamaProvider::new(base)))
            }
            "openai-compatible" => {
                let key = api_key.clone().unwrap_or_default();
                let url = base_url.clone()?;
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
    ///
    /// Public so `RouterAdapter` in `cade-server.rs` can resolve a provider Arc while
    /// holding the lock for only this call, then drop the lock before making HTTP calls.
    pub fn resolve_provider(&self, model: &str) -> Result<(Arc<dyn LlmProvider>, String)> {
        // 1. Explicit prefix: `gemini/gemini-2.5-pro`
        if let Some(slash) = model.find('/') {
            let prefix = &model[..slash];
            let bare = model[slash + 1..].to_string();
            return self
                .providers
                .get(prefix)
                .map(|p| (Arc::clone(p), bare))
                .ok_or_else(|| {
                    Error::custom(format!(
                        "Provider '{prefix}' is not configured. Run /connect {prefix} to add it."
                    ))
                });
        }

        // 2. Infer provider from model name pattern
        if let Some(prefix) = infer_provider_prefix(model) {
            return self
                .providers
                .get(prefix)
                .map(|p| (Arc::clone(p), model.to_string()))
                .ok_or_else(|| {
                    Error::custom(format!(
                        "Model '{model}' requires the '{prefix}' provider. Run /connect {prefix} to add it."
                    ))
                });
        }

        // 3. Truly unknown model — use the default provider
        self.providers
            .get(&self.default_provider)
            .map(|p| (Arc::clone(p), model.to_string()))
            .ok_or_else(|| Error::custom("No LLM provider available"))
    }

    /// Validate that the given model string can be routed.
    pub fn validate_model(&self, model: &str) -> Result<()> {
        self.resolve_provider(model).map(|_| ())
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
        let (provider, bare_model) = self.resolve_provider(&req.model)?;
        let routed = CompletionRequest {
            model: bare_model,
            ..req.clone()
        };
        provider.complete(&routed).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>> {
        let (provider, bare_model) = self.resolve_provider(&req.model)?;
        let routed = CompletionRequest {
            model: bare_model,
            ..req.clone()
        };
        provider.stream(&routed).await
    }
}

// -- Factory (kept for compatibility)

pub fn make_provider(config: &AiConfig) -> Result<Arc<dyn LlmProvider>> {
    Ok(Arc::new(LlmRouter::build(config)))
}

// region:    --- Tests

#[cfg(test)]
mod tests;

// endregion: --- Tests
