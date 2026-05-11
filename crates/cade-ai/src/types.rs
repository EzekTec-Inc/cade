use crate::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_stream::Stream;

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
