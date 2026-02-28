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

use crate::server::config::{LlmProviderKind, ServerConfig};

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
            // Accept both "gemini" and "google" prefixes
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

    /// Select provider and bare model name for a `provider/model` or bare `model` string.
    ///
    /// Resolution order:
    ///   1. Explicit `provider/model` prefix (e.g. `gemini/gemini-2.5-pro`)
    ///   2. Auto-detect provider from well-known model name patterns
    ///   3. Fall back to the configured default provider
    fn pick(&self, model: &str) -> Option<(Arc<dyn LlmProvider>, String)> {
        // 1. Explicit prefix
        if let Some(slash) = model.find('/') {
            let prefix = &model[..slash];
            let bare   = model[slash + 1..].to_string();
            if let Some(p) = self.providers.get(prefix) {
                return Some((Arc::clone(p), bare));
            }
        }

        // 2. Infer provider from model name pattern
        if let Some(prefix) = infer_provider_prefix(model) {
            if let Some(p) = self.providers.get(prefix) {
                return Some((Arc::clone(p), model.to_string()));
            }
        }

        // 3. Default provider fallback
        self.providers
            .get(&self.default_provider)
            .map(|p| (Arc::clone(p), model.to_string()))
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
        let (provider, bare_model) = self
            .pick(&req.model)
            .ok_or_else(|| anyhow::anyhow!("No provider available for model '{}'", req.model))?;
        let routed = CompletionRequest { model: bare_model, ..req.clone() };
        provider.complete(&routed).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>> {
        let (provider, bare_model) = self
            .pick(&req.model)
            .ok_or_else(|| anyhow::anyhow!("No provider available for model '{}'", req.model))?;
        let routed = CompletionRequest { model: bare_model, ..req.clone() };
        provider.stream(&routed).await
    }
}

// ── Factory (kept for compatibility) ──────────────────────────────────────────

pub fn make_provider(config: &ServerConfig) -> Result<Arc<dyn LlmProvider>> {
    Ok(Arc::new(LlmRouter::build(config)))
}
