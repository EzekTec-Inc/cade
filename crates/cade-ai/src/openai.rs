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

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";

/// Recursively fix JSON Schema fields that OpenAI rejects.
/// OpenAI requires every object-type node to have a `properties` field.
/// Missing `properties` on an object causes a 400 "object schema missing properties".
fn clean_openai_schema(v: &mut Value) {
    match v {
        Value::Object(map) => {
            if map.get("type").and_then(|t| t.as_str()) == Some("object")
                && !map.contains_key("properties")
            {
                map.insert("properties".to_string(), json!({}));
            }
            for val in map.values_mut() {
                clean_openai_schema(val);
            }
        }
        Value::Array(arr) => {
            for val in arr.iter_mut() {
                clean_openai_schema(val);
            }
        }
        _ => {}
    }
}

fn needs_max_completion_tokens(model: &str) -> bool {
    let bare = model.to_lowercase();
    bare.starts_with("gpt-5")
        || bare.starts_with("o1")
        || bare.starts_with("o3")
        || bare.starts_with("o4")
}

fn needs_responses_api(model: &str) -> bool {
    let bare = model.to_lowercase();
    bare.starts_with("gpt-5")
        || bare == "o1-pro"
        || bare.starts_with("o3-pro")
        || bare.starts_with("computer-use-preview")
}

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
        Ok(Ok(r)) => r,
        Ok(Err(_)) | Err(_) => return vec![],
    };
    if !resp.status().is_success() {
        return vec![];
    }
    let Ok(body) = resp.json::<Value>().await else {
        return vec![];
    };

    let arr = match body["data"].as_array() {
        Some(a) => a.clone(),
        None => return vec![],
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
    let mut entries: Vec<(u64, String)> = arr
        .iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            if !is_chat_model(id) {
                return None;
            }
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
        Ok(Ok(r)) => r,
        Ok(Err(_)) | Err(_) => return vec![],
    };
    if !resp.status().is_success() {
        return vec![];
    }
    let Ok(body) = resp.json::<Value>().await else {
        return vec![];
    };

    // Try { "data": [...] } (OpenAI format)
    let items = if let Some(arr) = body["data"].as_array() {
        arr.clone()
    } else if body.is_array() {
        // Bare array
        body.as_array().cloned().unwrap_or_default()
    } else {
        return vec![];
    };

    let mut ids: Vec<String> = items
        .iter()
        .filter_map(|m| m["id"].as_str().map(String::from))
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort();
    ids
}

