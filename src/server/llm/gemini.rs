use anyhow::Result;
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::pin::Pin;
use tokio_stream::Stream;

use super::{bare_model, provider_error, retry_with_backoff, CompletionRequest, CompletionResponse, LlmProvider, LlmToolCall, StreamChunk, TokenUsage};

/// Recursively strip JSON Schema fields that Gemini's functionDeclarations format rejects.
///
/// Gemini accepts a strict subset of JSON Schema — the following fields cause 400 errors
/// when present anywhere in the parameter schema tree:
///   - `$schema`            — JSON Schema meta-schema declaration
///   - `additionalProperties` — not supported in Gemini's schema dialect
///
/// This function walks the entire Value tree and removes those keys in-place.
fn clean_gemini_schema(v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.remove("$schema");
            map.remove("additionalProperties");
            for val in map.values_mut() {
                clean_gemini_schema(val);
            }
        }
        Value::Array(arr) => {
            for val in arr.iter_mut() {
                clean_gemini_schema(val);
            }
        }
        _ => {}
    }
}

const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const GEMINI_LIST_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models?pageSize=200";

/// Fetch all generative-content-capable models available to this API key.
/// Filters to models that support `generateContent` and whose names contain "gemini"
/// (excludes embedding models, AQA, TTS, image-gen, etc.).
/// Returns `(bare_id, display_name)` pairs.
pub async fn fetch_gemini_models(api_key: &str) -> Vec<(String, String)> {
    let url = format!("{GEMINI_LIST_URL}&key={api_key}");
    let client = reqwest::Client::new();
    let req = client.get(&url).send();
    let resp = match tokio::time::timeout(std::time::Duration::from_secs(5), req).await {
        Ok(Ok(r))  => r,
        Ok(Err(_)) | Err(_) => return vec![],
    };
    if !resp.status().is_success() { return vec![]; }
    let Ok(body) = resp.json::<Value>().await else { return vec![]; };

    body["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    // name format: "models/gemini-2.0-flash"
                    let full_name = m["name"].as_str()?;
                    let id = full_name.strip_prefix("models/").unwrap_or(full_name);

                    // Only models that support generateContent
                    let supports_generate = m["supportedGenerationMethods"]
                        .as_array()
                        .map(|a| a.iter().any(|v| v.as_str() == Some("generateContent")))
                        .unwrap_or(false);
                    if !supports_generate { return None; }

                    // Only "gemini" family (excludes embedding-*, aqa, etc.)
                    if !id.contains("gemini") { return None; }

                    let display = m["displayName"].as_str().unwrap_or(id).to_string();
                    Some((id.to_string(), display))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub struct GeminiProvider {
    client: Client,
    api_key: String,
}

impl GeminiProvider {
    pub fn new(api_key: String) -> Self {
        Self { client: Client::new(), api_key }
    }

    fn url(&self, model: &str, stream: bool) -> String {
        let action = if stream { "streamGenerateContent?alt=sse" } else { "generateContent" };
        // Strip provider prefix for URL construction
        format!("{GEMINI_BASE}/{}:{action}&key={}", bare_model(model), self.api_key)
    }

    /// Convert our messages to Gemini `contents` format
    fn to_gemini_contents(req: &CompletionRequest) -> (Option<String>, Vec<Value>) {
        let mut system_text = None;
        let mut contents = Vec::new();

        for msg in &req.messages {
            match msg.role.as_str() {
                "system" => {
                    system_text = Some(msg.content.clone());
                }
                "tool" => {
                    contents.push(json!({
                        "role": "user",
                        "parts": [{ "functionResponse": {
                            "name": "tool",
                            "response": { "result": msg.content }
                        }}]
                    }));
                }
                "assistant" if msg.tool_calls.is_some() => {
                    let calls: Vec<Value> = msg.tool_calls.as_ref().unwrap().iter().map(|tc| json!({
                        "functionCall": { "name": tc.name, "args": tc.arguments }
                    })).collect();
                    contents.push(json!({"role": "model", "parts": calls}));
                }
                "assistant" => {
                    contents.push(json!({
                        "role": "model",
                        "parts": [{"text": msg.content}]
                    }));
                }
                _ => {
                    contents.push(json!({
                        "role": "user",
                        "parts": [{"text": msg.content}]
                    }));
                }
            }
        }
        (system_text, contents)
    }

    fn parse_response(body: &Value) -> CompletionResponse {
        let candidate = &body["candidates"][0];
        let finish_reason = candidate["finishReason"].as_str().unwrap_or("STOP").to_string();
        let mut content = None;
        let mut tool_calls = Vec::new();

        if let Some(parts) = candidate["content"]["parts"].as_array() {
            for part in parts {
                if let Some(text) = part["text"].as_str() {
                    content = Some(text.to_string());
                }
                if let Some(fc) = part.get("functionCall") {
                    tool_calls.push(LlmToolCall {
                        id:        uuid::Uuid::new_v4().to_string(),
                        name:      fc["name"].as_str().unwrap_or("").to_string(),
                        arguments: fc["args"].clone(),
                    });
                }
            }
        }
        CompletionResponse { content, tool_calls, finish_reason }
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let (system_text, contents) = Self::to_gemini_contents(req);
        let tools: Vec<Value> = req.tools.iter().map(|s| {
            let mut params = s["parameters"].clone();
            clean_gemini_schema(&mut params);
            json!({
                "name": s["name"],
                "description": s["description"],
                "parameters": params
            })
        }).collect();

        let mut body = json!({ "contents": contents });
        if let Some(sys) = &system_text {
            body["systemInstruction"] = json!({"parts": [{"text": sys}]});
        }
        if !tools.is_empty() {
            body["tools"] = json!([{"functionDeclarations": tools}]);
        }

        let url = self.url(&req.model, false);
        retry_with_backoff("Gemini::complete", 3, std::time::Duration::from_secs(1), |_| {
            let client = self.client.clone();
            let url    = url.clone();
            let body   = body.clone();
            async move {
                let resp = client.post(&url).json(&body).send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    return Err(provider_error("Gemini", status, &text));
                }
                Ok(Self::parse_response(&resp.json::<Value>().await?))
            }
        }).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let (system_text, contents) = Self::to_gemini_contents(req);
        let tools: Vec<Value> = req.tools.iter().map(|s| {
            let mut params = s["parameters"].clone();
            clean_gemini_schema(&mut params);
            json!({"name": s["name"], "description": s["description"], "parameters": params})
        }).collect();

        let mut body = json!({ "contents": contents });
        if let Some(sys) = &system_text {
            body["systemInstruction"] = json!({"parts": [{"text": sys}]});
        }
        if !tools.is_empty() {
            body["tools"] = json!([{"functionDeclarations": tools}]);
        }

        let url = self.url(&req.model, true);
        let resp = retry_with_backoff("Gemini::stream", 3, std::time::Duration::from_secs(1), |_| {
            let client = self.client.clone();
            let url    = url.clone();
            let body   = body.clone();
            async move {
                let resp = client.post(&url).json(&body).send().await?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    return Err(provider_error("Gemini", status, &text));
                }
                Ok(resp)
            }
        }).await?;

        let mut byte_stream = resp.bytes_stream();
        let s = stream! {
            let mut buf = String::new();
            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk { Ok(c) => c, Err(e) => { yield Err(anyhow::anyhow!("{e}")); break; } };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf = buf[pos + 1..].to_string();
                    let data = match line.strip_prefix("data: ") { Some(d) => d, None => continue };
                    let v: Value = match serde_json::from_str(data) { Ok(v) => v, Err(_) => continue };

                    // 1. Always check for usage metadata at the root if it's present
                    if let Some(usage) = v.get("usageMetadata") {
                        let in_tok  = usage["promptTokenCount"].as_u64().unwrap_or(0) as u32;
                        let out_tok = usage["candidatesTokenCount"].as_u64().unwrap_or(0) as u32;
                        if in_tok > 0 || out_tok > 0 {
                            yield Ok(StreamChunk::Usage(TokenUsage { input_tokens: in_tok, output_tokens: out_tok }));
                        }
                    }

                    // 2. Parse candidates (content, tool calls, finishReason)
                    if let Some(candidates) = v.get("candidates").and_then(|c| c.as_array()) {
                        if let Some(candidate) = candidates.first() {
                            if let Some(parts) = candidate["content"]["parts"].as_array() {
                                for part in parts {
                                    if let Some(text) = part["text"].as_str() {
                                        if !text.is_empty() { yield Ok(StreamChunk::Text(text.to_string())); }
                                    }
                                    if let Some(fc) = part.get("functionCall") {
                                        yield Ok(StreamChunk::ToolCall(LlmToolCall {
                                            id:        uuid::Uuid::new_v4().to_string(),
                                            name:      fc["name"].as_str().unwrap_or("").to_string(),
                                            arguments: fc["args"].clone(),
                                        }));
                                    }
                                }
                            }
                            
                            if candidate["finishReason"].as_str().is_some() {
                                yield Ok(StreamChunk::Done); return;
                            }
                        }
                    }
                }
            }
            // Byte stream exhausted without an explicit finishReason chunk.
            // Always yield Done so the SSE handler sends [DONE] and the client
            // doesn't trigger its fallback-to-blocking-endpoint path.
            yield Ok(StreamChunk::Done);
        };
        Ok(Box::pin(s))
    }
}
