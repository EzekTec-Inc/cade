use anyhow::Result;
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::pin::Pin;
use tokio_stream::Stream;

use super::{bare_model, provider_error, retry_with_backoff, CompletionRequest, CompletionResponse, LlmProvider, LlmToolCall, StreamChunk, TokenUsage};

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
        Ok(Ok(r))  => r,
        Ok(Err(_)) | Err(_) => return vec![],
    };
    if !resp.status().is_success() { return vec![]; }
    let Ok(body) = resp.json::<Value>().await else { return vec![]; };

    body["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id   = m["id"].as_str()?;
                    let name = m["display_name"].as_str().unwrap_or(id);
                    Some((id.to_string(), name.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
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
            api_key 
        }
    }

    fn build_body(&self, req: &CompletionRequest, stream: bool) -> Value {
        // Separate system messages from the conversation
        let (system, messages): (Vec<_>, Vec<_>) = req.messages.iter().partition(|m| m.role == "system");
        let system_text: String = system.iter().map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n\n");

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
                "assistant" if m.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty()) => {
                    let tool_uses: Vec<Value> = m.tool_calls.as_ref().unwrap().iter().map(|tc| json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments
                    })).collect();
                    let mut blocks = tool_uses;
                    if !m.content.is_empty() {
                        blocks.insert(0, json!({"type": "text", "text": m.content}));
                    }
                    anthropic_messages.push(json!({"role": "assistant", "content": blocks}));
                    i += 1;
                }
                _ => {
                    // When images are attached, build a multi-part content array.
                    // Anthropic format: [{"type":"image","source":{…}}, {"type":"text","text":"…"}]
                    if let Some(images) = &m.images {
                        if !images.is_empty() {
                            let mut blocks: Vec<Value> = images.iter().map(|img| json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": img.media_type,
                                    "data": img.data
                                }
                            })).collect();
                            if !m.content.is_empty() {
                                blocks.push(json!({"type": "text", "text": m.content}));
                            }
                            anthropic_messages.push(json!({"role": m.role, "content": blocks}));
                            i += 1;
                            continue;
                        }
                    }
                    anthropic_messages.push(json!({"role": m.role, "content": m.content}));
                    i += 1;
                }
            }
        }

        // ── Prompt caching ────────────────────────────────────────────────────
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
        let mut tools: Vec<Value> = req.tools.iter().map(|schema| json!({
            "name": schema["name"],
            "description": schema["description"],
            "input_schema": schema["parameters"]
        })).collect();

        // Mark the last tool with cache_control to cache the entire tools list.
        if let Some(last) = tools.last_mut() {
            last["cache_control"] = json!({"type": "ephemeral"});
        }

        let mut body = json!({
            "model": bare_model(&req.model),
            "max_tokens": req.max_tokens.max(4096), // At least 4096, but allows higher if specified
            "messages": anthropic_messages,
            "stream": stream
        });

        if let Some(effort) = &req.reasoning_effort {
            let budget = match effort.as_str() {
                "low" => 1024,
                "medium" => 4096,
                "high" => 16384,
                "xhigh" => 32768,
                _ => 0,
            };
            if budget > 0 {
                body["thinking"] = json!({
                    "type": "enabled",
                    "budget_tokens": budget
                });
            }
        }

        // System prompt: use structured block form so we can attach cache_control.
        if !system_text.is_empty() {
            body["system"] = json!([{
                "type": "text",
                "text": system_text,
                "cache_control": { "type": "ephemeral" }
            }]);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        body
    }

    fn parse_response(body: &Value) -> CompletionResponse {
        let finish_reason = body["stop_reason"].as_str().unwrap_or("end_turn").to_string();
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
                            id:                block["id"].as_str().unwrap_or("").to_string(),
                            name:              block["name"].as_str().unwrap_or("").to_string(),
                            arguments:         block["input"].clone(),
                            thought_signature: None,
                        });
                    }
                    _ => {}
                }
            }
        }

        CompletionResponse { content, tool_calls, finish_reason }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let body = self.build_body(req, false);
        retry_with_backoff("Anthropic::complete", 3, std::time::Duration::from_secs(1), |_| {
            let client  = self.client.clone();
            let api_key = self.api_key.clone();
            let body    = body.clone();
            async move {
                let resp = client
                    .post(API_URL)
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", "2023-06-01")
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
        }).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let body       = self.build_body(req, true);
        let req_model  = req.model.clone();   // extracted before async_stream to avoid lifetime capture
        // Retry the HTTP handshake only; the byte stream itself is not retried
        // (partial streams can't be safely resumed without re-sending the request).
        let resp = retry_with_backoff("Anthropic::stream", 3, std::time::Duration::from_secs(1), |_| {
            let client  = self.client.clone();
            let api_key = self.api_key.clone();
            let body    = body.clone();
            async move {
                let resp = client
                    .post(API_URL)
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", "2023-06-01")
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
        }).await?;

        let mut byte_stream = resp.bytes_stream();

        let s = stream! {
            let mut buf = String::new();
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
                    Err(e) => { yield Err(anyhow::anyhow!("stream error: {e}")); break; }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                // Process complete SSE lines
                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf = buf[pos + 1..].to_string();

                    if line.is_empty() || line.starts_with(':') { continue; }
                    let data = if let Some(d) = line.strip_prefix("data: ") { d } else { continue };

                    let event: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => continue,
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
                                    id:                tool_id.clone(),
                                    name:              tool_name.clone(),
                                    arguments:         args,
                                    thought_signature: None,
                                }));
                                tool_name.clear();
                                tool_id.clear();
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
                            yield Ok(StreamChunk::Done);
                            break;
                        }
                        _ => {}
                    }
                }
            }
        };

        Ok(Box::pin(s))
    }
}
