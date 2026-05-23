use crate::Result;
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use std::pin::Pin;
use tokio_stream::Stream;

use super::{
    CompletionRequest, CompletionResponse, LlmProvider, LlmToolCall, StreamChunk, TokenUsage,
    bare_model, provider_error, retry_with_backoff,
};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const MODELS_URL: &str = "https://api.anthropic.com/v1/models?limit=1000";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Fetch all models available to this API key from Anthropic's models endpoint.
/// Returns `(id, display_name)` pairs, newest first (as returned by the API).
/// Returns empty Vec on any error or timeout.
pub async fn fetch_anthropic_models(api_key: &str) -> Vec<(String, String)> {
    let client = Client::new();
    let req = client
        .get(MODELS_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .send();
    let resp = match tokio::time::timeout(std::time::Duration::from_secs(5), req).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) | Err(_) => return vec![],
    };
    if !resp.status().is_success() {
        return vec![];
    }
    let Ok(body) = resp.json::<Value>().await else {
        return vec![];
    };

    body["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m["id"].as_str()?;
                    let name = m["display_name"].as_str().unwrap_or(id);
                    Some((id.to_string(), name.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

// endregion: --- Tests

/// Returns true if the given Anthropic model expects the newer
/// `thinking.type=adaptive` + `output_config.effort` request shape.
///
/// As of 2025 Anthropic returns HTTP 400 `"thinking.type.enabled" is not
/// supported for this model` when sending the legacy `enabled` shape to
/// Claude 4+ family models. The canonical TypedDicts live in the official
/// Python SDK (`ThinkingConfigAdaptiveParam`, `OutputConfigParam`).
///
/// Heuristic: match `claude-(opus|sonnet|haiku)-<N>-…` where N ≥ 4. This
/// naturally covers current releases (`claude-sonnet-4-5-…`,
/// `claude-opus-4-…`) and future majors (claude-5-*, claude-10-*) without a
/// hardcoded model list.
pub(crate) fn supports_adaptive_thinking(model: &str) -> bool {
    let bare = bare_model(model);
    for family in ["sonnet", "opus", "haiku"] {
        let prefix = format!("claude-{family}-");
        if let Some(rest) = bare.strip_prefix(&prefix)
            && let Some(first) = rest.split('-').next()
            && let Ok(n) = first.parse::<u32>()
        {
            return n >= 4;
        }
    }
    false
}

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .tcp_keepalive(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| Client::new()),
            api_key,
        }
    }

    fn build_body(&self, req: &CompletionRequest, stream: bool) -> Value {
        // Separate system messages from the conversation
        let (system, messages): (Vec<_>, Vec<_>) =
            req.messages.iter().partition(|m| m.role == "system");
        let mut system_blocks: Vec<Value> = Vec::new();
        for m in system.iter() {
            if m.content.is_empty() {
                continue;
            }
            system_blocks.push(json!({
                "type": "text",
                "text": m.content,
            }));
        }

        // Apply cache_control to the first system block (static context)
        if let Some(first) = system_blocks.first_mut() {
            first["cache_control"] = json!({ "type": "ephemeral" });
        }

        // Anthropic rule: all tool_result blocks for a given assistant turn MUST be
        // in ONE user message. Consecutive "tool" messages must be merged.
        let mut anthropic_messages: Vec<Value> = Vec::new();
        let mut i = 0;
        while i < messages.len() {
            let m = &messages[i];
            match m.role.as_str() {
                "tool" => {
                    // Collect ALL consecutive tool messages into one user message
                    let mut tool_results: Vec<Value> = Vec::new();
                    while i < messages.len() && messages[i].role == "tool" {
                        let tm = &messages[i];
                        tool_results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": tm.tool_call_id.as_deref().unwrap_or(""),
                            "content": tm.content
                        }));
                        i += 1;
                    }
                    anthropic_messages.push(json!({ "role": "user", "content": tool_results }));
                }
                "assistant" if m.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) => {
                    let mut blocks =
                        Vec::with_capacity(1 + m.tool_calls.as_deref().unwrap_or_default().len());
                    if !m.content.is_empty() {
                        blocks.push(json!({"type": "text", "text": m.content}));
                    }
                    if let Some(calls) = &m.tool_calls {
                        blocks.extend(calls.iter().map(|tc| {
                            json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": tc.arguments
                            })
                        }));
                    }
                    anthropic_messages.push(json!({"role": "assistant", "content": blocks}));
                    i += 1;
                }
                _ => {
                    // When images are attached, build a multi-part content array.
                    // Anthropic format: [{"type":"image","source":{…}}, {"type":"text","text":"…"}]
                    if let Some(images) = &m.images
                        && !images.is_empty()
                    {
                        let mut blocks: Vec<Value> = images
                            .iter()
                            .map(|img| {
                                json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": img.media_type,
                                        "data": img.data
                                    }
                                })
                            })
                            .collect();
                        if !m.content.is_empty() {
                            blocks.push(json!({"type": "text", "text": m.content}));
                        }
                        anthropic_messages.push(json!({"role": m.role, "content": blocks}));
                        i += 1;
                        continue;
                    }
                    anthropic_messages.push(json!({"role": m.role, "content": m.content}));
                    i += 1;
                }
            }
        }

        // -- Prompt caching
        // Anthropic charges ~90% less for tokens served from the prompt cache.
        // We mark two stable, large anchors with cache_control so Anthropic
        // pins them in its KV cache across turns:
        //
        //   1. System prompt  — static for the entire session; always ≥1 024 tok.
        //   2. Last tool def  — tool schemas are fixed per session; marking the
        //                       last entry caches the entire tools array prefix.
        //
        // The cache TTL is 5 minutes (refreshed on every cache hit).  For a
        // typical coding session the system prompt + tool schemas are re-used
        // on every turn, so hit-rate is near 100% after the first request.
        //
        // Requirement: prompt-caching beta header must be sent (added in the
        // HTTP call sites below). claude-3-5+ supports it natively but the
        // header is harmless for all models.

        // Build tools array in Anthropic format, injecting cache_control on
        // the last entry so the full tools prefix is cached.
        let mut tools: Vec<Value> = req
            .tools
            .iter()
            .map(|schema| {
                let params = schema
                    .get("parameters")
                    .filter(|v| !v.is_null())
                    .or_else(|| schema.get("input_schema").filter(|v| !v.is_null()))
                    .cloned()
                    .unwrap_or(json!({"type": "object", "properties": {}, "required": []}));
                json!({
                    "name": schema["name"],
                    "description": schema["description"],
                    "input_schema": params
                })
            })
            .collect();

        // Mark the last tool with cache_control to cache the entire tools list.
        if let Some(last) = tools.last_mut() {
            last["cache_control"] = json!({"type": "ephemeral"});
        }

        // 3. Historical context caching
        // Place a cache_control breakpoint on the second-to-last user message
        // (the last historical user turn, skipping the current request).
        let mut user_count = 0;
        for msg in anthropic_messages.iter_mut().rev() {
            if msg["role"] == "user" {
                user_count += 1;
                if user_count == 2 {
                    // Convert content to array if it isn't one.
                    if let Some(content_str) = msg["content"].as_str() {
                        msg["content"] = json!([{
                            "type": "text",
                            "text": content_str,
                            "cache_control": { "type": "ephemeral" }
                        }]);
                    } else if let Some(arr) = msg["content"].as_array_mut()
                        && let Some(last_block) = arr.last_mut()
                    {
                        last_block["cache_control"] = json!({ "type": "ephemeral" });
                    }
                    break;
                }
            }
        }

        let mut body = json!({
            "model": bare_model(&req.model),
            "max_tokens": req.max_tokens.max(4096), // At least 4096, but allows higher if specified
            "messages": anthropic_messages,
            "stream": stream
        });

        if let Some(effort) = &req.reasoning_effort {
            if supports_adaptive_thinking(&req.model) {
                // Claude 4+ models require `thinking.type=adaptive` and the
                // effort level is passed via the top-level `output_config`.
                // Budget is managed dynamically by the server, so we do NOT
                // pre-allocate budget_tokens or inflate max_tokens.
                let mapped_effort = match effort.as_str() {
                    "low" | "medium" | "high" | "xhigh" | "max" => effort.clone(),
                    _ => "medium".to_string(),
                };
                body["thinking"] = json!({ "type": "adaptive" });
                body["output_config"] = json!({ "effort": mapped_effort });
            } else if bare_model(&req.model).contains("claude-3-7-sonnet") {
                // Legacy Claude 3.7 extended-thinking models:
                // `thinking.type=enabled` with an explicit `budget_tokens`.
                // Anthropic requires budget_tokens ≤ max_tokens. The max_tokens
                // field is shared between reasoning and output, so we scale the
                // reasoning budget relative to max_tokens.
                let effective_max = req.max_tokens.max(4096);
                let budget = match effort.as_str() {
                    "low" => (effective_max / 4).max(1024), // 25% of max_tokens
                    "medium" => (effective_max / 2).max(2048), // 50%
                    "high" => (effective_max * 3 / 4).max(4096), // 75%
                    "xhigh" => effective_max.saturating_sub(1024), // nearly all
                    _ => 0,
                };
                if budget > 0 {
                    // Ensure max_tokens is at least budget + 1024 so the model
                    // still has room for visible output after reasoning.
                    let adjusted_max = effective_max.max(budget + 1024);
                    body["max_tokens"] = json!(adjusted_max);
                    body["thinking"] = json!({
                        "type": "enabled",
                        "budget_tokens": budget
                    });
                }
            }
        }

        // System prompt: use structured block form so we can attach cache_control.
        if !system_blocks.is_empty() {
            body["system"] = json!(system_blocks);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        body
    }

    fn parse_response(body: &Value) -> CompletionResponse {
        let finish_reason = body["stop_reason"]
            .as_str()
            .unwrap_or("end_turn")
            .to_string();
        let mut content = None;
        let mut tool_calls = Vec::new();

        if let Some(arr) = body["content"].as_array() {
            for block in arr {
                match block["type"].as_str().unwrap_or("") {
                    "text" => {
                        content = block["text"].as_str().map(|s| s.to_string());
                    }
                    "tool_use" => {
                        tool_calls.push(LlmToolCall {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            name: block["name"].as_str().unwrap_or("").to_string(),
                            arguments: block["input"].clone(),
                            thought_signature: None,
                        });
                    }
                    _ => {}
                }
            }
        }

        CompletionResponse {
            content,
            tool_calls,
            finish_reason,
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let body = self.build_body(req, false);
        retry_with_backoff(
            "Anthropic::complete",
            3,
            std::time::Duration::from_secs(1),
            |_| {
                let client = self.client.clone();
                let api_key = self.api_key.clone();
                let body = body.clone();
                async move {
                    let resp = client
                        .post(API_URL)
                        .header("x-api-key", &api_key)
                        .header("anthropic-version", ANTHROPIC_VERSION)
                        .header("anthropic-beta", "prompt-caching-2024-07-31")
                        .header("content-type", "application/json")
                        .json(&body)
                        .send()
                        .await?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        return Err(provider_error("Anthropic", status, &text));
                    }
                    let json: serde_json::Value = resp.json().await?;
                    Ok(Self::parse_response(&json))
                }
            },
        )
        .await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let body = self.build_body(req, true);
        let req_model = req.model.clone(); // extracted before async_stream to avoid lifetime capture
        // Retry the HTTP handshake only; the byte stream itself is not retried
        // (partial streams can't be safely resumed without re-sending the request).
        let resp = retry_with_backoff(
            "Anthropic::stream",
            3,
            std::time::Duration::from_secs(1),
            |_| {
                let client = self.client.clone();
                let api_key = self.api_key.clone();
                let body = body.clone();
                async move {
                    let resp = client
                        .post(API_URL)
                        .header("x-api-key", &api_key)
                        .header("anthropic-version", ANTHROPIC_VERSION)
                        .header("anthropic-beta", "prompt-caching-2024-07-31")
                        .header("content-type", "application/json")
                        .json(&body)
                        .send()
                        .await?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        return Err(provider_error("Anthropic", status, &text));
                    }
                    Ok(resp)
                }
            },
        )
        .await?;

        let mut byte_stream = resp.bytes_stream();

        let s = stream! {
            let mut buf = Vec::new();
            // Accumulate partial tool call state
            let mut tool_id = String::new();
            let mut tool_name = String::new();
            let mut tool_args = String::new();
            let mut thinking_text = String::new();
            let mut in_thinking = false;
            // Accumulate token usage across message_start + message_delta
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut cache_read_tokens: u32 = 0;
            let mut cache_write_tokens: u32 = 0;

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => { yield Err(crate::Error::custom(format!("stream error: {e}"))); break; }
                };
                buf.extend_from_slice(&chunk);

                // Process complete SSE lines
                let mut start = 0;
                while let Some(pos) = buf[start..].iter().position(|&b| b == b'\n') {
                    let end = start + pos;
                    if let Ok(line_str) = std::str::from_utf8(&buf[start..end]) {
                        let line = line_str.trim();
                        if !line.is_empty() && !line.starts_with(':')
                            && let Some(data) = line.strip_prefix("data: ") {
                                let event: Value = match serde_json::from_str(data) {
                                    Ok(v) => v,
                                    Err(_) => { start = end + 1; continue; }
                                };

                    match event["type"].as_str().unwrap_or("") {
                        "content_block_delta" => {
                            match event["delta"]["type"].as_str().unwrap_or("") {
                                "text_delta" => {
                                    if let Some(text) = event["delta"]["text"].as_str() {
                                        yield Ok(StreamChunk::Text(text.to_string()));
                                    }
                                }
                                "input_json_delta" => {
                                    if let Some(partial) = event["delta"]["partial_json"].as_str() {
                                        tool_args.push_str(partial);
                                    }
                                }
                                "thinking_delta" => {
                                    if let Some(t) = event["delta"]["thinking"].as_str() {
                                        thinking_text.push_str(t);
                                    }
                                }
                                _ => {}
                            }
                        }
                        "content_block_start" => {
                            match event["content_block"]["type"].as_str().unwrap_or("") {
                                "tool_use" => {
                                    tool_id   = event["content_block"]["id"].as_str().unwrap_or("").to_string();
                                    tool_name = event["content_block"]["name"].as_str().unwrap_or("").to_string();
                                    tool_args.clear();
                                }
                                "thinking" => {
                                    in_thinking = true;
                                    thinking_text.clear();
                                }
                                _ => {}
                            }
                        }
                        "content_block_stop" => {
                            if in_thinking {
                                if !thinking_text.is_empty() {
                                    yield Ok(StreamChunk::Reasoning(std::mem::take(&mut thinking_text)));
                                }
                                in_thinking = false;
                            }
                            if !tool_name.is_empty() {
                                let args: Value = serde_json::from_str(&tool_args)
                                    .unwrap_or_else(|e| {
                                        tracing::warn!("Tool '{}' argument JSON parse failed: {e}; raw: {:?}", tool_name, tool_args);
                                        Value::Object(serde_json::Map::new())
                                    });
                                yield Ok(StreamChunk::ToolCall(LlmToolCall {
                                    id:                std::mem::take(&mut tool_id),
                                    name:              std::mem::take(&mut tool_name),
                                    arguments:         args,
                                    thought_signature: None,
                                }));
                                tool_args.clear();
                            }
                        }
                        "message_start" => {
                            // e.g. {"type":"message_start","message":{"usage":{"input_tokens":N,"cache_read_input_tokens":N,"cache_creation_input_tokens":N}}}
                            if let Some(n) = event["message"]["usage"]["input_tokens"].as_u64() {
                                input_tokens += n as u32;
                            }
                            if let Some(n) = event["message"]["usage"]["cache_read_input_tokens"].as_u64() {
                                cache_read_tokens += n as u32;
                            }
                            if let Some(n) = event["message"]["usage"]["cache_creation_input_tokens"].as_u64() {
                                cache_write_tokens += n as u32;
                            }
                        }
                        "message_delta" => {
                            // e.g. {"type":"message_delta","usage":{"output_tokens":N}}
                            if let Some(n) = event["usage"]["output_tokens"].as_u64() {
                                output_tokens += n as u32;
                            }
                        }
                        "message_stop" => {
                            if input_tokens > 0 || output_tokens > 0 || cache_read_tokens > 0 || cache_write_tokens > 0 {
                                yield Ok(StreamChunk::Usage(TokenUsage {
                                    input_tokens,
                                    output_tokens,
                                    cache_read_tokens,
                                    cache_write_tokens,
                                    model: req_model.clone(),
                                }));
                            }
                            if let Some(reason) = event["stop_reason"].as_str() {
                                yield Ok(StreamChunk::FinishReason(reason.to_string()));
                            }
                            yield Ok(StreamChunk::Done);
                            break;
                        }
                        _ => {}
                    }
                            }
                    }
                    start = end + 1;
                }
                if start > 0 {
                    buf.drain(..start);
                }
            }
        };

        Ok(Box::pin(s))
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;

    #[test]
    fn parse_response_text_only() {
        let body = json!({
            "stop_reason": "end_turn",
            "content": [{
                "type": "text",
                "text": "Hello from Claude!"
            }]
        });
        let resp = AnthropicProvider::parse_response(&body);
        assert_eq!(resp.content.as_deref(), Some("Hello from Claude!"));
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason, "end_turn");
    }

    #[test]
    fn parse_response_with_tool_use() {
        let body = json!({
            "stop_reason": "tool_use",
            "content": [
                {"type": "text", "text": "Let me check."},
                {
                    "type": "tool_use",
                    "id": "toolu_123",
                    "name": "bash",
                    "input": {"command": "ls -la"}
                }
            ]
        });
        let resp = AnthropicProvider::parse_response(&body);
        assert_eq!(resp.content.as_deref(), Some("Let me check."));
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "toolu_123");
        assert_eq!(resp.tool_calls[0].name, "bash");
        assert_eq!(resp.tool_calls[0].arguments["command"], "ls -la");
        assert_eq!(resp.finish_reason, "tool_use");
    }

    #[test]
    fn parse_response_multiple_tool_calls() {
        let body = json!({
            "stop_reason": "tool_use",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "bash",
                    "input": {"command": "pwd"}
                },
                {
                    "type": "tool_use",
                    "id": "toolu_2",
                    "name": "read_file",
                    "input": {"path": "Cargo.toml"}
                }
            ]
        });
        let resp = AnthropicProvider::parse_response(&body);
        assert_eq!(resp.tool_calls.len(), 2);
        assert_eq!(resp.tool_calls[0].name, "bash");
        assert_eq!(resp.tool_calls[1].name, "read_file");
    }

    #[test]
    fn parse_response_empty_content() {
        let body = json!({
            "stop_reason": "end_turn",
            "content": []
        });
        let resp = AnthropicProvider::parse_response(&body);
        assert!(resp.content.is_none());
        assert!(resp.tool_calls.is_empty());
    }

    #[test]
    fn build_body_includes_model_and_system() -> Result<()> {
        // -- Setup & Fixtures
        let provider = AnthropicProvider::new("sk-test".into());
        let req = CompletionRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![
                super::super::LlmMessage {
                    role: "system".into(),
                    content: "You are a helpful assistant.".into(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                },
                super::super::LlmMessage {
                    role: "user".into(),
                    content: "Hello".into(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                },
            ],
            tools: vec![],
            max_tokens: 8192,
            reasoning_effort: None,
        };
        let body = provider.build_body(&req, false);
        // -- Check
        assert_eq!(body["model"], "claude-sonnet-4-5-20250929");
        let stream = body["stream"].as_bool().ok_or("Should have stream bool")?;
        assert!(!stream);
        assert!(body["system"].is_array());
        assert_eq!(body["system"][0]["text"], "You are a helpful assistant.");
        let msgs = body["messages"]
            .as_array()
            .ok_or("Should have messages array")?;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");

        Ok(())
    }

    #[test]
    fn build_body_with_tools_adds_cache_control() -> Result<()> {
        // -- Setup & Fixtures
        let provider = AnthropicProvider::new("sk-test".into());
        let req = CompletionRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![super::super::LlmMessage {
                role: "user".into(),
                content: "Hello".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            }],
            tools: vec![json!({
                "name": "bash",
                "description": "Run command",
                "parameters": {"type": "object"}
            })],
            max_tokens: 8192,
            reasoning_effort: None,
        };
        let body = provider.build_body(&req, false);
        // -- Check
        let tools = body["tools"].as_array().ok_or("Should have tools array")?;
        assert_eq!(tools.len(), 1);
        assert!(tools[0]["cache_control"].is_object());

        Ok(())
    }

    #[test]
    fn build_body_with_reasoning_effort_legacy() {
        // Older Claude 3.7 still expects the `enabled` + budget_tokens shape.
        let provider = AnthropicProvider::new("sk-test".into());
        let req = CompletionRequest {
            model: "claude-3-7-sonnet-20250219".into(),
            messages: vec![super::super::LlmMessage {
                role: "user".into(),
                content: "Think hard".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            }],
            tools: vec![],
            max_tokens: 8192,
            reasoning_effort: Some("high".into()),
        };
        let body = provider.build_body(&req, false);
        assert!(body["thinking"].is_object());
        assert_eq!(body["thinking"]["type"], "enabled");
        // "high" = 75% of max_tokens (8192) = 6144, clamped to .max(4096)
        assert_eq!(body["thinking"]["budget_tokens"], 6144);
        // No output_config in the legacy shape.
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn build_body_with_reasoning_effort_adaptive() {
        // Claude 4+ (e.g. sonnet-4-5) requires adaptive thinking + output_config.effort.
        let provider = AnthropicProvider::new("sk-test".into());
        let req = CompletionRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![super::super::LlmMessage {
                role: "user".into(),
                content: "Think hard".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            }],
            tools: vec![],
            max_tokens: 8192,
            reasoning_effort: Some("high".into()),
        };
        let body = provider.build_body(&req, false);
        assert_eq!(body["thinking"]["type"], "adaptive");
        // Adaptive mode must NOT send budget_tokens (server manages budget).
        assert!(body["thinking"].get("budget_tokens").is_none());
        assert_eq!(body["output_config"]["effort"], "high");
        // max_tokens must not be auto-inflated in adaptive mode.
        assert_eq!(body["max_tokens"], 8192);
    }

    #[test]
    fn supports_adaptive_thinking_matrix() {
        use super::supports_adaptive_thinking;
        // Claude 4+ family -> adaptive
        assert!(supports_adaptive_thinking("claude-sonnet-4-5-20250929"));
        assert!(supports_adaptive_thinking("claude-opus-4-20250514"));
        assert!(supports_adaptive_thinking("claude-haiku-4-20250815"));
        assert!(supports_adaptive_thinking("anthropic/claude-sonnet-4-6"));
        // Future majors must keep working.
        assert!(supports_adaptive_thinking("claude-sonnet-5-20260101"));
        assert!(supports_adaptive_thinking("claude-opus-10-20270101"));
        // Legacy Claude 3.x -> NOT adaptive
        assert!(!supports_adaptive_thinking("claude-3-7-sonnet-20250219"));
        assert!(!supports_adaptive_thinking("claude-3-5-haiku-20241022"));
        assert!(!supports_adaptive_thinking("claude-3-opus-20240229"));
        // Non-Claude models -> false
        assert!(!supports_adaptive_thinking("gpt-4o"));
        assert!(!supports_adaptive_thinking("gemini-2.5-pro"));
    }

    #[test]
    fn build_body_merges_consecutive_tool_results() -> Result<()> {
        // -- Setup & Fixtures
        let provider = AnthropicProvider::new("sk-test".into());
        let req = CompletionRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![
                super::super::LlmMessage {
                    role: "user".into(),
                    content: "Do two things".into(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                },
                super::super::LlmMessage {
                    role: "assistant".into(),
                    content: "".into(),
                    tool_call_id: None,
                    tool_calls: Some(vec![
                        super::super::LlmToolCall {
                            id: "t1".into(),
                            name: "bash".into(),
                            arguments: json!({}),
                            thought_signature: None,
                        },
                        super::super::LlmToolCall {
                            id: "t2".into(),
                            name: "bash".into(),
                            arguments: json!({}),
                            thought_signature: None,
                        },
                    ]),
                    images: None,
                },
                super::super::LlmMessage {
                    role: "tool".into(),
                    content: "result 1".into(),
                    tool_call_id: Some("t1".into()),
                    tool_calls: None,
                    images: None,
                },
                super::super::LlmMessage {
                    role: "tool".into(),
                    content: "result 2".into(),
                    tool_call_id: Some("t2".into()),
                    tool_calls: None,
                    images: None,
                },
            ],
            tools: vec![],
            max_tokens: 8192,
            reasoning_effort: None,
        };
        let body = provider.build_body(&req, false);
        // -- Check
        let msgs = body["messages"]
            .as_array()
            .ok_or("Should have messages array")?;
        assert_eq!(msgs.len(), 3);
        let tool_results = msgs[2]["content"]
            .as_array()
            .ok_or("Should have content array")?;
        assert_eq!(tool_results.len(), 2);
        assert_eq!(tool_results[0]["type"], "tool_result");
        assert_eq!(tool_results[1]["type"], "tool_result");

        Ok(())
    }
}