// endregion: --- Tests

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    /// Override base URL for OpenAI-compatible endpoints (e.g. Together, Groq)
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: Client::builder()
                .tcp_keepalive(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| Client::new()),
            api_key,
            base_url: base_url.unwrap_or_else(|| OPENAI_URL.to_string()),
        }
    }

    fn to_openai_messages(req: &CompletionRequest) -> Value {
        let messages: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                match m.role.as_str() {
                    "tool" => json!({
                        "role": "tool",
                        // OpenAI rejects null tool_call_id — fall back to empty string
                        // so the message is at least structurally valid.
                        "tool_call_id": m.tool_call_id.as_deref().unwrap_or(""),
                        "content": m.content
                    }),
                    "assistant" if m.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) => {
                        let tcs: Vec<Value> = m.tool_calls.as_deref().unwrap_or_default().iter().map(|tc| json!({
                        "id": tc.id,
                        "type": "function",
                        "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
                    })).collect();
                        // OpenAI requires `content` to be null (not "") when tool_calls present
                        let content = if m.content.is_empty() {
                            Value::Null
                        } else {
                            Value::String(m.content.clone())
                        };
                        json!({"role": "assistant", "content": content, "tool_calls": tcs})
                    }
                    _ => {
                        // When images are attached, build a multi-part content array.
                        // OpenAI vision format: [{"type":"image_url","image_url":{"url":"data:…"}}, …]
                        if let Some(images) = &m.images
                            && !images.is_empty()
                        {
                            let mut parts: Vec<Value> = images.iter().map(|img| json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:{};base64,{}", img.media_type, img.data)
                                }
                            })).collect();
                            if !m.content.is_empty() {
                                parts.push(json!({"type": "text", "text": m.content}));
                            }
                            return json!({"role": m.role, "content": parts});
                        }
                        json!({"role": m.role, "content": m.content})
                    }
                }
            })
            .collect();
        json!(messages)
    }

    fn parse_response(body: &Value) -> CompletionResponse {
        let choice = &body["choices"][0];
        let finish_reason = choice["finish_reason"]
            .as_str()
            .unwrap_or("stop")
            .to_string();
        let msg = &choice["message"];
        let content = msg["content"].as_str().map(|s| s.to_string());
        let tool_calls: Vec<LlmToolCall> = msg["tool_calls"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|tc| LlmToolCall {
                id: tc["id"].as_str().unwrap_or("").to_string(),
                name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
                arguments: serde_json::from_str(
                    tc["function"]["arguments"].as_str().unwrap_or("{}"),
                )
                .unwrap_or_default(),
                thought_signature: None,
            })
            .collect();
        CompletionResponse {
            content,
            tool_calls,
            finish_reason,
        }
    }

    fn build_tools(req: &CompletionRequest) -> Value {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|s| {
                let mut params = s
                    .get("parameters")
                    .filter(|v| !v.is_null())
                    .or_else(|| s.get("input_schema").filter(|v| !v.is_null()))
                    .cloned()
                    .unwrap_or(json!({"type": "object", "properties": {}, "required": []}));
                clean_openai_schema(&mut params);
                json!({
                    "type": "function",
                    "function": {
                        "name": s["name"],
                        "description": s["description"],
                        "parameters": params
                    }
                })
            })
            .collect();
        json!(tools)
    }

    fn build_responses_tools(req: &CompletionRequest) -> Value {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|s| {
                let mut params = s
                    .get("parameters")
                    .filter(|v| !v.is_null())
                    .or_else(|| s.get("input_schema").filter(|v| !v.is_null()))
                    .cloned()
                    .unwrap_or(json!({"type": "object", "properties": {}, "required": []}));
                clean_openai_schema(&mut params);
                json!({
                    "type": "function",
                    "name": s["name"],
                    "description": s["description"],
                    "parameters": params
                })
            })
            .collect();
        json!(tools)
    }

    fn to_responses_input(req: &CompletionRequest) -> Value {
        let mut items: Vec<Value> = Vec::new();
        for m in &req.messages {
            match m.role.as_str() {
                "tool" => items.push(json!({
                    "type": "function_call_output",
                    "call_id": m.tool_call_id.as_deref().unwrap_or(""),
                    "output": m.content
                })),
                "assistant" if m.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) => {
                    if !m.content.is_empty() {
                        items.push(json!({"role": "assistant", "content": m.content}));
                    }
                    for tc in m.tool_calls.as_deref().unwrap_or_default() {
                        items.push(json!({
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": tc.name,
                            "arguments": tc.arguments.to_string()
                        }));
                    }
                }
                _ => items.push(json!({"role": m.role, "content": m.content})),
            }
        }
        json!(items)
    }

    fn parse_responses_response(body: &Value) -> CompletionResponse {
        let output = body["output"].as_array().cloned().unwrap_or_default();
        let mut content: Option<String> = None;
        let mut tool_calls: Vec<LlmToolCall> = Vec::new();
        let mut finish_reason = "stop".to_string();

        for item in &output {
            let item_type = item["type"].as_str().unwrap_or("");
            match item_type {
                "message" => {
                    let parts = item["content"].as_array().cloned().unwrap_or_default();
                    let mut text = String::new();
                    for part in &parts {
                        if part["type"].as_str() == Some("output_text")
                            && let Some(t) = part["text"].as_str()
                        {
                            text.push_str(t);
                        }
                    }
                    if !text.is_empty() {
                        content = Some(text);
                    }
                }
                "function_call" => {
                    finish_reason = "tool_calls".to_string();
                    let name = item["name"].as_str().unwrap_or("").to_string();
                    let id = item["call_id"].as_str().unwrap_or("").to_string();
                    let args_str = item["arguments"].as_str().unwrap_or("{}");
                    let arguments = serde_json::from_str(args_str).unwrap_or_default();
                    tool_calls.push(LlmToolCall {
                        id,
                        name,
                        arguments,
                        thought_signature: None,
                    });
                }
                _ => {}
            }
        }

        if body["status"].as_str() == Some("incomplete") {
            finish_reason = body["incomplete_details"]["reason"]
                .as_str()
                .unwrap_or("stop")
                .to_string();
        }

        CompletionResponse {
            content,
            tool_calls,
            finish_reason,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let bare_model_id = bare_model(&req.model);
        let use_responses = needs_responses_api(bare_model_id) && self.base_url == OPENAI_URL;

        if use_responses {
            let mut body = json!({
                "model": bare_model_id,
                "input": Self::to_responses_input(req),
                "max_output_tokens": req.max_tokens
            });
            if !req.tools.is_empty() {
                body["tools"] = Self::build_responses_tools(req);
            }
            if let Some(effort) = &req.reasoning_effort
                && ["low", "medium", "high"].contains(&effort.as_str())
            {
                body["reasoning_effort"] = effort.clone().into();
            }
            return retry_with_backoff(
                "OpenAI::complete",
                3,
                std::time::Duration::from_secs(1),
                |_| {
                    let client = self.client.clone();
                    let api_key = self.api_key.clone();
                    let body = body.clone();
                    async move {
                        let resp = client
                            .post(OPENAI_RESPONSES_URL)
                            .bearer_auth(&api_key)
                            .json(&body)
                            .send()
                            .await?;
                        if !resp.status().is_success() {
                            let status = resp.status();
                            let text = resp.text().await.unwrap_or_default();
                            return Err(provider_error("OpenAI", status, &text));
                        }
                        Ok(Self::parse_responses_response(&resp.json::<Value>().await?))
                    }
                },
            )
            .await;
        }

        let mut body = if needs_max_completion_tokens(bare_model_id) {
            json!({
                "model": bare_model_id,
                "messages": Self::to_openai_messages(req),
                "max_completion_tokens": req.max_tokens
            })
        } else {
            json!({
                "model": bare_model_id,
                "messages": Self::to_openai_messages(req),
                "max_tokens": req.max_tokens
            })
        };
        if !req.tools.is_empty() {
            body["tools"] = Self::build_tools(req);
        }
        if let Some(effort) = &req.reasoning_effort
            && ["low", "medium", "high"].contains(&effort.as_str())
        {
            body["reasoning_effort"] = effort.clone().into();
        }

        retry_with_backoff(
            "OpenAI::complete",
            3,
            std::time::Duration::from_secs(1),
            |_| {
                let client = self.client.clone();
                let base_url = self.base_url.clone();
                let api_key = self.api_key.clone();
                let body = body.clone();
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
            },
        )
        .await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let req_model = req.model.clone();
        let bare_model_id = bare_model(&req_model);
        let use_responses = needs_responses_api(bare_model_id) && self.base_url == OPENAI_URL;

        if use_responses {
            let mut body = json!({
                "model": bare_model_id,
                "input": Self::to_responses_input(req),
                "max_output_tokens": req.max_tokens,
                "stream": true
            });
            if !req.tools.is_empty() {
                body["tools"] = Self::build_responses_tools(req);
            }
            if let Some(effort) = &req.reasoning_effort
                && ["low", "medium", "high"].contains(&effort.as_str())
            {
                body["reasoning_effort"] = effort.clone().into();
            }

            let resp = retry_with_backoff(
                "OpenAI::stream",
                3,
                std::time::Duration::from_secs(1),
                |_| {
                    let client = self.client.clone();
                    let api_key = self.api_key.clone();
                    let body = body.clone();
                    async move {
                        let resp = client
                            .post(OPENAI_RESPONSES_URL)
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
                },
            )
            .await?;

            let mut byte_stream = resp.bytes_stream();
            let s = stream! {
                let mut buf = Vec::new();
                let mut tool_map: std::collections::BTreeMap<usize, (String, String, String)> =
                    std::collections::BTreeMap::new();

                while let Some(chunk) = byte_stream.next().await {
                    let chunk = match chunk { Ok(c) => c, Err(e) => { yield Err(crate::Error::custom(format!("{e}"))); break; } };
                    buf.extend_from_slice(&chunk);

                    let mut start = 0;
                    while let Some((pos, len)) = {
                        let mut earliest = None;
                        if let Some(p) = buf[start..].windows(2).position(|w| w == b"\n\n") {
                            earliest = Some((p, 2));
                        }
                        if let Some(p) = buf[start..].windows(4).position(|w| w == b"\r\n\r\n")
                            && earliest.is_none_or(|(ep, _)| p < ep) {
                                earliest = Some((p, 4));
                            }
                        earliest
                    } {
                        let end = start + pos;
                        let next_start = end + len;

                        if let Ok(block_str) = std::str::from_utf8(&buf[start..end]) {
                            let block = block_str.trim();
                            if !block.is_empty() {
                                let mut event_type = "";
                                let mut data_str = "";

                                for line in block.lines() {
                                    let line = line.trim();
                                    if let Some(t) = line.strip_prefix("event: ") {
                                        event_type = t.trim();
                                    } else if let Some(d) = line.strip_prefix("data: ") {
                                        data_str = d.trim();
                                    }
                                }

                                if !data_str.is_empty() && data_str != "[DONE]"
                                    && let Ok(v) = serde_json::from_str::<Value>(data_str) {
                                        match event_type {
                                    "response.output_text.delta" => {
                                        if let Some(text) = v["delta"].as_str()
                                            && !text.is_empty() { yield Ok(StreamChunk::Text(text.to_string())); }
                                    }
                                    "response.function_call_arguments.delta" => {
                                        let idx = v["output_index"].as_u64().unwrap_or(0) as usize;
                                        let entry = tool_map.entry(idx).or_insert_with(|| (String::new(), String::new(), String::new()));
                                        if let Some(a) = v["delta"].as_str() { entry.2.push_str(a); }
                                    }
                                    "response.output_item.added" => {
                                        let item = &v["item"];
                                        if item["type"].as_str() == Some("function_call") {
                                            let idx = v["output_index"].as_u64().unwrap_or(0) as usize;
                                            let entry = tool_map.entry(idx).or_insert_with(|| (String::new(), String::new(), String::new()));
                                            if let Some(id) = item["call_id"].as_str() { entry.0 = id.to_string(); }
                                            if let Some(n) = item["name"].as_str() { entry.1 = n.to_string(); }
                                        }
                                    }
                                    "response.completed" => {
                                        let calls: Vec<(String, String, String)> =
                                            std::mem::take(&mut tool_map).into_values().collect();
                                        for (id, name, args_str) in calls {
                                            if !name.is_empty() {
                                                let args = serde_json::from_str(&args_str).unwrap_or_else(|e| {
                                                    tracing::warn!("Tool '{}' argument JSON parse failed: {e}; raw: {args_str:?}", name);
                                                    serde_json::Value::Object(Default::default())
                                                });
                                                yield Ok(StreamChunk::ToolCall(LlmToolCall { id, name, arguments: args, thought_signature: None }));
                                            }
                                        }
                                        let usage = &v["response"]["usage"];
                                        let in_tok  = usage["input_tokens"].as_u64().unwrap_or(0) as u32;
                                        let out_tok = usage["output_tokens"].as_u64().unwrap_or(0) as u32;
                                        let cache_tok = usage["input_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0) as u32;
                                        if in_tok > 0 || out_tok > 0 {
                                            yield Ok(StreamChunk::Usage(TokenUsage {
                                                input_tokens:       in_tok,
                                                output_tokens:      out_tok,
                                                cache_read_tokens:  cache_tok,
                                                cache_write_tokens: 0,
                                                model:              req_model.clone(),
                                            }));
                                        }
                                        if let Some(reason) = v["response"]["output"].as_array()
                                            .and_then(|arr| arr.iter().find_map(|item| item["finish_reason"].as_str()))
                                            .or_else(|| v["response"]["status"].as_str()) {
                                                yield Ok(StreamChunk::FinishReason(reason.to_string()));
                                        }
                                        yield Ok(StreamChunk::Done);
                                        return;
                                    }
                                    _ => {}
                                }
                                    }
                            }
                        }
                        start = next_start;
                    }
                    if start > 0 {
                        buf.drain(..start);
                    }
                }
                let remaining: Vec<(String, String, String)> =
                    std::mem::take(&mut tool_map).into_values().collect();
                for (id, name, args_str) in remaining {
                    if !name.is_empty() {
                        let args = serde_json::from_str(&args_str).unwrap_or_default();
                        yield Ok(StreamChunk::ToolCall(LlmToolCall { id, name, arguments: args, thought_signature: None }));
                    }
                }
                yield Ok(StreamChunk::Done);
            };
            return Ok(Box::pin(s));
        }

        let mut body = if needs_max_completion_tokens(bare_model_id) {
            json!({
                "model": bare_model_id,
                "messages": Self::to_openai_messages(req),
                "max_completion_tokens": req.max_tokens,
                "stream": true,
                "stream_options": { "include_usage": true }
            })
        } else {
            json!({
                "model": bare_model_id,
                "messages": Self::to_openai_messages(req),
                "max_tokens": req.max_tokens,
                "stream": true,
                "stream_options": { "include_usage": true }
            })
        };
        if !req.tools.is_empty() {
            body["tools"] = Self::build_tools(req);
        }
        if let Some(effort) = &req.reasoning_effort
            && ["low", "medium", "high"].contains(&effort.as_str())
        {
            body["reasoning_effort"] = effort.clone().into();
        }

        let resp = retry_with_backoff(
            "OpenAI::stream",
            3,
            std::time::Duration::from_secs(1),
            |_| {
                let client = self.client.clone();
                let base_url = self.base_url.clone();
                let api_key = self.api_key.clone();
                let body = body.clone();
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
            },
        )
        .await?;

        let mut byte_stream = resp.bytes_stream();
        let s = stream! {
            let mut buf = Vec::new();
            // OpenAI streams tool calls with an `index` field to distinguish
            // parallel calls.  Use a BTreeMap keyed by index so multiple
            // tool calls in one turn are accumulated and emitted separately.
            let mut tool_map: std::collections::BTreeMap<usize, (String, String, String)> =
                std::collections::BTreeMap::new();
            let mut finish_emitted = false;

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk { Ok(c) => c, Err(e) => { yield Err(crate::Error::custom(format!("{e}"))); break; } };
                buf.extend_from_slice(&chunk);

                let mut start = 0;
                while let Some(pos) = buf[start..].iter().position(|&b| b == b'\n') {
                    let end = start + pos;
                    if let Ok(line_str) = std::str::from_utf8(&buf[start..end]) {
                        let line = line_str.trim();
                        if let Some(data) = line.strip_prefix("data: ") {
                            if data == "[DONE]" {
                        let remaining: Vec<(String, String, String)> =
                            std::mem::take(&mut tool_map).into_values().collect();
                        for (id, name, args_str) in remaining {
                            if !name.is_empty() {
                                let args = serde_json::from_str(&args_str).unwrap_or_else(|e| {
                                    tracing::warn!("Tool '{}' argument JSON parse failed: {e}; raw: {args_str:?}", name);
                                    serde_json::Value::Object(Default::default())
                                });
                                yield Ok(StreamChunk::ToolCall(LlmToolCall { id, name, arguments: args, thought_signature: None }));
                            }
                        }
                        yield Ok(StreamChunk::Done);
                        return;
                    }
                    let v: Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => continue };
                    let delta = &v["choices"][0]["delta"];

                    if let Some(text) = delta["content"].as_str()
                        && !text.is_empty() { yield Ok(StreamChunk::Text(text.to_string())); }
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
                    if let Some(reason) = v["choices"][0]["finish_reason"].as_str() {
                        if matches!(reason, "stop" | "tool_calls") {
                            // Emit every accumulated tool call in index order
                            let calls: Vec<(String, String, String)> =
                                std::mem::take(&mut tool_map).into_values().collect();
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
                        if !finish_emitted {
                            yield Ok(StreamChunk::FinishReason(reason.to_string()));
                            finish_emitted = true;
                        }
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
                        start = end + 1;
                    }
                    if start > 0 {
                        buf.drain(..start);
                    }
                }
            }
            // Byte stream exhausted without explicit [DONE] — always send Done
            // so the SSE client doesn't fall back to the blocking endpoint.
            // Also flush any tool calls that arrived without an explicit finish_reason
            // (some OpenAI-compatible providers omit it).
            let remaining: Vec<(String, String, String)> =
                std::mem::take(&mut tool_map).into_values().collect();
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

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;

    #[test]
    fn clean_schema_adds_missing_properties() {
        let mut v = json!({"type": "object"});
        clean_openai_schema(&mut v);
        assert!(v.get("properties").is_some());
    }

    #[test]
    fn clean_schema_does_not_overwrite_existing_properties() {
        let mut v = json!({"type": "object", "properties": {"foo": {"type": "string"}}});
        clean_openai_schema(&mut v);
        assert!(v["properties"]["foo"]["type"].as_str() == Some("string"));
    }

    #[test]
    fn clean_schema_recurses_into_nested() {
        let mut v = json!({
            "type": "object",
            "properties": {
                "nested": {"type": "object"}
            }
        });
        clean_openai_schema(&mut v);
        // The nested object should also get an empty properties
        assert!(v["properties"]["nested"]["properties"].is_object());
    }

    #[test]
    fn clean_schema_handles_arrays() {
        let mut v = json!([{"type": "object"}, {"type": "string"}]);
        clean_openai_schema(&mut v);
        assert!(v[0]["properties"].is_object());
    }

    #[test]
    fn needs_max_completion_tokens_reasoning_models() {
        assert!(needs_max_completion_tokens("o1-preview"));
        assert!(needs_max_completion_tokens("o3-mini"));
        assert!(needs_max_completion_tokens("o4-mini"));
        assert!(needs_max_completion_tokens("gpt-5"));
        assert!(!needs_max_completion_tokens("gpt-4o"));
        assert!(!needs_max_completion_tokens("gpt-4o-mini"));
    }

    #[test]
    fn needs_responses_api_check() {
        assert!(needs_responses_api("gpt-5"));
        assert!(needs_responses_api("o1-pro"));
        assert!(needs_responses_api("o3-pro"));
        assert!(!needs_responses_api("gpt-4o"));
        assert!(!needs_responses_api("o3-mini"));
    }

    #[test]
    fn parse_response_text_only() {
        let body = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": {
                    "content": "Hello, world!"
                }
            }]
        });
        let resp = OpenAiProvider::parse_response(&body);
        assert_eq!(resp.content.as_deref(), Some("Hello, world!"));
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.finish_reason, "stop");
    }

    #[test]
    fn parse_response_with_tool_calls() {
        let body = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": "{\"command\":\"ls -la\"}"
                        }
                    }]
                }
            }]
        });
        let resp = OpenAiProvider::parse_response(&body);
        assert!(resp.content.is_none());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "bash");
        assert_eq!(resp.tool_calls[0].id, "call_123");
        assert_eq!(resp.tool_calls[0].arguments["command"], "ls -la");
    }

    #[test]
    fn parse_responses_api_text() {
        let body = json!({
            "output": [{
                "type": "message",
                "content": [{
                    "type": "output_text",
                    "text": "Response text"
                }]
            }]
        });
        let resp = OpenAiProvider::parse_responses_response(&body);
        assert_eq!(resp.content.as_deref(), Some("Response text"));
        assert!(resp.tool_calls.is_empty());
    }

    #[test]
    fn parse_responses_api_function_call() {
        let body = json!({
            "output": [{
                "type": "function_call",
                "name": "read_file",
                "call_id": "fc_456",
                "arguments": "{\"path\":\"src/main.rs\"}"
            }]
        });
        let resp = OpenAiProvider::parse_responses_response(&body);
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "read_file");
        assert_eq!(resp.tool_calls[0].id, "fc_456");
        assert_eq!(resp.finish_reason, "tool_calls");
    }

    #[test]
    fn to_openai_messages_basic() -> Result<()> {
        // -- Setup & Fixtures
        let req = CompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![
                super::super::LlmMessage {
                    role: "system".into(),
                    content: "You are helpful.".into(),
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
            max_tokens: 4096,
            reasoning_effort: None,
        };
        // -- Exec
        let messages = OpenAiProvider::to_openai_messages(&req);

        // -- Check
        let arr = messages.as_array().ok_or("Should be an array")?;
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["role"], "system");
        assert_eq!(arr[1]["role"], "user");

        Ok(())
    }

    #[test]
    fn build_tools_wraps_in_function_type() -> Result<()> {
        let req = CompletionRequest {
            model: "gpt-4o".into(),
            messages: vec![],
            tools: vec![json!({
                "name": "bash",
                "description": "Run a command",
                "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}
            })],
            max_tokens: 4096,
            reasoning_effort: None,
        };
        // -- Exec
        let tools = OpenAiProvider::build_tools(&req);

        // -- Check
        let arr = tools.as_array().ok_or("Should be an array")?;
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "function");
        assert_eq!(arr[0]["function"]["name"], "bash");

        Ok(())
    }
}
