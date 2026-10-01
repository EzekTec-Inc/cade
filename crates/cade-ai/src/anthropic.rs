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
    provider_error, retry_with_backoff,
};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Fetch all models available to this API key from Anthropic's models endpoint.
/// Returns `(id, display_name)` pairs using the shared gateway-aware discovery.
/// Returns empty Vec on any error or timeout.
pub async fn fetch_anthropic_models(api_key: &str) -> Vec<(String, String)> {
    crate::discovery::default_models("anthropic", api_key)
        .await
        .into_iter()
        .filter_map(|entry| {
            entry
                .id
                .split_once('/')
                .map(|(_, id)| (id.to_owned(), entry.display_name.clone()))
        })
        .collect()
}

// endregion: --- Tests

/// Returns true if the given Anthropic model expects the newer
/// `thinking.type=adaptive` + `output_config.effort` request shape.
///
/// Compatibility classification comes from editable metadata; future major
/// versions are not automatically assigned an unverified thinking protocol.
#[cfg(test)]
pub(crate) fn supports_adaptive_thinking(model: &str) -> bool {
    let (provider, bare) = model.split_once('/').unwrap_or(("anthropic", model));
    crate::runtime::RuntimeRegistry::configured()
        .metadata(provider, bare)
        .thinking
        .as_deref()
        == Some("adaptive")
}

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: Option<String>,
    provider_name: String,
    models: crate::SharedModelRegistry,
}

