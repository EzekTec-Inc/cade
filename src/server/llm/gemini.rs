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

        // Build a call_id → function_name lookup from all assistant messages
        // so that tool-result messages can supply the correct function name in
        // their `functionResponse.name` field (Gemini requires it to match the
        // original `functionCall.name`; using a hardcoded "tool" causes 400s).
        let mut call_id_to_name: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for msg in &req.messages {
            if msg.role == "assistant" {
                if let Some(calls) = &msg.tool_calls {
                    for tc in calls {
                        call_id_to_name.insert(tc.id.clone(), tc.name.clone());
                    }
                }
            }
        }

        // Use indexed iteration so consecutive "tool" messages can be batched
        // into a single user turn.  Gemini requires that ALL function responses
        // for a parallel function-call model turn appear in ONE user turn; emitting
        // a separate user turn per tool result causes a 400 "function call turn
        // ordering" error.
        let mut i = 0;
        while i < req.messages.len() {
            let msg = &req.messages[i];
            match msg.role.as_str() {
                "system" => {
                    system_text = Some(msg.content.clone());
                    i += 1;
                }
                "tool" => {
                    // Batch ALL consecutive tool messages into one user turn.
                    // Resolves each function name from the pre-built lookup so the
                    // functionResponse.name always matches its functionCall.name.
                    let mut parts: Vec<Value> = Vec::new();
                    while i < req.messages.len() && req.messages[i].role == "tool" {
                        let m = &req.messages[i];
                        let fn_name = m.tool_call_id
                            .as_deref()
                            .and_then(|id| call_id_to_name.get(id).map(String::as_str))
                            .or(m.tool_call_id.as_deref())
                            .unwrap_or("tool")
                            .to_string();
                        parts.push(json!({ "functionResponse": {
                            "name": fn_name,
                            "response": { "result": m.content }
                        }}));
                        i += 1;
                    }
                    contents.push(json!({"role": "user", "parts": parts}));
                }
                "assistant" if msg.tool_calls.is_some() => {
                    // Build parts: optional text first, then all functionCall parts.
                    // Including any text prevents a separate preceding model(text) turn
                    // from being orphaned when the message has both content and calls.
                    let mut all_parts: Vec<Value> = Vec::new();
                    if !msg.content.is_empty() {
                        all_parts.push(json!({"text": msg.content}));
                    }
                    for tc in msg.tool_calls.as_ref().unwrap().iter() {
                        let mut fc = serde_json::Map::new();
                        fc.insert("name".to_string(), json!(tc.name));
                        fc.insert("args".to_string(), tc.arguments.clone());
                        if let Some(sig) = &tc.thought_signature {
                            fc.insert("thought_signature".to_string(), json!(sig));
                        }
                        all_parts.push(json!({ "functionCall": fc }));
                    }
                    // If the immediately preceding contents entry is already a model
                    // turn (e.g., a text-only assistant message that preceded this
                    // tool-call message after context trimming removed the user turn
                    // between them), merge our parts into it.  Two consecutive model
                    // turns cause Gemini 400 "function call turn ordering" errors.
                    let merged = if let Some(last) = contents.last_mut() {
                        if last.get("role").and_then(|v| v.as_str()) == Some("model") {
                            if let Some(arr) = last.get_mut("parts").and_then(|v| v.as_array_mut()) {
                                arr.extend(all_parts.drain(..));
                                true
                            } else { false }
                        } else { false }
                    } else { false };
                    if !merged {
                        contents.push(json!({"role": "model", "parts": all_parts}));
                    }
                    i += 1;
                }
                "assistant" => {
                    // Gemini rejects empty text parts — only add the message if
                    // it has actual content (pure tool-call turns have none).
                    if !msg.content.is_empty() {
                        contents.push(json!({
                            "role": "model",
                            "parts": [{"text": msg.content}]
                        }));
                    }
                    i += 1;
                }
                _ => {
                    contents.push(json!({
                        "role": "user",
                        "parts": [{"text": msg.content}]
                    }));
                    i += 1;
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
                        id:                uuid::Uuid::new_v4().to_string(),
                        name:              fc["name"].as_str().unwrap_or("").to_string(),
                        arguments:         fc["args"].clone(),
                        thought_signature: fc["thought_signature"].as_str().map(String::from),
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
        let req_model = req.model.clone();   // extracted before async_stream to avoid lifetime capture
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

        let url = self.url(&req_model, true);
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
                        let in_tok   = usage["promptTokenCount"].as_u64().unwrap_or(0) as u32;
                        let out_tok  = usage["candidatesTokenCount"].as_u64().unwrap_or(0) as u32;
                        let cache_tok = usage["cachedContentTokenCount"].as_u64().unwrap_or(0) as u32;
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

                    // 2. Parse candidates (content, tool calls, finishReason)
                    if let Some(candidates) = v.get("candidates").and_then(|c| c.as_array()) {
                        if let Some(candidate) = candidates.first() {
                            if let Some(parts) = candidate["content"]["parts"].as_array() {
                                for part in parts {
                                    if let Some(text) = part["text"].as_str() {
                                        if !text.is_empty() { yield Ok(StreamChunk::Text(text.to_string())); }
                                    }
                                    if let Some(fc) = part.get("functionCall") {
                                        let name = fc["name"].as_str().unwrap_or("").to_string();
                                        let arguments = {
                                            let raw = fc["args"].clone();
                                            if raw.is_object() || raw.is_null() {
                                                raw
                                            } else {
                                                tracing::warn!("Tool '{}' arguments are not an object: {:?}", name, raw);
                                                serde_json::Value::Object(Default::default())
                                            }
                                        };
                                        let thought_signature = part
                                            .get("functionCall")
                                            .and_then(|fc| fc["thought_signature"].as_str())
                                            .map(String::from);
                                        yield Ok(StreamChunk::ToolCall(LlmToolCall {
                                            id: uuid::Uuid::new_v4().to_string(),
                                            name,
                                            arguments,
                                            thought_signature,
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
