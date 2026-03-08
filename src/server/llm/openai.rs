use anyhow::Result;
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::pin::Pin;
use tokio_stream::Stream;

use super::{bare_model, provider_error, retry_with_backoff, CompletionRequest, CompletionResponse, LlmProvider, LlmToolCall, StreamChunk, TokenUsage};

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";

/// Fetch model IDs from an OpenAI-compatible `/v1/models` endpoint.
///
/// Handles two response shapes:
///   `{ "data": [ { "id": "..." }, … ] }` — OpenAI / Groq / OpenRouter
///   `[ { "id": "..." }, … ]`             — some providers return a bare array
///
/// Returns a sorted `Vec<String>` of model IDs; empty on any error.
/// Fetch only chat-completion-capable models from the OpenAI API.
/// Filters out embeddings, TTS, Whisper, DALL-E, and legacy completions models.
/// Returns model IDs sorted newest-first (by `created` timestamp).
pub async fn fetch_openai_chat_models(api_key: &str) -> Vec<String> {
    let client = Client::new();
    let req = client
        .get("https://api.openai.com/v1/models")
        .header("Authorization", format!("Bearer {api_key}"))
        .send();
    let resp = match tokio::time::timeout(std::time::Duration::from_secs(5), req).await {
        Ok(Ok(r))  => r,
        Ok(Err(_)) | Err(_) => return vec![],
    };
    if !resp.status().is_success() { return vec![]; }
    let Ok(body) = resp.json::<Value>().await else { return vec![]; };

    let arr = match body["data"].as_array() {
        Some(a) => a.clone(),
        None    => return vec![],
    };

    // Keep only models that support chat completions — filter by well-known prefixes
    let is_chat_model = |id: &str| -> bool {
        let id = id.to_lowercase();
        id.starts_with("gpt-")
            || id.starts_with("o1")
            || id.starts_with("o3")
            || id.starts_with("o4")
            || id.starts_with("chatgpt")
    };

    // Sort newest first using the `created` Unix timestamp
    let mut entries: Vec<(u64, String)> = arr.iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            if !is_chat_model(id) { return None; }
            let created = m["created"].as_u64().unwrap_or(0);
            Some((created, id.to_string()))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    entries.into_iter().map(|(_, id)| id).collect()
}

