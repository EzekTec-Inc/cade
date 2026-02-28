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

// ── Factory ───────────────────────────────────────────────────────────────────

pub fn make_provider(config: &ServerConfig) -> Result<Arc<dyn LlmProvider>> {
    match config.llm_provider {
        LlmProviderKind::Anthropic => {
            let key = config.anthropic_api_key.clone()
                .ok_or_else(|| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
            Ok(Arc::new(anthropic::AnthropicProvider::new(key)))
        }
        LlmProviderKind::OpenAI => {
            let key = config.openai_api_key.clone()
                .ok_or_else(|| anyhow::anyhow!("OPENAI_API_KEY not set"))?;
            Ok(Arc::new(openai::OpenAiProvider::new(key, None)))
        }
        LlmProviderKind::Gemini => {
            let key = config.google_api_key.clone()
                .ok_or_else(|| anyhow::anyhow!("GOOGLE_API_KEY not set"))?;
            Ok(Arc::new(gemini::GeminiProvider::new(key)))
        }
        LlmProviderKind::Ollama => {
            Ok(Arc::new(ollama::OllamaProvider::new(config.ollama_base_url.clone())))
        }
    }
}
