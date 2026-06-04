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
    bare_model, clean_openai_schema, provider_error, retry_with_backoff,
    seal_top_level_additional_properties,
};

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";

fn needs_max_completion_tokens(model: &str) -> bool {
    let bare = bare_model(model).to_lowercase();
    bare.starts_with("gpt-4.5")
        || bare.starts_with("gpt-5")
        || bare.starts_with("o1")
        || bare.starts_with("o3")
        || bare.starts_with("o4")
}

fn is_o_series(model: &str) -> bool {
    let bare = bare_model(model).to_lowercase();
    bare.starts_with("o1") || bare.starts_with("o3") || bare.starts_with("o4")
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
    let mut req_builder = client
        .get(models_url)
        .header("Authorization", format!("Bearer {api_key}"));

    if models_url.contains("openrouter.ai") {
        req_builder = req_builder
            .header("HTTP-Referer", "https://github.com/EzekTec-Inc/CADE")
            .header("X-Title", "CADE");
    }
    let req = req_builder.send();
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

const OPENAI_MAX_TOOLS: usize = 128;
const PRIORITY_TOOL_NAMES: &[&str] = &[
    "load_skill",
    "search_memory",
    "conversation_search",
    "archival_memory_search",
    "update_memory",
    "update_memory_typed",
    "memory_apply_patch",
];

fn tool_name(schema: &Value) -> Option<&str> {
    if let Some(function) = schema.get("function") {
        if let Some(name) = function.get("name").and_then(Value::as_str) {
            return Some(name);
        }
    }
    schema.get("name").and_then(Value::as_str)
}

fn is_priority_tool(schema: &Value) -> bool {
    tool_name(schema).is_some_and(|name| {
        PRIORITY_TOOL_NAMES.contains(&name)
            || name.starts_with("serena__")
            || name.starts_with("cade-rag__")
            || name.starts_with("cade-ide-mcp__")
    })
}

fn capped_tools(schemas: &[Value]) -> Vec<&Value> {
    let mut selected: Vec<&Value> = schemas
        .iter()
        .filter(|schema| is_priority_tool(schema))
        .collect();
    selected.extend(
        schemas
            .iter()
            .filter(|schema| !is_priority_tool(schema))
            .take(OPENAI_MAX_TOOLS.saturating_sub(selected.len())),
    );
    selected.truncate(OPENAI_MAX_TOOLS);
    selected
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        let base = base_url.unwrap_or_else(|| OPENAI_URL.to_string());

        let client = if base.contains("openrouter.ai") {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                "HTTP-Referer",
                reqwest::header::HeaderValue::from_static("https://github.com/EzekTec-Inc/CADE"),
            );
            headers.insert("X-Title", reqwest::header::HeaderValue::from_static("CADE"));

            Client::builder()
                .tcp_keepalive(std::time::Duration::from_secs(60))
                .connect_timeout(std::time::Duration::from_secs(15))
                .timeout(std::time::Duration::from_secs(120))
                .default_headers(headers)
                .build()
                .unwrap_or_else(|_| Client::new())
        } else {
            crate::utils::build_standard_http_client()
        };

        Self {
            client,
            api_key,
            base_url: base,
        }
    }

    fn to_openai_messages(req: &CompletionRequest) -> Value {
        let is_o_series = is_o_series(&req.model);

        let mut combined_system = String::new();
        let mut processed_messages = Vec::new();

        for m in &req.messages {
            if m.role == "system" {
                if !combined_system.is_empty() {
                    combined_system.push_str("\n\n");
                }
                combined_system.push_str(&m.content);
            } else {
                processed_messages.push(m);
            }
        }

        let mut json_messages = Vec::new();

        if !combined_system.is_empty() {
            let role = if is_o_series { "developer" } else { "system" };
            json_messages.push(json!({"role": role, "content": combined_system}));
        }

        for m in processed_messages {
            let value = match m.role.as_str() {
                "tool" => {
                    let tid = m.tool_call_id.as_deref().unwrap_or("");
                    if tid.is_empty() {
                        tracing::warn!("OpenAI: skipping tool message with missing tool_call_id");
                        continue;
                    }
                    json!({
                        "role": "tool",
                        "tool_call_id": tid,
                        "content": m.content
                    })
                }
                "assistant" if m.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) => {
                    let tcs: Vec<Value> = m.tool_calls.as_deref().unwrap_or_default().iter().map(|tc| json!({
                        "id": tc.id,
                        "type": "function",
                        "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
                    })).collect();
                    let content = if m.content.is_empty() {
                        Value::Null
                    } else {
                        Value::String(m.content.clone())
                    };
                    json!({"role": "assistant", "content": content, "tool_calls": tcs})
                }
                "assistant" => {
                    let content = if m.content.is_empty() {
                        Value::Null
                    } else {
                        Value::String(m.content.clone())
                    };
                    json!({"role": "assistant", "content": content})
                }
                _ => {
                    // When images are attached, build a multi-part content array.
                    if m.role == "user"
                        && let Some(images) = &m.images
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
                        json!({"role": m.role, "content": parts})
                    } else {
                        json!({"role": m.role, "content": m.content})
                    }
                }
            };
            json_messages.push(value);
        }

        json!(json_messages)
    }

    fn parse_response(body: &Value) -> CompletionResponse {
        let choice = &body["choices"][0];
        let finish_reason = choice["finish_reason"]
            .as_str()
            .unwrap_or("stop")
            .to_string();
        let msg = &choice["message"];
        let mut content = msg["content"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        if let Some(reasoning) = msg["reasoning"].as_str().filter(|s| !s.is_empty()) {
            if let Some(c) = &mut content {
                *c = format!("<reasoning>\n{}\n</reasoning>\n\n{}", reasoning, c);
            } else {
                content = Some(format!("<reasoning>\n{}\n</reasoning>", reasoning));
            }
        }
        let tool_calls: Vec<LlmToolCall> = msg["tool_calls"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|tc| LlmToolCall {
                id: tc["id"].as_str().unwrap_or("").to_string(),
                name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
                arguments: {
                    let arg_str = tc["function"]["arguments"].as_str().unwrap_or("{}").trim();
                    let arg_str = if arg_str.is_empty() { "{}" } else { arg_str };
                    serde_json::from_str(arg_str).unwrap_or_else(|_| json!({}))
                },
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
        let tools: Vec<Value> = capped_tools(&req.tools)
            .iter()
            .map(|s| {
                let params_val = s
                    .get("parameters")
                    .or_else(|| s.get("input_schema"))
                    .or_else(|| s.get("function").and_then(|f| f.get("parameters")));
                    
                let mut params = params_val
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or(json!({"type": "object", "properties": {}, "required": []}));
                crate::utils::inline_schema_refs(&mut params);
                clean_openai_schema(&mut params);
                seal_top_level_additional_properties(&mut params);

                let name = tool_name(s).unwrap_or("unknown_tool").to_string();
                let description = s
                    .get("description")
                    .or_else(|| s.get("function").and_then(|f| f.get("description")))
                    .cloned()
                    .unwrap_or(Value::Null);

                if name == "unknown_tool" {
                    tracing::warn!(
                        "OpenAI: missing tool name in schema: {}",
                        serde_json::to_string(s).unwrap_or_default()
                    );
                }

                json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": params,
                        "strict": false
                    }
                })
            })
            .collect();
        json!(tools)
    }

}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        use tracing::Instrument;
        let span = crate::gen_ai_span!("openai", req);

        let fut = async move {
            let bare_model_id = if self.base_url == OPENAI_URL {
                bare_model(&req.model)
            } else {
                &req.model
            };
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
            if is_o_series(&req.model)
                && let Some(effort) = &req.reasoning_effort
            {
                let mapped = match effort.as_str() {
                    "xhigh" => "high",
                    e @ ("low" | "medium" | "high") => e,
                    _ => "",
                };
                if !mapped.is_empty() {
                    body["reasoning_effort"] = mapped.into();
                }
            }
            if self.base_url.contains("openrouter.ai") && req.reasoning_effort.is_some() {
                body["include_reasoning"] = true.into();
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
                        let mut req = client.post(&base_url).json(&body);
                        if !api_key.is_empty() {
                            req = req.bearer_auth(&api_key);
                        }
                        let resp = req.send().await?;
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
        };

        fut.instrument(span).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let req_model = req.model.clone();
        let bare_model_id = if self.base_url == OPENAI_URL {
            bare_model(&req_model)
        } else {
            &req_model
        };
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
        if is_o_series(&req.model)
            && let Some(effort) = &req.reasoning_effort
        {
            let mapped = match effort.as_str() {
                "xhigh" => "high",
                e @ ("low" | "medium" | "high") => e,
                _ => "",
            };
            if !mapped.is_empty() {
                body["reasoning_effort"] = mapped.into();
            }
        }
        if self.base_url.contains("openrouter.ai") && req.reasoning_effort.is_some() {
            body["include_reasoning"] = true.into();
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
                    let mut req = client.post(&base_url).json(&body);
                    if !api_key.is_empty() {
                        req = req.bearer_auth(&api_key);
                    }
                    let resp = req.send().await?;
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
                while start <= buf.len() {
                    let Some(pos) = buf.get(start..).and_then(|s| s.iter().position(|&b| b == b'\n')) else { break };
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
                    let v_res = serde_json::from_str::<Value>(data);
                    if let Ok(v) = v_res {
                        let delta = &v["choices"][0]["delta"];

                        if let Some(text) = delta["content"].as_str()
                            && !text.is_empty() { yield Ok(StreamChunk::Text(text.to_string())); }
                        if let Some(reasoning) = delta["reasoning"].as_str()
                            && !reasoning.is_empty() { yield Ok(StreamChunk::Reasoning(reasoning.to_string())); }
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
                        }
                        start = end + 1;
                    }
                }
                if start > 0 {
                    buf.drain(..start);
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

    async fn complete_structured(
        &self,
        req: &CompletionRequest,
        mut schema: serde_json::Value,
    ) -> Result<serde_json::Value> {
        use tracing::Instrument;
        let span = crate::gen_ai_span!("openai", req);

        crate::utils::enforce_strict_json_schema(&mut schema);

        let fut = async move {
            let bare_model_id = if self.base_url == OPENAI_URL {
                bare_model(&req.model)
            } else {
                &req.model
            };

            let mut body = if needs_max_completion_tokens(bare_model_id) {
                json!({
                    "model": bare_model_id,
                    "messages": Self::to_openai_messages(req),
                    "max_completion_tokens": req.max_tokens,
                    "response_format": {
                        "type": "json_schema",
                        "json_schema": {
                            "name": "structured_output",
                            "strict": true,
                            "schema": schema
                        }
                    }
                })
            } else {
                json!({
                    "model": bare_model_id,
                    "messages": Self::to_openai_messages(req),
                    "max_tokens": req.max_tokens,
                    "response_format": {
                        "type": "json_schema",
                        "json_schema": {
                            "name": "structured_output",
                            "strict": true,
                            "schema": schema
                        }
                    }
                })
            };

            if !req.tools.is_empty() {
                body["tools"] = Self::build_tools(req);
            }
            if is_o_series(&req.model)
                && let Some(effort) = &req.reasoning_effort
            {
                let mapped = match effort.as_str() {
                    "xhigh" => "high",
                    e @ ("low" | "medium" | "high") => e,
                    _ => "",
                };
                if !mapped.is_empty() {
                    body["reasoning_effort"] = mapped.into();
                }
            }

            let res = retry_with_backoff(
                "OpenAI::complete_structured",
                3,
                std::time::Duration::from_secs(1),
                |_| {
                    let client = self.client.clone();
                    let base_url = self.base_url.clone();
                    let api_key = self.api_key.clone();
                    let body = body.clone();
                    async move {
                        let mut req = client.post(&base_url).json(&body);
                        if !api_key.is_empty() {
                            req = req.bearer_auth(&api_key);
                        }
                        let resp = req.send().await?;
                        if !resp.status().is_success() {
                            let status = resp.status();
                            let text = resp.text().await.unwrap_or_default();
                            return Err(provider_error("OpenAI", status, &text));
                        }
                        let parsed = Self::parse_response(&resp.json::<Value>().await?);
                        Ok(parsed)
                    }
                },
            )
            .await?;

            let text = res.content.unwrap_or_default();
            let json_str = crate::utils::clean_json_markers(&text);
            serde_json::from_str(&json_str).map_err(|e| {
                crate::Error::custom(format!(
                    "OpenAI structured output parsing failed: {e}. Raw response: {text}"
                ))
            })
        };

        fut.instrument(span).await
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests;
