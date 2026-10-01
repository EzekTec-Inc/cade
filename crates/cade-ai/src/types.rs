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
    pub deepseek_api_key: Option<String>,
    pub ollama_base_url: String,
    pub llm_provider: String,
}

impl AiConfig {
    /// Credential slots are legacy adapter fields; provider identities/endpoints
    /// and their environment variable bindings come from the editable registry.
    pub fn from_env() -> Self {
        let registry = crate::provider_registry::ProviderRegistry::configured();
        let key = |slot: &str| {
            registry
                .get_all_providers()
                .iter()
                .find(|provider| provider.config_key.as_deref() == Some(slot))
                .and_then(|provider| provider.env_key())
        };
        let requested = std::env::var("CADE_LLM_PROVIDER")
            .ok()
            .filter(|provider| !provider.trim().is_empty());
        let provider = requested
            .as_deref()
            .and_then(|name| registry.get(name))
            .or_else(|| registry.detected_default());
        Self {
            anthropic_api_key: key("anthropic"),
            openai_api_key: key("openai"),
            google_api_key: key("google"),
            deepseek_api_key: key("deepseek"),
            ollama_base_url: registry
                .get_all_providers()
                .iter()
                .find(|provider| provider.config_key.as_deref() == Some("ollama"))
                .map(|provider| provider.endpoint())
                .unwrap_or_default(),
            llm_provider: requested
                .map(|name| {
                    registry
                        .get(&name)
                        .map(|provider| provider.name.clone())
                        .unwrap_or(name)
                })
                .or_else(|| provider.map(|provider| provider.name.clone()))
                .unwrap_or_default(),
        }
    }
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
    /// Provider-agnostic prompt caching metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    /// Opaque provider continuation carried with a tool call through persistence.
    /// Gemini uses its native thought signature. OpenAI Responses uses a tagged,
    /// versioned envelope of reasoning items. Adapters interpret only their own
    /// format; this is replay metadata, never assistant text or tool arguments.
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

pub(crate) fn validate_model_id(model: &str) -> Result<()> {
    if model.trim().is_empty()
        || model != model.trim()
        || model.ends_with('/')
        || model.starts_with('/')
        || model.chars().any(char::is_control)
    {
        return Err(crate::Error::custom(
            "Model ID must be a nonempty, valid model name",
        ));
    }
    Ok(())
}

pub(crate) fn structured_fallback_request(
    req: &CompletionRequest,
    schema: &Value,
) -> CompletionRequest {
    let mut request = req.clone();
    request.messages.insert(
        0,
        LlmMessage {
            role: "system".into(),
            content: format!("Return only JSON matching this JSON Schema: {schema}"),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        },
    );
    request
}

pub(crate) fn parse_structured_text(text: &str) -> Result<Value> {
    serde_json::from_str(&crate::utils::clean_json_markers(text)).map_err(|e| {
        crate::Error::custom(format!(
            "Structured output parsing failed: {e}. Raw response: {text}"
        ))
    })
}

pub(crate) async fn structured_fallback<P: LlmProvider + ?Sized>(
    provider: &P,
    req: &CompletionRequest,
    schema: &Value,
) -> Result<Value> {
    let response = provider
        .complete(&structured_fallback_request(req, schema))
        .await?;
    parse_structured_text(response.content.as_deref().unwrap_or_default())
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Optional configured/registered routing identity for factory defaults.
    /// Providers without metadata keep returning None and accept explicit models.
    fn default_model(&self) -> Option<String> {
        None
    }

    /// Validate a requested model before acknowledging a live control action.
    fn validate_model(&self, model: &str) -> Result<()> {
        validate_model_id(model)
    }

    /// Discovery reports provider data without restricting execution to listed IDs.
    async fn discover_models(&self) -> Result<Vec<crate::ModelEntry>> {
        Ok(Vec::new())
    }

    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse>;
    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<std::pin::Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>>;

    /// Request structured output. The fallback instructs the model with the schema
    /// and parses JSON; full local schema validation is not implied. Providers can
    /// override this with registered native structured formats.
    async fn complete_structured(
        &self,
        req: &CompletionRequest,
        schema: serde_json::Value,
    ) -> Result<serde_json::Value> {
        structured_fallback(self, req, &schema).await
    }
}
