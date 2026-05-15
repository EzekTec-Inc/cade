use crate::Result;
use async_stream::stream;
use async_trait::async_trait;
use futures::StreamExt;
use parking_lot::Mutex;
use reqwest::Client;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;

use super::{
    CompletionRequest, CompletionResponse, LlmProvider, LlmToolCall, StreamChunk, TokenUsage,
    bare_model, clean_gemini_schema, inline_schema_refs, provider_error, retry_with_backoff,
};

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

/// Server-side TTL bounds (Gemini API limits).
/// Below 60s the API rejects with 400; above ~604_800s (7 days) it caps.
const GEMINI_TTL_MIN_SECS: u64 = 60;
const GEMINI_TTL_MAX_SECS: u64 = 86_400; // 1 day — anything longer is wasted
// because skill/memory edits invalidate the hash anyway.
/// Default TTL used when the env override is absent/invalid.
/// 1 hour matches the previous hardcoded value; it's a reasonable middle
/// ground for typical coding sessions.
const GEMINI_TTL_DEFAULT_SECS: u64 = 3_600;

/// P7: parse a `CADE_GEMINI_CACHE_TTL_SECS`-style value into a clamped TTL.
///
/// Pure helper for testability.  The production wrapper [`gemini_cache_ttl_secs`]
/// reads the env var and delegates here.
///
/// Returns `GEMINI_TTL_DEFAULT_SECS` when the input is `None`, empty, non-numeric,
/// or zero.  Otherwise clamps to `[GEMINI_TTL_MIN_SECS, GEMINI_TTL_MAX_SECS]`.
fn parse_gemini_ttl(raw: Option<&str>) -> u64 {
    let parsed = raw
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|v| *v > 0);
    match parsed {
        Some(v) => v.clamp(GEMINI_TTL_MIN_SECS, GEMINI_TTL_MAX_SECS),
        None => GEMINI_TTL_DEFAULT_SECS,
    }
}

