pub mod anthropic;
pub mod gemini;
pub mod ollama;
pub mod openai;

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

/// Known OpenAI-compatible provider presets (name → base URL).
pub const OPENAI_COMPAT_PRESETS: &[(&str, &str)] = &[
    ("openrouter", "https://openrouter.ai/api/v1/chat/completions"),
    ("together",   "https://api.together.xyz/v1/chat/completions"),
    ("groq",       "https://api.groq.com/openai/v1/chat/completions"),
    ("fireworks",  "https://api.fireworks.ai/inference/v1/chat/completions"),
    ("deepinfra",  "https://api.deepinfra.com/v1/openai/chat/completions"),
];

pub struct LlmRouter {
    providers: std::collections::HashMap<String, Arc<dyn LlmProvider>>,
    default_provider: String,
}

impl LlmRouter {
    pub fn build(config: &ServerConfig) -> Self {
        let mut providers: std::collections::HashMap<String, Arc<dyn LlmProvider>> =
            std::collections::HashMap::new();
        let mut default_provider = config.llm_provider.to_string();

        // Register every provider for which an API key is available
        if let Some(key) = &config.anthropic_api_key {
            providers.insert(
                "anthropic".to_string(),
                Arc::new(anthropic::AnthropicProvider::new(key.clone())),
            );
        }
        if let Some(key) = &config.openai_api_key {
            providers.insert(
                "openai".to_string(),
                Arc::new(openai::OpenAiProvider::new(key.clone(), None)),
            );
        }
        if let Some(key) = &config.google_api_key {
            providers.insert(
                "gemini".to_string(),
                Arc::new(gemini::GeminiProvider::new(key.clone())),
            );
            providers.insert(
                "google".to_string(),
                Arc::new(gemini::GeminiProvider::new(
                    config.google_api_key.clone().unwrap(),
                )),
            );
        }
        // Ollama is always available as a fallback
        providers.insert(
            "ollama".to_string(),
            Arc::new(ollama::OllamaProvider::new(config.ollama_base_url.clone())),
        );

        // Ensure the configured default is actually available; fall back gracefully
        if !providers.contains_key(&default_provider) {
            if let Some(first) = providers.keys().next() {
                default_provider = first.clone();
            }
        }

        Self { providers, default_provider }
    }

    /// Add or replace a provider at runtime (hot-reload via /connect).
    pub fn add_provider(&mut self, name: String, provider: Arc<dyn LlmProvider>) {
        tracing::info!("Provider hot-loaded: {name}");
        self.providers.insert(name, provider);
    }

    /// Remove a provider at runtime (via /disconnect).
    /// Returns false if the name was not found.
    pub fn remove_provider(&mut self, name: &str) -> bool {
        if self.providers.remove(name).is_some() {
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
