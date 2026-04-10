use crate::{Error, Result};
use serde_json::{json, Value};

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
pub fn clean_openai_schema(v: &mut Value) {
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

/// Recursively fix JSON Schema fields that Gemini rejects.
pub fn clean_gemini_schema(v: &mut Value) {
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
