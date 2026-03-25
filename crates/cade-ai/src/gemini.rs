use crate::Result;
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio_stream::Stream;

use super::{
    CompletionRequest, CompletionResponse, LlmProvider, LlmToolCall, StreamChunk, TokenUsage,
    bare_model, provider_error, retry_with_backoff,
};

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
const GEMINI_LIST_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/models?pageSize=200";

/// Fetch all generative-content-capable models available to this API key.
/// Filters to models that support `generateContent` and whose names contain "gemini"
/// (excludes embedding models, AQA, TTS, image-gen, etc.).
/// Returns `(bare_id, display_name)` pairs.
pub async fn fetch_gemini_models(api_key: &str) -> Vec<(String, String)> {
    let url = format!("{GEMINI_LIST_URL}&key={api_key}");
    let client = reqwest::Client::new();
    let req = client.get(&url).send();
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
                    if !supports_generate {
                        return None;
                    }

                    // Only "gemini" family (excludes embedding-*, aqa, etc.)
                    if !id.contains("gemini") {
                        return None;
                    }

                    let display = m["displayName"].as_str().unwrap_or(id).to_string();
                    Some((id.to_string(), display))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// An in-process record of a Gemini `cachedContent` object.
struct GeminiCacheEntry {
    /// The cache resource name returned by the API, e.g. `cachedContents/abc123`.
    name: String,
    /// Wall-clock expiry — we use a 55-minute TTL (5-min buffer before the 1-hour server TTL).
    expires_at: std::time::Instant,
}

pub struct GeminiProvider {
    client: Client,
    api_key: String,
    /// Per-content-hash cache of Gemini `cachedContent` names.
    /// Key   = hash(bare_model + system_text + tool_names)
    /// Value = (cache_resource_name, expiry)
    /// Allows every turn to reuse the same system+tools cache rather than
    /// re-sending them as raw tokens, cutting ~90% of the system/tools cost.
    content_cache: Arc<Mutex<HashMap<u64, GeminiCacheEntry>>>,
}

impl GeminiProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::builder()
                .tcp_keepalive(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_else(|_| Client::new()),
            api_key,
            content_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // -- Content-cache helpers

    /// Stable 64-bit hash of the cacheable parts of a request.
    fn content_hash(model: &str, system_text: &Option<String>, tools: &[Value]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        model.hash(&mut h);
        system_text.as_deref().unwrap_or("").hash(&mut h);
        tools.len().hash(&mut h);
        for t in tools {
            t["name"].as_str().unwrap_or("").hash(&mut h);
        }
        h.finish()
    }

    /// Check the in-process cache and return the resource name if still valid.
    fn cached_name(&self, hash: u64) -> Option<String> {
        let cache = self.content_cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(&hash).and_then(|e| {
            if e.expires_at > std::time::Instant::now() {
                Some(e.name.clone())
            } else {
                None
            }
        })
    }

    /// POST to Gemini's `cachedContents` endpoint and return the resource name.
    /// Returns `None` on any error (e.g. payload below the minimum token threshold)
    /// so callers can transparently fall back to sending system+tools inline.
    async fn create_cache(
        &self,
        model: &str,
        system_text: &Option<String>,
        tools: &[Value],
    ) -> Option<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/cachedContents?key={}",
            self.api_key
        );
        let mut body = json!({
            "model": format!("models/{model}"),
            "ttl": "3600s"
        });
        if let Some(sys) = system_text
            && !sys.is_empty()
        {
            body["systemInstruction"] = json!({"parts": [{"text": sys}]});
        }
        if !tools.is_empty() {
            body["tools"] = json!([{"functionDeclarations": tools}]);
        }

        let resp = match self.client.post(&url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("Gemini cache POST failed: {e}");
                return None;
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            // Log at debug — this is expected when payload is below the min-token threshold
            tracing::debug!(
                "Gemini cache creation {status}: {}",
                &text[..text.len().min(300)]
            );
            return None;
        }
        let json: Value = resp.json().await.ok()?;
        json["name"].as_str().map(String::from)
    }

    /// Return a valid cache resource name for the given request, creating one if needed.
    /// Falls back to `None` (= send system+tools inline) if caching is unavailable.
    async fn get_or_create_cache(
        &self,
        model: &str,
        system_text: &Option<String>,
        tools: &[Value],
    ) -> Option<String> {
        // Nothing to cache
        if system_text.as_ref().is_none_or(|s| s.is_empty()) && tools.is_empty() {
            return None;
        }
        let hash = Self::content_hash(model, system_text, tools);

        // Fast path: valid cache entry already in memory
        if let Some(name) = self.cached_name(hash) {
            tracing::debug!("Gemini cache hit: {name}");
            return Some(name);
        }

        // Slow path: create a new cache entry
        let name = self.create_cache(model, system_text, tools).await?;
        tracing::debug!("Gemini cache created: {name}");

        // Store with 55-min TTL (5-min buffer before the 1-hour server TTL)
        let expires_at = std::time::Instant::now() + std::time::Duration::from_secs(3300);
        let mut cache = self.content_cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(
            hash,
            GeminiCacheEntry {
                name: name.clone(),
                expires_at,
            },
        );

        Some(name)
    }

    /// Build the `body["tools"]` / `body["systemInstruction"]` sections,
    /// OR inject a `cachedContent` reference if a valid cache exists.
    /// Returns the (possibly modified) body.
    async fn apply_system_and_tools(
        &self,
        model: &str,
        mut body: Value,
        system_text: &Option<String>,
        tools: &[Value],
    ) -> Value {
        let cache_name = self.get_or_create_cache(model, system_text, tools).await;
        if let Some(name) = cache_name {
            // Gemini requires that cachedContent be set AND systemInstruction/tools be absent
            body["cachedContent"] = json!(name);
        } else {
            // Inline fallback
            if let Some(sys) = system_text {
                body["systemInstruction"] = json!({"parts": [{"text": sys}]});
            }
            if !tools.is_empty() {
                body["tools"] = json!([{"functionDeclarations": tools}]);
            }
        }
        body
    }

    fn url(&self, model: &str, stream: bool) -> String {
        let action = if stream {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        // Strip provider prefix for URL construction
        format!(
            "{GEMINI_BASE}/{}:{action}&key={}",
            bare_model(model),
            self.api_key
        )
    }

    /// Convert our messages to Gemini `contents` format
    fn to_gemini_contents(req: &CompletionRequest) -> (Option<String>, Vec<Value>) {
        let mut system_text = None;
        let mut contents: Vec<Value> = Vec::new();

        // Build a call_id → function_name lookup from all assistant messages
        // so that tool-result messages can supply the correct function name in
        // their `functionResponse.name` field (Gemini requires it to match the
        // original `functionCall.name`; using a hardcoded "tool" causes 400s).
        let mut call_id_to_name: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        // Track tool call IDs that lack thought_signatures.  Newer Gemini models
        // (e.g. gemini-3.x) require a `thoughtSignature` on every `functionCall`
        // part.  Historical calls from other providers (Anthropic, OpenAI) or
        // older Gemini models will not have one — we convert those exchanges to
        // plain text summaries so the model still sees the context without
        // triggering a 400 validation error.
        let mut unsigned_call_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for msg in &req.messages {
            if msg.role == "assistant"
                && let Some(calls) = &msg.tool_calls
            {
                for tc in calls {
                    call_id_to_name.insert(tc.id.clone(), tc.name.clone());
                    if tc.thought_signature.is_none() {
                        unsigned_call_ids.insert(tc.id.clone());
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
                    //
                    // If ANY tool result in this batch belongs to a tool call
                    // that lacked a thought_signature, the entire batch is
                    // converted to plain text.  Gemini rejects mixed
                    // functionResponse / text parts in the same turn when some
                    // of the matching functionCall parts had no signature.
                    let _batch_start = i;
                    let mut has_unsigned = false;
                    {
                        let mut j = i;
                        while j < req.messages.len() && req.messages[j].role == "tool" {
                            if let Some(id) = &req.messages[j].tool_call_id
                                && unsigned_call_ids.contains(id)
                            {
                                has_unsigned = true;
                            }
                            j += 1;
                        }
                    }

                    if has_unsigned {
                        // Convert to text summaries instead of functionResponse
                        let mut text_parts: Vec<String> = Vec::new();
                        while i < req.messages.len() && req.messages[i].role == "tool" {
                            let m = &req.messages[i];
                            let fn_name = m
                                .tool_call_id
                                .as_deref()
                                .and_then(|id| call_id_to_name.get(id).map(String::as_str))
                                .unwrap_or("tool");
                            let content_preview: String = m.content.chars().take(500).collect();
                            let truncated = if m.content.chars().count() > 500 {
                                "…"
                            } else {
                                ""
                            };
                            text_parts.push(format!(
                                "[Result of '{fn_name}': {content_preview}{truncated}]"
                            ));
                            i += 1;
                        }
                        let text = text_parts.join("\n");
                        // Merge with preceding user turn if possible
                        let merged = if let Some(last) = contents.last_mut() {
                            if last.get("role").and_then(|v| v.as_str()) == Some("user") {
                                if let Some(arr) =
                                    last.get_mut("parts").and_then(|v| v.as_array_mut())
                                {
                                    arr.push(json!({"text": text}));
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !merged {
                            contents.push(json!({"role": "user", "parts": [{"text": text}]}));
                        }
                    } else {
                        let mut parts: Vec<Value> = Vec::new();
                        while i < req.messages.len() && req.messages[i].role == "tool" {
                            let m = &req.messages[i];
                            let fn_name = m
                                .tool_call_id
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
                }
                "assistant" if msg.tool_calls.is_some() => {
                    // Check whether any tool call in this message lacks a
                    // thought_signature.  If so, convert the entire turn to a
                    // plain text summary.  Gemini models that enforce thought
                    // signatures reject functionCall parts without one.
                    let has_unsigned = msg
                        .tool_calls
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .any(|tc| unsigned_call_ids.contains(&tc.id));

                    if has_unsigned {
                        // Fallback: emit a text-only model turn summarising
                        // what the assistant did.
                        let mut text_parts: Vec<String> = Vec::new();
                        if !msg.content.is_empty() {
                            text_parts.push(msg.content.clone());
                        }
                        for tc in msg.tool_calls.as_deref().unwrap_or_default() {
                            let args_str = serde_json::to_string(&tc.arguments).unwrap_or_default();
                            let args_preview: String = args_str.chars().take(200).collect();
                            let truncated = if args_str.chars().count() > 200 {
                                "…"
                            } else {
                                ""
                            };
                            text_parts
                                .push(format!("[Called '{}': {args_preview}{truncated}]", tc.name));
                        }
                        let summary = text_parts.join("\n");
                        if !summary.is_empty() {
                            let merged = if let Some(last) = contents.last_mut() {
                                if last.get("role").and_then(|v| v.as_str()) == Some("model") {
                                    if let Some(arr) =
                                        last.get_mut("parts").and_then(|v| v.as_array_mut())
                                    {
                                        arr.push(json!({"text": summary}));
                                        true
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                            if !merged {
                                contents
                                    .push(json!({"role": "model", "parts": [{"text": summary}]}));
                            }
                        }
                        i += 1;
                    } else {
                        // Build parts: optional text first, then all functionCall parts.
                        // Including any text prevents a separate preceding model(text) turn
                        // from being orphaned when the message has both content and calls.
                        let mut all_parts: Vec<Value> = Vec::new();
                        if !msg.content.is_empty() {
                            all_parts.push(json!({"text": msg.content}));
                        }
                        for tc in msg.tool_calls.as_deref().unwrap_or_default().iter() {
                            let mut fc = serde_json::Map::new();
                            fc.insert("name".to_string(), json!(tc.name));
                            fc.insert("args".to_string(), tc.arguments.clone());
                            let mut part = serde_json::Map::new();
                            part.insert("functionCall".to_string(), Value::Object(fc));
                            if let Some(sig) = &tc.thought_signature {
                                part.insert("thoughtSignature".to_string(), json!(sig));
                            }
                            all_parts.push(Value::Object(part));
                        }
                        // If the immediately preceding contents entry is already a model
                        // turn (e.g., a text-only assistant message that preceded this
                        // tool-call message after context trimming removed the user turn
                        // between them), merge our parts into it.  Two consecutive model
                        // turns cause Gemini 400 "function call turn ordering" errors.
                        let merged = if let Some(last) = contents.last_mut() {
                            if last.get("role").and_then(|v| v.as_str()) == Some("model") {
                                if let Some(arr) =
                                    last.get_mut("parts").and_then(|v| v.as_array_mut())
                                {
                                    arr.append(&mut all_parts);
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !merged {
                            contents.push(json!({"role": "model", "parts": all_parts}));
                        }
                        i += 1;
                    }
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
                    // Build the parts array for this user turn.
                    // Gemini vision format: inline_data parts precede the text part.
                    let mut new_parts: Vec<Value> = Vec::new();
                    if let Some(images) = &msg.images {
                        for img in images {
                            new_parts.push(json!({
                                "inline_data": {
                                    "mime_type": img.media_type,
                                    "data": img.data
                                }
                            }));
                        }
                    }
                    if !msg.content.is_empty() {
                        new_parts.push(json!({"text": msg.content}));
                    }
                    if new_parts.is_empty() {
                        new_parts.push(json!({"text": ""}));
                    }

                    // Merge consecutive user turns — Gemini rejects two user
                    // turns in a row (can happen after context trimming strips
                    // an intervening model turn, or when an ephemeral re-prompt
                    // follows a functionResponse user turn).
                    let merged = if msg.images.as_ref().is_none_or(|v| v.is_empty()) {
                        // Only merge plain-text turns; image turns always start a new entry
                        // to avoid the Gemini API rejecting mixed inline_data in a merged part.
                        if let Some(last) = contents.last_mut() {
                            if last.get("role").and_then(|v| v.as_str()) == Some("user") {
                                if let Some(arr) =
                                    last.get_mut("parts").and_then(|v| v.as_array_mut())
                                {
                                    arr.append(&mut new_parts);
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if !merged {
                        contents.push(json!({
                            "role": "user",
                            "parts": new_parts
                        }));
                    }
                    i += 1;
                }
            }
        }
        (system_text, contents)
    }

    fn parse_response(body: &Value) -> CompletionResponse {
        let candidate = &body["candidates"][0];
        let finish_reason = candidate["finishReason"]
            .as_str()
            .unwrap_or("STOP")
            .to_string();
        let mut content = None;
        let mut tool_calls = Vec::new();

        if let Some(parts) = candidate["content"]["parts"].as_array() {
            for part in parts {
                if let Some(text) = part["text"].as_str() {
                    content = Some(text.to_string());
                }
                if let Some(fc) = part.get("functionCall") {
                    let thought_signature = part["thoughtSignature"]
                        .as_str()
                        .or(part["thought_signature"].as_str())
                        .map(String::from);
                    tool_calls.push(LlmToolCall {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: fc["name"].as_str().unwrap_or("").to_string(),
                        arguments: fc["args"].clone(),
                        thought_signature,
                    });
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
impl LlmProvider for GeminiProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        let (system_text, contents) = Self::to_gemini_contents(req);
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
                clean_gemini_schema(&mut params);
                json!({
                    "name": s["name"],
                    "description": s["description"],
                    "parameters": params
                })
            })
            .collect();

        let base_body = json!({ "contents": contents });
        let body = self
            .apply_system_and_tools(bare_model(&req.model), base_body, &system_text, &tools)
            .await;

        let url = self.url(&req.model, false);
        retry_with_backoff(
            "Gemini::complete",
            3,
            std::time::Duration::from_secs(1),
            |_| {
                let client = self.client.clone();
                let url = url.clone();
                let body = body.clone();
                async move {
                    let resp = client.post(&url).json(&body).send().await?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        return Err(provider_error("Gemini", status, &text));
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
        let req_model = req.model.clone(); // extracted before async_stream to avoid lifetime capture
        let (system_text, contents) = Self::to_gemini_contents(req);
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
                clean_gemini_schema(&mut params);
                json!({"name": s["name"], "description": s["description"], "parameters": params})
            })
            .collect();

        let base_body = json!({ "contents": contents });
        let body = self
            .apply_system_and_tools(bare_model(&req_model), base_body, &system_text, &tools)
            .await;

        let url = self.url(&req_model, true);
        let resp = retry_with_backoff(
            "Gemini::stream",
            3,
            std::time::Duration::from_secs(1),
            |_| {
                let client = self.client.clone();
                let url = url.clone();
                let body = body.clone();
                async move {
                    let resp = client.post(&url).json(&body).send().await?;
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        return Err(provider_error("Gemini", status, &text));
                    }
                    Ok(resp)
                }
            },
        )
        .await?;

        let mut byte_stream = resp.bytes_stream();
        let s = stream! {
            let mut buf = String::new();
            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk { Ok(c) => c, Err(e) => { yield Err(crate::Error::custom(format!("{e}"))); break; } };
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
                    if let Some(candidates) = v.get("candidates").and_then(|c| c.as_array())
                        && let Some(candidate) = candidates.first() {
                            if let Some(parts) = candidate["content"]["parts"].as_array() {
                                for part in parts {
                                    if let Some(text) = part["text"].as_str()
                                        && !text.is_empty() {
                                            if part["thought"].as_bool() == Some(true) {
                                                yield Ok(StreamChunk::Reasoning(text.to_string()));
                                            } else {
                                                yield Ok(StreamChunk::Text(text.to_string()));
                                            }
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
                                        let thought_signature = part["thoughtSignature"].as_str()
                                            .or(part["thought_signature"].as_str())
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

                            if let Some(reason) = candidate["finishReason"].as_str() {
                                yield Ok(StreamChunk::FinishReason(reason.to_string()));
                                yield Ok(StreamChunk::Done); return;
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