/// P7: read `CADE_GEMINI_CACHE_TTL_SECS` env var, applying defaults + clamping.
fn gemini_cache_ttl_secs() -> u64 {
    parse_gemini_ttl(std::env::var("CADE_GEMINI_CACHE_TTL_SECS").ok().as_deref())
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
        let cache = self.content_cache.lock();
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
            "ttl": format!("{}s", gemini_cache_ttl_secs())
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

        // P7: in-memory expiry tracks the server-side TTL with a 5-min
        // safety buffer (or 10% of the TTL, whichever is smaller, but
        // never below 30 s) so we don't reuse a stale handle right at the
        // edge of expiry.
        let server_ttl = gemini_cache_ttl_secs();
        let buffer = (server_ttl / 10).clamp(30, 300);
        let local_ttl = server_ttl.saturating_sub(buffer).max(30);
        let expires_at = std::time::Instant::now() + std::time::Duration::from_secs(local_ttl);
        let mut cache = self.content_cache.lock();
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

        for msg in &req.messages {
            if msg.role == "assistant"
                && let Some(calls) = &msg.tool_calls
            {
                for tc in calls {
                    call_id_to_name.insert(tc.id.clone(), tc.name.clone());
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
                    if system_text.is_none() {
                        system_text = Some(msg.content.clone());
                    } else {
                        // For Gemini, dynamic system messages (like working_set) are appended
                        // as a user message to avoid busting the systemInstruction cache.
                        if let Some(last) = contents.last_mut()
                            && last["role"].as_str() == Some("user")
                        {
                            if let Some(parts) = last["parts"].as_array_mut() {
                                parts.push(json!({ "text": msg.content }));
                            }
                        } else {
                            contents.push(json!({
                                "role": "user",
                                "parts": [{ "text": msg.content }]
                            }));
                        }
                    }
                    i += 1;
                }
                "tool" => {
                    // Batch ALL consecutive tool messages into one user turn.
                    // Resolves each function name from the pre-built lookup so the
                    // functionResponse.name always matches its functionCall.name.
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
                "assistant" if msg.tool_calls.is_some() => {
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
                        } else {
                            part.insert("thoughtSignature".to_string(), json!(""));
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
                            if let Some(arr) = last.get_mut("parts").and_then(|v| v.as_array_mut())
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
                inline_schema_refs(&mut params);
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
                inline_schema_refs(&mut params);
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
            let mut buf = Vec::new();
            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk { Ok(c) => c, Err(e) => { yield Err(crate::Error::custom(format!("{e}"))); break; } };
                buf.extend_from_slice(&chunk);

                let mut start = 0;
                while let Some(pos) = buf[start..].iter().position(|&b| b == b'\n') {
                    let end = start + pos;
                    if let Ok(line_str) = std::str::from_utf8(&buf[start..end]) {
                        let line = line_str.trim();
                        if let Some(data) = line.strip_prefix("data: ")
                            && let Ok(mut v) = serde_json::from_str::<Value>(data) {

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
                    if let Some(candidates) = v.get_mut("candidates").and_then(|c| c.as_array_mut())
                        && let Some(candidate) = candidates.first_mut() {
                            if let Some(parts) = candidate.get_mut("content").and_then(|c| c.get_mut("parts")).and_then(|p| p.as_array_mut()) {
                                for part in parts {
                                    if let Some(text) = part["text"].as_str()
                                        && !text.is_empty() {
                                            if part["thought"].as_bool() == Some(true) {
                                                yield Ok(StreamChunk::Reasoning(text.to_string()));
                                            } else {
                                                yield Ok(StreamChunk::Text(text.to_string()));
                                            }
                                        }
                                    if let Some(fc) = part.get_mut("functionCall") {
                                        let name = fc["name"].as_str().unwrap_or("").to_string();
                                        let arguments = {
                                            let raw = fc["args"].take();
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
                    start = end + 1;
                }
                if start > 0 {
                    buf.drain(..start);
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

#[cfg(test)]
mod p7_ttl_tests {
    use super::*;

    #[test]
    fn parse_gemini_ttl_unset_returns_default() {
        assert_eq!(parse_gemini_ttl(None), GEMINI_TTL_DEFAULT_SECS);
    }

    #[test]
    fn parse_gemini_ttl_empty_returns_default() {
        assert_eq!(parse_gemini_ttl(Some("")), GEMINI_TTL_DEFAULT_SECS);
        assert_eq!(parse_gemini_ttl(Some("   ")), GEMINI_TTL_DEFAULT_SECS);
    }

    #[test]
    fn parse_gemini_ttl_garbage_returns_default() {
        assert_eq!(parse_gemini_ttl(Some("abc")), GEMINI_TTL_DEFAULT_SECS);
        assert_eq!(parse_gemini_ttl(Some("3600s")), GEMINI_TTL_DEFAULT_SECS);
    }

    #[test]
    fn parse_gemini_ttl_zero_returns_default() {
        assert_eq!(parse_gemini_ttl(Some("0")), GEMINI_TTL_DEFAULT_SECS);
    }

    #[test]
    fn parse_gemini_ttl_below_min_clamps_up() {
        assert_eq!(parse_gemini_ttl(Some("10")), GEMINI_TTL_MIN_SECS);
        assert_eq!(parse_gemini_ttl(Some("59")), GEMINI_TTL_MIN_SECS);
    }

    #[test]
    fn parse_gemini_ttl_above_max_clamps_down() {
        assert_eq!(parse_gemini_ttl(Some("999999")), GEMINI_TTL_MAX_SECS);
    }

    #[test]
    fn parse_gemini_ttl_in_range_returns_value() {
        assert_eq!(parse_gemini_ttl(Some("60")), 60);
        assert_eq!(parse_gemini_ttl(Some("300")), 300);
        assert_eq!(parse_gemini_ttl(Some("7200")), 7200);
        assert_eq!(parse_gemini_ttl(Some("86400")), 86_400);
    }

    #[test]
    fn parse_gemini_ttl_strips_whitespace() {
        assert_eq!(parse_gemini_ttl(Some(" 600 ")), 600);
    }
}