impl AnthropicProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        let base = base_url.filter(|s| !s.trim().is_empty()).or_else(|| {
            std::env::var("ANTHROPIC_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
        });

        Self {
            client: crate::utils::build_standard_http_client(),
            api_key,
            base_url: base,
            provider_name: "anthropic".into(),
            models: crate::runtime::shared_registry(),
        }
    }

    pub fn with_registry(
        mut self,
        provider_name: String,
        models: crate::SharedModelRegistry,
    ) -> Self {
        self.provider_name = provider_name;
        self.models = models;
        self
    }

    pub(crate) fn with_provider_definition(
        mut self,
        definition: &crate::provider_registry::ProviderDef,
    ) -> Self {
        self.client = crate::utils::build_provider_http_client(Some(definition));
        self
    }

    fn metadata(&self, model: &str) -> crate::runtime::ModelMetadata {
        let registry = self.models.read();
        registry.metadata(
            &self.provider_name,
            registry.upstream_model(&self.provider_name, model),
        )
    }

    fn validate_request(&self, req: &CompletionRequest) -> Result<()> {
        self.validate_model(&req.model)?;
        if req.max_tokens == 0 {
            return Err(crate::Error::custom("max_tokens must be greater than zero"));
        }
        if !req.tools.is_empty() && self.metadata(&req.model).tools == Some(false) {
            return Err(crate::Error::custom(format!(
                "Model '{}' is registered without tool support",
                req.model
            )));
        }
        Ok(())
    }

    pub fn endpoint_url(&self) -> String {
        match &self.base_url {
            Some(base) => {
                let trimmed = base.trim_end_matches('/');
                if trimmed.ends_with("/messages") {
                    trimmed.to_string()
                } else if trimmed.ends_with("/v1") {
                    format!("{trimmed}/messages")
                } else {
                    format!("{trimmed}/v1/messages")
                }
            }
            None => API_URL.to_string(),
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
            let mut block = json!({
                "type": "text",
                "text": m.content,
            });
            if let Some(cc) = &m.cache_control {
                block["cache_control"] = json!({ "type": cc });
            }
            system_blocks.push(block);
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
                    if let Some(cc) = &m.cache_control {
                        let content_val = if let Some(images) = &m.images
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
                            blocks.push(json!({
                                "type": "text",
                                "text": m.content,
                                "cache_control": { "type": cc }
                            }));
                            json!(blocks)
                        } else {
                            json!([{
                                "type": "text",
                                "text": m.content,
                                "cache_control": { "type": cc }
                            }])
                        };
                        anthropic_messages.push(json!({"role": m.role, "content": content_val}));
                        i += 1;
                    } else if let Some(images) = &m.images
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
                    } else {
                        anthropic_messages.push(json!({"role": m.role, "content": m.content}));
                        i += 1;
                    }
                }
            }
        }

        // Build tools array in Anthropic format, mapping cache_control annotations directly
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|schema| {
                let params = schema
                    .get("parameters")
                    .filter(|v| !v.is_null())
                    .or_else(|| schema.get("input_schema").filter(|v| !v.is_null()))
                    .cloned()
                    .unwrap_or(json!({"type": "object", "properties": {}, "required": []}));
                let mut tool_obj = json!({
                    "name": schema["name"],
                    "description": schema["description"],
                    "input_schema": params
                });
                if let Some(cc) = schema.get("cache_control") {
                    tool_obj["cache_control"] = cc.clone();
                }
                tool_obj
            })
            .collect();

        let mut body = json!({
            "model": self.models.read().upstream_model(&self.provider_name, &req.model),
            "max_tokens": req.max_tokens,
            "messages": anthropic_messages,
            "stream": stream
        });

        let metadata = self.metadata(&req.model);
        if let Some(effort) = &req.reasoning_effort {
            if metadata.thinking.as_deref() == Some("adaptive") && effort != "none" {
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
            } else if metadata.thinking.as_deref() == Some("budget")
                && let Some(budget) = metadata
                    .thinking_budgets
                    .as_ref()
                    .and_then(|map| map.get(effort))
                && *budget > 0
                && (*budget as u64) < u64::from(req.max_tokens)
            {
                body["thinking"] = json!({
                    "type": "enabled",
                    "budget_tokens": budget
                });
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
        self.validate_request(req)?;
        let body = self.build_body(req, false);
        retry_with_backoff(
            "Anthropic::complete",
            3,
            std::time::Duration::from_secs(1),
            |_| {
                let client = self.client.clone();
                let api_key = self.api_key.clone();
                let body = body.clone();
                let url = self.endpoint_url();
                async move {
                    let resp = client
                        .post(&url)
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
        self.validate_request(req)?;
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
                let url = self.endpoint_url();
                async move {
                    let resp = client
                        .post(&url)
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
            // Calls are withheld until message_stop: a later in-band error or
            // premature EOF must never expose tools from an unsuccessful turn.
            let mut pending_tools: std::collections::BTreeMap<usize, (String, String, String, Value)> =
                std::collections::BTreeMap::new();
            let mut completed_tools = Vec::new();
            let mut thinking_text = String::new();
            let mut in_thinking = false;
            // Accumulate token usage across message_start + message_delta
            let mut input_tokens: u32 = 0;
            let mut output_tokens: u32 = 0;
            let mut cache_read_tokens: u32 = 0;
            let mut cache_write_tokens: u32 = 0;
            let mut finish_reason = None;

            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => { yield Err(crate::Error::custom(format!("Anthropic stream transport error: {e}"))); return; }
                };
                buf.extend_from_slice(&chunk);

                // Process complete SSE lines
                let mut start = 0;
                while let Some(pos) = buf[start..].iter().position(|&b| b == b'\n') {
                    let end = start + pos;
                    if let Ok(line_str) = std::str::from_utf8(&buf[start..end]) {
                        let line = line_str.trim();
                        if !line.is_empty() && !line.starts_with(':')
                            && let Some(data) = line.strip_prefix("data:").map(str::trim_start) {
                                if data.is_empty() { start = end + 1; continue; }
                                let event: Value = match serde_json::from_str(data) {
                                    Ok(v) => v,
                                    Err(e) => { yield Err(crate::Error::custom(format!("Invalid Anthropic SSE JSON: {e}"))); return; }
                                };

                    match event["type"].as_str().unwrap_or("") {
                        "error" => {
                            let kind = event["error"]["type"].as_str().unwrap_or("unknown_error");
                            let message = event["error"]["message"].as_str().unwrap_or("Provider reported a streaming failure");
                            let status = event["error"]["status_code"].as_u64().or_else(|| event["status"].as_u64())
                                .and_then(|n| u16::try_from(n).ok()).filter(|n| (400..=599).contains(n))
                                .unwrap_or(match kind {
                                    "invalid_request_error" => 400, "authentication_error" => 401,
                                    "permission_error" => 403, "not_found_error" => 404,
                                    "rate_limit_error" => 429, "overloaded_error" => 529, _ => 500,
                                });
                            yield Err(crate::Error::Provider { status, msg: format!("Anthropic stream error ({kind}): {message}") });
                            return;
                        }
                        "content_block_delta" => {
                            match event["delta"]["type"].as_str().unwrap_or("") {
                                "text_delta" => {
                                    if let Some(text) = event["delta"]["text"].as_str() {
                                        yield Ok(StreamChunk::Text(text.to_string()));
                                    }
                                }
                                "input_json_delta" => {
                                    if let Some(partial) = event["delta"]["partial_json"].as_str() {
                                        let index = event["index"].as_u64().unwrap_or(0) as usize;
                                        let Some(tool) = pending_tools.get_mut(&index) else {
                                            yield Err(crate::Error::custom("Anthropic tool arguments arrived without a content_block_start")); return;
                                        };
                                        tool.2.push_str(partial);
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
                                    let index = event["index"].as_u64().unwrap_or(0) as usize;
                                    let block = &event["content_block"];
                                    let initial = block.get("input").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!({}));
                                    pending_tools.insert(index, (
                                        block["id"].as_str().unwrap_or_default().into(),
                                        block["name"].as_str().unwrap_or_default().into(), String::new(), initial,
                                    ));
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
                            let index = event["index"].as_u64().unwrap_or(0) as usize;
                            if let Some((id, name, arguments, initial)) = pending_tools.remove(&index) {
                                let args = if arguments.trim().is_empty() { initial } else {
                                    match serde_json::from_str::<Value>(&arguments) {
                                        Ok(args) => args,
                                        Err(e) => { yield Err(crate::Error::custom(format!("Invalid Anthropic tool arguments for '{name}': {e}"))); return; }
                                    }
                                };
                                if id.is_empty() || name.is_empty() || !args.is_object() {
                                    yield Err(crate::Error::custom("Malformed Anthropic tool call")); return;
                                }
                                completed_tools.push(LlmToolCall {
                                    id,
                                    name,
                                    arguments: args,
                                    thought_signature: None,
                                });
                            }
                        }
                        "message_start" => {
                            // e.g. {"type":"message_start","message":{"usage":{"input_tokens":N,"cache_read_input_tokens":N,"cache_creation_input_tokens":N}}}
                            if let Some(n) = event["message"]["usage"]["input_tokens"].as_u64() {
                                input_tokens = n.try_into().unwrap_or(u32::MAX);
                            }
                            if let Some(n) = event["message"]["usage"]["cache_read_input_tokens"].as_u64() {
                                cache_read_tokens = n.try_into().unwrap_or(u32::MAX);
                            }
                            if let Some(n) = event["message"]["usage"]["cache_creation_input_tokens"].as_u64() {
                                cache_write_tokens = n.try_into().unwrap_or(u32::MAX);
                            }
                        }
                        "message_delta" => {
                            // e.g. {"type":"message_delta","usage":{"output_tokens":N}}
                            if let Some(n) = event["usage"]["output_tokens"].as_u64() {
                                // Anthropic reports the cumulative output count.
                                output_tokens = n.try_into().unwrap_or(u32::MAX);
                            }
                            if let Some(reason) = event["delta"]["stop_reason"].as_str() { finish_reason = Some(reason.to_string()); }
                        }
                        "message_stop" => {
                            if !pending_tools.is_empty() {
                                yield Err(crate::Error::custom("Incomplete Anthropic stream: message_stop before tool blocks completed")); return;
                            }
                            for call in completed_tools { yield Ok(StreamChunk::ToolCall(call)); }
                            if input_tokens > 0 || output_tokens > 0 || cache_read_tokens > 0 || cache_write_tokens > 0 {
                                yield Ok(StreamChunk::Usage(TokenUsage {
                                    input_tokens,
                                    output_tokens,
                                    cache_read_tokens,
                                    cache_write_tokens,
                                    model: req_model.clone(),
                                }));
                            }
                            if let Some(reason) = event["stop_reason"].as_str().map(String::from).or(finish_reason) {
                                yield Ok(StreamChunk::FinishReason(reason));
                            }
                            yield Ok(StreamChunk::Done);
                            return;
                        }
                        _ => {}
                     }
                             }
                     } else { yield Err(crate::Error::custom("Invalid UTF-8 in Anthropic SSE")); return; }
                    start = end + 1;
                }
                if start > 0 {
                    buf.drain(..start);
                }
            }
            yield Err(crate::Error::Provider { status: 502,
                msg: "Incomplete Anthropic stream: EOF before message_stop".into() });
        };

        Ok(Box::pin(s))
    }

    async fn complete_structured(
        &self,
        req: &CompletionRequest,
        schema: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.validate_request(req)?;
        let metadata = self.metadata(&req.model);
        if metadata.native_structured != Some(true) || metadata.tools == Some(false) {
            return crate::types::structured_fallback(self, req, &schema).await;
        }
        use tracing::Instrument;
        let span = crate::gen_ai_span!("anthropic", req);

        let fut = async move {
            let mut body = self.build_body(req, false);

            // Set up a single forced structured output tool matching the required schema
            let forced_tool = json!({
                "name": "structured_output",
                "description": "Output the final structured JSON response matching the required schema.",
                "input_schema": schema
            });
            body["tools"] = json!([forced_tool]);
            body["tool_choice"] = json!({
                "type": "tool",
                "name": "structured_output"
            });

            let res = retry_with_backoff(
                "Anthropic::complete_structured",
                3,
                std::time::Duration::from_secs(1),
                |_| {
                    let client = self.client.clone();
                    let api_key = self.api_key.clone();
                    let body = body.clone();
                    let url = self.endpoint_url();
                    async move {
                        let resp = client
                            .post(&url)
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
                        Ok(json)
                    }
                },
            )
            .await?;

            // Extract the input payload of the "tool_use" content block with name "structured_output"
            if let Some(content_array) = res["content"].as_array() {
                for block in content_array {
                    if block["type"] == "tool_use" && block["name"] == "structured_output" {
                        let input = block["input"].clone();
                        if !input.is_null() {
                            return Ok(input);
                        }
                    }
                }
            }

            Err(crate::Error::custom(format!(
                "Anthropic structured completions tool_use block not found. Response: {}",
                res
            )))
        };

        fut.instrument(span).await
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
        let provider = AnthropicProvider::new("sk-test".into(), None);
        let req = CompletionRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![
                super::super::LlmMessage {
                    role: "system".into(),
                    content: "You are a helpful assistant.".into(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                    cache_control: None,
                },
                super::super::LlmMessage {
                    role: "user".into(),
                    content: "Hello".into(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                    cache_control: None,
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
        let provider = AnthropicProvider::new("sk-test".into(), None);
        let mut req = CompletionRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![super::super::LlmMessage {
                role: "user".into(),
                content: "Hello".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            }],
            tools: vec![json!({
                "name": "bash",
                "description": "Run command",
                "parameters": {"type": "object"}
            })],
            max_tokens: 8192,
            reasoning_effort: None,
        };
        let manager = crate::resolve_prompt_cache_manager(&req.model);
        manager.optimize(&mut req);
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
        let provider = AnthropicProvider::new("sk-test".into(), None);
        let req = CompletionRequest {
            model: "claude-3-7-sonnet-20250219".into(),
            messages: vec![super::super::LlmMessage {
                role: "user".into(),
                content: "Think hard".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            }],
            tools: vec![],
            max_tokens: 8192,
            reasoning_effort: Some("high".into()),
        };
        let body = provider.build_body(&req, false);
        assert!(body["thinking"].is_object());
        assert_eq!(body["thinking"]["type"], "enabled");
        // The editable compatibility seed selects the budget without inflating
        // the caller's explicit output limit.
        assert_eq!(body["thinking"]["budget_tokens"], 4096);
        assert_eq!(body["max_tokens"], 8192);
        // No output_config in the legacy shape.
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn build_body_with_reasoning_effort_adaptive() {
        // Claude 4+ (e.g. sonnet-4-5) requires adaptive thinking + output_config.effort.
        let provider = AnthropicProvider::new("sk-test".into(), None);
        let req = CompletionRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![super::super::LlmMessage {
                role: "user".into(),
                content: "Think hard".into(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
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
        // Unknown future families need discovery/config, not optimistic guesses.
        assert!(!supports_adaptive_thinking("claude-sonnet-5-20260101"));
        assert!(!supports_adaptive_thinking("claude-opus-10-20270101"));
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
        let provider = AnthropicProvider::new("sk-test".into(), None);
        let req = CompletionRequest {
            model: "claude-sonnet-4-5-20250929".into(),
            messages: vec![
                super::super::LlmMessage {
                    role: "user".into(),
                    content: "Do two things".into(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                    cache_control: None,
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
                    cache_control: None,
                },
                super::super::LlmMessage {
                    role: "tool".into(),
                    content: "result 1".into(),
                    tool_call_id: Some("t1".into()),
                    tool_calls: None,
                    images: None,
                    cache_control: None,
                },
                super::super::LlmMessage {
                    role: "tool".into(),
                    content: "result 2".into(),
                    tool_call_id: Some("t2".into()),
                    tool_calls: None,
                    images: None,
                    cache_control: None,
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

    #[test]
    fn test_anthropic_provider_endpoint_resolution() {
        let p_default = AnthropicProvider::new("sk-test".into(), None);
        assert_eq!(
            p_default.endpoint_url(),
            "https://api.anthropic.com/v1/messages"
        );

        let p_custom =
            AnthropicProvider::new("sk-test".into(), Some("http://127.0.0.1:8787".into()));
        assert_eq!(p_custom.endpoint_url(), "http://127.0.0.1:8787/v1/messages");

        let p_custom_v1 =
            AnthropicProvider::new("sk-test".into(), Some("http://127.0.0.1:8787/v1".into()));
        assert_eq!(
            p_custom_v1.endpoint_url(),
            "http://127.0.0.1:8787/v1/messages"
        );

        let p_custom_messages = AnthropicProvider::new(
            "sk-test".into(),
            Some("http://127.0.0.1:8787/v1/messages".into()),
        );
        assert_eq!(
            p_custom_messages.endpoint_url(),
            "http://127.0.0.1:8787/v1/messages"
        );
    }
}
