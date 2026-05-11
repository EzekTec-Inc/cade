use crate::{Error, Result};
use serde_json::{Value, json};

/// Which HTTP status codes are worth retrying (transient / rate-limit errors).
/// 400, 401, 403, 404 are permanent — fail fast.
pub fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

/// Returns true if the error looks like a transient / rate-limit failure.
pub(crate) fn is_retryable_error(e: &Error) -> bool {
    // Check embedded reqwest errors
    if let Error::Reqwest(re) = e {
        if re.is_connect() || re.is_timeout() || re.is_request() {
            return true;
        }
        if let Some(status) = re.status() {
            return is_retryable_status(status);
        }
    }
    // Check structured provider errors
    if let Error::Provider { status, .. } = e
        && let Ok(status_code) = reqwest::StatusCode::from_u16(*status)
    {
        return is_retryable_status(status_code);
    }
    false
}

/// Retry an async fallible operation with exponential backoff.
pub async fn retry_with_backoff<F, Fut, T>(
    op_name: &str,
    max_attempts: u32,
    base_delay: std::time::Duration,
    mut f: F,
) -> Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_err = crate::Error::custom("retry_with_backoff: no attempts made");
    for attempt in 1..=max_attempts {
        match f(attempt).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let retryable = is_retryable_error(&e);
                if attempt < max_attempts && retryable {
                    let delay = std::cmp::min(
                        base_delay * 2u32.pow(attempt - 1),
                        std::time::Duration::from_secs(8),
                    );
                    tracing::warn!(
                        "{op_name}: attempt {attempt}/{max_attempts} failed ({e:#}), retrying in {}ms…",
                        delay.as_millis()
                    );
                    tokio::time::sleep(delay).await;
                    last_err = e;
                } else {
                    return Err(e);
                }
            }
        }
    }
    Err(last_err)
}

/// Extract a human-readable error message from a provider's JSON error body.
pub fn provider_error(provider: &str, status: reqwest::StatusCode, body: &str) -> Error {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or_else(|| body.trim().to_string());
    Error::Provider {
        status: status.as_u16(),
        msg: format!("{provider} {status}: {msg}"),
    }
}

/// Strip optional `provider/` prefix from a model handle.
pub fn bare_model(model: &str) -> &str {
    if let Some(pos) = model.find('/') {
        &model[pos + 1..]
    } else {
        model
    }
}

/// Recursively fix JSON Schema fields that OpenAI rejects.
///
/// Strips unsupported meta keys (`$schema`, `title`, `x-google-*`) at every
/// nesting level. Ensures object-type schemas have a `properties` field.
///
/// NOTE: does NOT add `additionalProperties: false` — that's controlled by
/// `seal_top_level_additional_properties` so it only affects the tool params
/// root, not nested objects (which commonly have loose shapes in MCP tools).
pub fn clean_openai_schema(v: &mut Value) {
    match v {
        Value::Object(map) => {
            if map.get("type").and_then(|t| t.as_str()) == Some("object")
                && !map.contains_key("properties")
            {
                map.insert("properties".to_string(), json!({}));
            }
            // Strip keys OpenAI doesn't support in tool schemas
            map.remove("$schema");
            map.remove("title");
            map.remove("x-google-enum-descriptions");
            map.remove("x-google-enum-deprecated");
            map.remove("x-google-identifier");
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

/// Seal ONLY the top-level object schema with `additionalProperties: false`.
///
/// OpenAI's fallback validation prefers this at the params root to reject
/// unknown top-level keys. Nested objects are left loose because MCP tools
/// frequently have optional sub-fields.
pub fn seal_top_level_additional_properties(v: &mut Value) {
    if let Value::Object(map) = v
        && map.get("type").and_then(|t| t.as_str()) == Some("object")
        && !map.contains_key("additionalProperties")
    {
        map.insert("additionalProperties".to_string(), json!(false));
    }
}

/// Resolve all local `#/$defs/<Name>` JSON Schema references by inlining the
/// referenced definition.  Must be called **before** `clean_gemini_schema` —
/// stripping `$defs` first would leave dangling `$ref` pointers.
///
/// Only handles local `#/$defs/...` refs.  External URL refs are left as-is
/// and will be stripped by `clean_gemini_schema`.
pub fn inline_schema_refs(v: &mut Value) {
    let defs = match v
        .as_object()
        .and_then(|m| m.get("$defs"))
        .and_then(|d| d.as_object())
    {
        Some(d) => d.clone(),
        None => return,
    };
    inline_refs_with_defs(v, &defs, 0);
}

/// Recursive workhorse for `inline_schema_refs`.  `depth` guards against
/// circular `$ref` chains — bails at depth > 10 to prevent stack overflow.
fn inline_refs_with_defs(v: &mut Value, defs: &serde_json::Map<String, Value>, depth: u32) {
    if depth > 10 {
        return;
    }
    match v {
        Value::Object(map) => {
            // Clone the ref string to release the immutable borrow on `map`
            // before we mutate `*v`.
            if let Some(ref_str) = map.get("$ref").and_then(|r| r.as_str()).map(String::from)
                && let Some(type_name) = ref_str.strip_prefix("#/$defs/")
                && let Some(def) = defs.get(type_name)
            {
                *v = def.clone();
                inline_refs_with_defs(v, defs, depth + 1);
                return;
            }
            for val in map.values_mut() {
                inline_refs_with_defs(val, defs, depth);
            }
        }
        Value::Array(arr) => {
            for val in arr.iter_mut() {
                inline_refs_with_defs(val, defs, depth);
            }
        }
        _ => {}
    }
}

/// Recursively strip JSON Schema fields that Gemini's `functionDeclarations`
/// format rejects.
///
/// Gemini accepts a strict subset of JSON Schema.  The following cause 400s:
/// - `$schema`, `$ref`, `$defs`  — JSON Schema meta/reference fields
/// - `additionalProperties`      — not supported in Gemini function schemas
/// - `nullable`                  — defensive; not confirmed to cause errors
/// - `x-google-*`               — Google API extension annotations (e.g.
///   `x-google-identifier`, `x-google-enum-descriptions`) that appear in MCP
///   tool schemas from Google services like Stitch
///
/// Call `inline_schema_refs` first to resolve `$ref` pointers before stripping
/// `$defs`.
pub fn clean_gemini_schema(v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.remove("$schema");
            map.remove("$ref");
            map.remove("$defs");
            map.remove("additionalProperties");
            map.remove("nullable");
            map.remove("deprecated");
            // Strip all x-google-* extension fields
            let x_google_keys: Vec<String> = map
                .keys()
                .filter(|k| k.starts_with("x-google-"))
                .cloned()
                .collect();
            for k in x_google_keys {
                map.remove(&k);
            }
            if let Some(Value::Array(types)) = map.get("type") {
                if let Some(primary_type) = types.iter().find(|t| t.as_str() != Some("null")) {
                    if let Some(s) = primary_type.as_str() {
                        map.insert("type".to_string(), Value::String(s.to_uppercase()));
                    } else {
                        map.insert("type".to_string(), primary_type.clone());
                    }
                } else {
                    map.remove("type");
                }
            } else if let Some(Value::String(s)) = map.get("type") {
                let up = s.to_uppercase();
                map.insert("type".to_string(), Value::String(up));
            }
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