pub async fn fetch_model_ids(models_url: &str, api_key: &str) -> Vec<String> {
    let client = Client::new();
    let req = client
        .get(models_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send();
    let resp = match tokio::time::timeout(std::time::Duration::from_secs(5), req).await {
        Ok(Ok(r))  => r,
        Ok(Err(_)) | Err(_) => return vec![],
    };
    if !resp.status().is_success() { return vec![]; }
    let Ok(body) = resp.json::<Value>().await else { return vec![]; };

    // Try { "data": [...] } (OpenAI format)
    let items = if let Some(arr) = body["data"].as_array() {
        arr.clone()
    } else if body.is_array() {
        // Bare array
        body.as_array().cloned().unwrap_or_default()
    } else {
        return vec![];
    };

    let mut ids: Vec<String> = items.iter()
        .filter_map(|m| m["id"].as_str().map(String::from))
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort();
    ids
}

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    /// Override base URL for OpenAI-compatible endpoints (e.g. Together, Groq)
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.unwrap_or_else(|| OPENAI_URL.to_string()),
        }
    }

    fn to_openai_messages(req: &CompletionRequest) -> Value {
        let messages: Vec<Value> = req.messages.iter().map(|m| {
            match m.role.as_str() {
                "tool" => json!({
                    "role": "tool",
                    // OpenAI rejects null tool_call_id — fall back to empty string
                    // so the message is at least structurally valid.
                    "tool_call_id": m.tool_call_id.as_deref().unwrap_or(""),
                    "content": m.content
                }),
                "assistant" if m.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty()) => {
                    let tcs: Vec<Value> = m.tool_calls.as_ref().unwrap().iter().map(|tc| json!({
                        "id": tc.id,
                        "type": "function",
                        "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
                    })).collect();
                    // OpenAI requires `content` to be null (not "") when tool_calls present
                    let content = if m.content.is_empty() { Value::Null } else { Value::String(m.content.clone()) };
                    json!({"role": "assistant", "content": content, "tool_calls": tcs})
                }
                _ => json!({"role": m.role, "content": m.content}),
            }
        }).collect();
        json!(messages)
    }

    fn parse_response(body: &Value) -> CompletionResponse {
        let choice = &body["choices"][0];
        let finish_reason = choice["finish_reason"].as_str().unwrap_or("stop").to_string();
        let msg = &choice["message"];
        let content = msg["content"].as_str().map(|s| s.to_string());
        let tool_calls: Vec<LlmToolCall> = msg["tool_calls"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|tc| LlmToolCall {
                id:                tc["id"].as_str().unwrap_or("").to_string(),
                name:              tc["function"]["name"].as_str().unwrap_or("").to_string(),
                arguments:         serde_json::from_str(
                    tc["function"]["arguments"].as_str().unwrap_or("{}")
                ).unwrap_or_default(),
                thought_signature: None,
            })
            .collect();
        CompletionResponse { content, tool_calls, finish_reason }
    }

    fn build_tools(req: &CompletionRequest) -> Value {
        let tools: Vec<Value> = req.tools.iter().map(|s| json!({
            "type": "function",
            "function": {
                "name": s["name"],
                "description": s["description"],
                "parameters": s["parameters"]
            }
        })).collect();
        json!(tools)
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let mut body = json!({
            "model": bare_model(&req.model),
            "messages": Self::to_openai_messages(req),
            "max_tokens": req.max_tokens
        });
        if !req.tools.is_empty() {
            body["tools"] = Self::build_tools(req);
        }

        retry_with_backoff("OpenAI::complete", 3, std::time::Duration::from_secs(1), |_| {
            let client   = self.client.clone();
            let base_url = self.base_url.clone();
            let api_key  = self.api_key.clone();
            let body     = body.clone();
            async move {
                let resp = client
                    .post(&base_url)
                    .bearer_auth(&api_key)
                    .json(&body)
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    return Err(provider_error("OpenAI", status, &text));
                }
                Ok(Self::parse_response(&resp.json::<Value>().await?))
            }
        }).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let req_model = req.model.clone();   // extracted before async_stream to avoid lifetime capture
        let mut body = json!({
            "model": bare_model(&req_model),
            "messages": Self::to_openai_messages(req),
            "max_tokens": req.max_tokens,
            "stream": true,
            "stream_options": { "include_usage": true }
        });
        if !req.tools.is_empty() {
            body["tools"] = Self::build_tools(req);
        }

        let resp = retry_with_backoff("OpenAI::stream", 3, std::time::Duration::from_secs(1), |_| {
            let client   = self.client.clone();
            let base_url = self.base_url.clone();
            let api_key  = self.api_key.clone();
            let body     = body.clone();
            async move {
                let resp = client
                    .post(&base_url)
                    .bearer_auth(&api_key)
                    .json(&body)
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    return Err(provider_error("OpenAI", status, &text));
                }
                Ok(resp)
            }
        }).await?;

        let mut byte_stream = resp.bytes_stream();
        let s = stream! {
            let mut buf = String::new();
            // OpenAI streams tool calls with an `index` field to distinguish
            // parallel calls.  Use a BTreeMap keyed by index so multiple
            // tool calls in one turn are accumulated and emitted separately.
            let mut tool_map: std::collections::BTreeMap<usize, (String, String, String)> =
                std::collections::BTreeMap::new();

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk { Ok(c) => c, Err(e) => { yield Err(anyhow::anyhow!("{e}")); break; } };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf = buf[pos + 1..].to_string();
                    let data = match line.strip_prefix("data: ") { Some(d) => d, None => continue };
                    if data == "[DONE]" { yield Ok(StreamChunk::Done); return; }
                    let v: Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => continue };
                    let delta = &v["choices"][0]["delta"];

                    if let Some(text) = delta["content"].as_str() {
                        if !text.is_empty() { yield Ok(StreamChunk::Text(text.to_string())); }
                    }
                    if let Some(tcs) = delta["tool_calls"].as_array() {
                        for tc in tcs {
                            // `index` distinguishes parallel tool calls in one stream
                            let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                            let entry = tool_map.entry(idx).or_insert_with(|| (String::new(), String::new(), String::new()));
                            if let Some(id) = tc["id"].as_str() { entry.0 = id.to_string(); }
                            if let Some(n) = tc["function"]["name"].as_str() { entry.1 = n.to_string(); }
                            if let Some(a) = tc["function"]["arguments"].as_str() { entry.2.push_str(a); }
                        }
                    }
                    if let Some("stop" | "tool_calls") = v["choices"][0]["finish_reason"].as_str() {
                        // Emit every accumulated tool call in index order
                        let calls: Vec<(String, String, String)> =
                            tool_map.iter().map(|(_, v)| v.clone()).collect();
                        tool_map.clear();
                        for (id, name, args_str) in calls {
                            if !name.is_empty() {
                                let args = serde_json::from_str(&args_str).unwrap_or_else(|e| {
                                    tracing::warn!("Tool '{}' argument JSON parse failed: {e}; raw: {args_str:?}", name);
                                    serde_json::Value::Object(Default::default())
                                });
                                yield Ok(StreamChunk::ToolCall(LlmToolCall { id, name, arguments: args, thought_signature: None }));
                            }
                        }
                        // Don't return here — OpenAI sends usage in a separate chunk
                        // before [DONE] when stream_options.include_usage=true.
                    }
                    // Usage chunk: may arrive in any chunk, including the separate
                    // empty-choices chunk OpenAI sends after finish_reason.
                    if let Some(usage) = v.get("usage").filter(|u| !u.is_null()) {
                        let in_tok   = usage["prompt_tokens"].as_u64().unwrap_or(0) as u32;
                        let out_tok  = usage["completion_tokens"].as_u64().unwrap_or(0) as u32;
                        let cache_tok = usage["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0) as u32;
                        if in_tok > 0 || out_tok > 0 || cache_tok > 0 {
                            yield Ok(StreamChunk::Usage(TokenUsage {
                                input_tokens:       in_tok,
                                output_tokens:      out_tok,
                                cache_read_tokens:  cache_tok,
                                cache_write_tokens: 0,
                                model:              req_model.clone(),
                            }));
                        }
                    }
                }
            }
            // Byte stream exhausted without explicit [DONE] — always send Done
            // so the SSE client doesn't fall back to the blocking endpoint.
            // Also flush any tool calls that arrived without an explicit finish_reason
            // (some OpenAI-compatible providers omit it).
            let remaining: Vec<(String, String, String)> =
                tool_map.iter().map(|(_, v)| v.clone()).collect();
            for (id, name, args_str) in remaining {
                if !name.is_empty() {
                    let args = serde_json::from_str(&args_str).unwrap_or_default();
                    yield Ok(StreamChunk::ToolCall(LlmToolCall { id, name, arguments: args, thought_signature: None }));
                }
            }
            yield Ok(StreamChunk::Done);
        };
        Ok(Box::pin(s))
    }
}
