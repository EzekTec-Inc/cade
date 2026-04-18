//! Pure API-client helpers for the cade-gui WASM app.
//!
//! This module contains **no browser dependencies**.  It handles:
//!   * Building absolute request URLs from `base_url + path`.
//!   * Building the `Authorization: Bearer <token>` header value.
//!   * Parsing JSON response bodies into the `cade-api-types` wire types.
//!   * Classifying HTTP status codes into a small typed error enum.
//!
//! The actual network I/O (gloo-net / fetch) lives in `http_wasm.rs` and is
//! compiled only for `wasm32`.  Keeping the logic here pure means native
//! `cargo test` covers URL building, header construction, JSON parsing, and
//! error classification without a browser.

use cade_api_types::{AgentInfo, ChatMessage, HealthInfo};

/// Typed error surface for API calls.  The wasm fetch wrapper produces
/// `Transport`; the pure logic here produces `Unauthorized`, `Server`, or
/// `Decode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 401 Unauthorized — token is missing or wrong.
    Unauthorized,
    /// 5xx or any non-2xx/non-401 response.  Carries the status code.
    Server { status: u16 },
    /// JSON body did not match the expected wire type.
    Decode { message: String },
    /// Network-level failure (wasm-side only; surfaced here for uniformity).
    Transport { message: String },
}

impl core::fmt::Display for ApiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "unauthorized"),
            Self::Server { status } => write!(f, "server error (status {status})"),
            Self::Decode { message } => write!(f, "decode error: {message}"),
            Self::Transport { message } => write!(f, "transport error: {message}"),
        }
    }
}

impl std::error::Error for ApiError {}

// ── SSE stream event types ─────────────────────────────────────────────

/// Events emitted by an SSE message stream.
///
/// Used by `send_message_stream` to relay parsed SSE frames back to the
/// caller in a type-safe way (as opposed to raw JSON).
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// The server assigned a conversation ID.
    ConversationId(String),
    /// A chunk of assistant text.
    Text(String),
    /// A chunk of chain-of-thought reasoning text.
    Reasoning(String),
    /// The assistant invoked a tool.
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    /// Token usage statistics for the turn.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        model: Option<String>,
    },
    /// The reason the stream ended (e.g. "stop", "length").
    FinishReason(String),
}

/// Build the absolute URL for an API path.
///
/// Rules:
///   * `base` may or may not end with `/`.  Both forms must produce the
///     same result.
///   * `path` must start with `/`; callers supply server-relative paths.
///   * No query-string handling here — callers that need `?foo=bar` pass
///     it as part of `path`.
pub fn build_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}{path}")
}

/// Build the value for the `Authorization` header.
///
/// The returned string is always of the form `"Bearer <token>"`.  Callers
/// are responsible for trimming the token before passing it in; this is a
/// zero-logic helper so it can be inlined.
pub fn bearer_header(token: &str) -> String {
    format!("Bearer {token}")
}

/// Map an HTTP status code + body into either a parsed value or a typed
/// error.  Keeps the pure logic together so wasm and native paths share it.
pub fn parse_health(status: u16, body: &str) -> Result<HealthInfo, ApiError> {
    decode_or_error(status, body)
}

/// Same as `parse_health`, but for the `GET /v1/agents` list.
pub fn parse_agents(status: u16, body: &str) -> Result<Vec<AgentInfo>, ApiError> {
    decode_or_error(status, body)
}

/// Server envelope for `GET /v1/agents/:id/messages`.
///
/// The server wraps the message list in `{ "messages": [...], "query": "" }`.
/// We only care about the `messages` array.
#[derive(serde::Deserialize)]
struct MessagesEnvelope {
    messages: Vec<ChatMessage>,
    #[serde(default)]
    has_more: bool,
}

/// Parse the response from `GET /v1/agents/:id/messages`.
///
/// Handles the server's `{ "messages": [...] }` envelope, status-code
/// classification, and JSON decode errors — same contract as `parse_health`.
pub fn parse_messages(status: u16, body: &str) -> Result<Vec<ChatMessage>, ApiError> {
    let envelope: MessagesEnvelope = decode_or_error(status, body)?;
    Ok(envelope.messages)
}

/// Like `parse_messages` but also returns the `has_more` flag.
pub fn parse_messages_paged(
    status: u16,
    body: &str,
) -> Result<(Vec<ChatMessage>, bool), ApiError> {
    let envelope: MessagesEnvelope = decode_or_error(status, body)?;
    Ok((envelope.messages, envelope.has_more))
}

fn decode_or_error<T>(status: u16, body: &str) -> Result<T, ApiError>
where
    T: serde::de::DeserializeOwned,
{
    match status {
        200..=299 => serde_json::from_str::<T>(body).map_err(|e| ApiError::Decode {
            message: e.to_string(),
        }),
        401 => Err(ApiError::Unauthorized),
        s => Err(ApiError::Server { status: s }),
    }
}

// ── SSE event parsing ──────────────────────────────────────────────────

// ── Conversation types ──────────────────────────────────────────────────

/// Minimal info about a server-side conversation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConversationInfo {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub message_count: u32,
    #[serde(default)]
    pub updated_at: String,
}/// Server envelope for `GET /v1/agents/:id/conversations`.
#[derive(serde::Deserialize)]
struct ConversationsEnvelope {
    conversations: Vec<ConversationInfo>,
}

/// Parse the response from `GET /v1/agents/:id/conversations`.
pub fn parse_conversations(status: u16, body: &str) -> Result<Vec<ConversationInfo>, ApiError> {
    let envelope: ConversationsEnvelope = decode_or_error(status, body)?;
    Ok(envelope.conversations)
}

/// Parse a single `ConversationInfo` from a create/get response.
pub fn decode_conversations_single(
    status: u16,
    body: &str,
) -> Result<ConversationInfo, ApiError> {
    decode_or_error(status, body)
}

/// Build the URL for listing or creating conversations.
pub fn conversations_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}/conversations"))
}

// ── Memory blocks ───────────────────────────────────────────────────────

/// A single memory block returned by `GET /v1/agents/:id/memory`.
///
/// Mirrors the server's `{label, value, description, tier}` shape.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct MemoryBlock {
    pub label: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
}

#[derive(serde::Deserialize)]
struct MemoryEnvelope {
    blocks: Vec<MemoryBlock>,
}

/// Parse the response from `GET /v1/agents/:id/memory`.
pub fn parse_memory(status: u16, body: &str) -> Result<Vec<MemoryBlock>, ApiError> {
    let env: MemoryEnvelope = decode_or_error(status, body)?;
    Ok(env.blocks)
}

/// Build the URL for the memory-collection endpoint.
pub fn memory_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}/memory"))
}

/// Build the URL for a single memory-block upsert/delete endpoint.
pub fn memory_block_url(server: &str, agent_id: &str, label: &str) -> String {
    build_url(
        server,
        &format!("/v1/agents/{agent_id}/memory/{label}"),
    )
}

/// Build the request body for `PUT /v1/agents/:id/memory/:label`.
pub fn upsert_memory_body(value: &str, description: Option<&str>) -> String {
    match description {
        Some(d) => serde_json::json!({ "value": value, "description": d }).to_string(),
        None => serde_json::json!({ "value": value }).to_string(),
    }
}

/// Classify the HTTP response for an upsert: server returns 204 on
/// success.  Anything else is surfaced as an [`ApiError`].
pub fn classify_upsert(status: u16) -> Result<(), ApiError> {
    match status {
        200..=299 => Ok(()),
        401 => Err(ApiError::Unauthorized),
        s => Err(ApiError::Server { status: s }),
    }
}

// ── Agent config (PATCH /v1/agents/:id) ────────────────────────────────

/// Build the URL for the agent-config PATCH endpoint.
pub fn agent_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}"))
}

/// Build the request body for `PATCH /v1/agents/:id` when only the model
/// is being updated.
pub fn patch_agent_model_body(model: &str) -> String {
    serde_json::json!({ "model": model }).to_string()
}

// ── SSE event parsing (continued) ───────────────────────────────────────

/// Try to convert raw SSE JSON into a typed [`StreamEvent`].
///
/// Returns `None` for unrecognised `message_type` values (the caller can
/// safely ignore them).
pub fn parse_stream_event(v: &serde_json::Value) -> Option<StreamEvent> {
    let mt = v.get("message_type")?.as_str()?;
    match mt {
        "stream_start" => {
            let cid = v.get("conversation_id")?.as_str()?;
            Some(StreamEvent::ConversationId(cid.to_string()))
        }
        "assistant_message" => {
            let text = v.get("content")?.as_str()?;
            Some(StreamEvent::Text(text.to_string()))
        }
        "reasoning_message" => {
            let text = v.get("reasoning")?.as_str()?;
            Some(StreamEvent::Reasoning(text.to_string()))
        }
        "tool_call_message" => {
            let tc = v.get("tool_call")?;
            Some(StreamEvent::ToolCall {
                id: tc.get("id")?.as_str()?.to_string(),
                name: tc.get("name")?.as_str()?.to_string(),
                arguments: tc.get("arguments")?.as_str()?.to_string(),
            })
        }
        "usage_statistics" => Some(StreamEvent::Usage {
            input_tokens: v.get("input_tokens").and_then(|n| n.as_u64()).unwrap_or(0),
            output_tokens: v.get("output_tokens").and_then(|n| n.as_u64()).unwrap_or(0),
            model: v.get("model").and_then(|m| m.as_str()).map(String::from),
        }),
        "finish_reason" => {
            let reason = v.get("reason")?.as_str()?;
            Some(StreamEvent::FinishReason(reason.to_string()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- build_url

    #[test]
    fn build_url_joins_base_and_path() {
        assert_eq!(
            build_url("http://localhost:8284", "/v1/health"),
            "http://localhost:8284/v1/health"
        );
    }

    #[test]
    fn build_url_strips_single_trailing_slash() {
        assert_eq!(
            build_url("http://localhost:8284/", "/v1/health"),
            "http://localhost:8284/v1/health"
        );
    }

    #[test]
    fn build_url_strips_multiple_trailing_slashes() {
        // `trim_end_matches` collapses runs — keeps normalisation predictable.
        assert_eq!(
            build_url("http://x///", "/v1/agents"),
            "http://x/v1/agents"
        );
    }

    // -- bearer_header

    #[test]
    fn bearer_header_formats_bearer_prefix() {
        assert_eq!(bearer_header("abc"), "Bearer abc");
    }

    #[test]
    fn bearer_header_does_not_trim() {
        // Upstream code is responsible for trimming; this helper is literal so
        // the caller cannot accidentally lose the prefix or suffix.
        assert_eq!(bearer_header(" tok "), "Bearer  tok ");
    }

    // -- parse_health (2xx)

    #[test]
    fn parse_health_ok_decodes_server_shape() {
        let body = r#"{"status":"ok","server":"cade-server","version":"0.2.0"}"#;
        let h = parse_health(200, body).expect("decode");
        assert_eq!(h.status, "ok");
        assert_eq!(h.server.as_deref(), Some("cade-server"));
    }

    #[test]
    fn parse_health_accepts_any_2xx() {
        // 204 wouldn't have a body, but 200/201/202 should all decode.
        let body = r#"{"status":"ok"}"#;
        assert!(parse_health(200, body).is_ok());
        assert!(parse_health(202, body).is_ok());
    }

    // -- parse_health (errors)

    #[test]
    fn parse_health_401_returns_unauthorized() {
        let err = parse_health(401, "Unauthorized: missing or invalid API key")
            .expect_err("must error");
        assert_eq!(err, ApiError::Unauthorized);
    }

    #[test]
    fn parse_health_500_returns_server() {
        let err = parse_health(500, r#"{"error":"internal error"}"#).expect_err("must error");
        assert_eq!(err, ApiError::Server { status: 500 });
    }

    #[test]
    fn parse_health_malformed_json_returns_decode() {
        let err = parse_health(200, "not json").expect_err("must error");
        match err {
            ApiError::Decode { .. } => {}
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    // -- parse_agents

    #[test]
    fn parse_agents_ok_decodes_list() {
        let body = r#"[{"id":"a1","name":"A1"},{"id":"a2","name":"A2","model":"gpt-4o"}]"#;
        let agents = parse_agents(200, body).expect("decode");
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].id, "a1");
        assert_eq!(agents[1].model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn parse_agents_empty_list_ok() {
        let agents = parse_agents(200, "[]").expect("decode");
        assert!(agents.is_empty());
    }

    #[test]
    fn parse_agents_401_returns_unauthorized() {
        let err = parse_agents(401, "nope").expect_err("must error");
        assert_eq!(err, ApiError::Unauthorized);
    }

    #[test]
    fn parse_agents_503_returns_server() {
        let err = parse_agents(503, "down").expect_err("must error");
        assert_eq!(err, ApiError::Server { status: 503 });
    }

    // -- Display

    #[test]
    fn api_error_display_is_user_safe() {
        // Never leak stack traces or internal paths — the tdd-guide §3.3
        // rule applies here even though we're on the client side.
        assert_eq!(ApiError::Unauthorized.to_string(), "unauthorized");
        assert_eq!(
            ApiError::Server { status: 500 }.to_string(),
            "server error (status 500)"
        );
    }

    // -- parse_messages

    #[test]
    fn parse_messages_ok_decodes_server_envelope() {
        // Server wraps messages in `{ "messages": [...], "query": "" }`.
        let body = r#"{"messages":[{"id":"m1","role":"user","content":"hello"},{"id":"m2","role":"assistant","content":"hi"}],"query":""}"#;
        let msgs = parse_messages(200, body).expect("decode");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, "m1");
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].id, "m2");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn parse_messages_empty_list() {
        let body = r#"{"messages":[],"query":""}"#;
        let msgs = parse_messages(200, body).expect("decode");
        assert!(msgs.is_empty());
    }

    #[test]
    fn parse_messages_401_returns_unauthorized() {
        let err = parse_messages(401, "nope").expect_err("must error");
        assert_eq!(err, ApiError::Unauthorized);
    }

    #[test]
    fn parse_messages_500_returns_server() {
        let err = parse_messages(500, "err").expect_err("must error");
        assert_eq!(err, ApiError::Server { status: 500 });
    }

    #[test]
    fn parse_messages_malformed_json_returns_decode() {
        let err = parse_messages(200, "not json").expect_err("must error");
        match err {
            ApiError::Decode { .. } => {}
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    // ── parse_stream_event ────────────────────────────────────────────

    #[test]
    fn parse_stream_start() {
        let v = serde_json::json!({"message_type":"stream_start","conversation_id":"c-1","run_id":"r-1"});
        assert_eq!(
            parse_stream_event(&v),
            Some(StreamEvent::ConversationId("c-1".to_string()))
        );
    }

    #[test]
    fn parse_assistant_message() {
        let v = serde_json::json!({"message_type":"assistant_message","content":"hello"});
        assert_eq!(
            parse_stream_event(&v),
            Some(StreamEvent::Text("hello".to_string()))
        );
    }

    #[test]
    fn parse_reasoning_message() {
        let v = serde_json::json!({"message_type":"reasoning_message","reasoning":"hmm"});
        assert_eq!(
            parse_stream_event(&v),
            Some(StreamEvent::Reasoning("hmm".to_string()))
        );
    }

    #[test]
    fn parse_tool_call_message() {
        let v = serde_json::json!({
            "message_type": "tool_call_message",
            "tool_call": {"id": "tc-1", "name": "read_file", "arguments": "{\"path\":\"a.rs\"}"}
        });
        assert_eq!(
            parse_stream_event(&v),
            Some(StreamEvent::ToolCall {
                id: "tc-1".to_string(),
                name: "read_file".to_string(),
                arguments: "{\"path\":\"a.rs\"}".to_string(),
            })
        );
    }

    #[test]
    fn parse_usage_statistics() {
        let v = serde_json::json!({
            "message_type": "usage_statistics",
            "input_tokens": 100,
            "output_tokens": 50,
            "model": "gpt-4o"
        });
        assert_eq!(
            parse_stream_event(&v),
            Some(StreamEvent::Usage {
                input_tokens: 100,
                output_tokens: 50,
                model: Some("gpt-4o".to_string()),
            })
        );
    }

    #[test]
    fn parse_finish_reason() {
        let v = serde_json::json!({"message_type":"finish_reason","reason":"stop"});
        assert_eq!(
            parse_stream_event(&v),
            Some(StreamEvent::FinishReason("stop".to_string()))
        );
    }

    #[test]
    fn parse_unknown_event_returns_none() {
        let v = serde_json::json!({"message_type":"unknown_event"});
        assert_eq!(parse_stream_event(&v), None);
    }

    // -- conversations

    #[test]
    fn parse_conversations_normal() {
        let body = r#"{"conversations":[
            {"id":"c1","title":"First chat","message_count":5,"updated_at":"2025-01-01T00:00:00Z"},
            {"id":"c2","title":"Second chat","message_count":0,"updated_at":"2025-01-02T00:00:00Z"}
        ]}"#;
        let convs = parse_conversations(200, body).unwrap();
        assert_eq!(convs.len(), 2);
        assert_eq!(convs[0].id, "c1");
        assert_eq!(convs[0].title, "First chat");
        assert_eq!(convs[0].message_count, 5);
        assert_eq!(convs[1].id, "c2");
    }

    #[test]
    fn parse_conversations_empty() {
        let body = r#"{"conversations":[]}"#;
        let convs = parse_conversations(200, body).unwrap();
        assert!(convs.is_empty());
    }

    #[test]
    fn parse_conversations_unauthorized() {
        assert_eq!(
            parse_conversations(401, ""),
            Err(ApiError::Unauthorized),
        );
    }

    #[test]
    fn conversations_url_format() {
        assert_eq!(
            conversations_url("http://localhost:8284", "agent-1"),
            "http://localhost:8284/v1/agents/agent-1/conversations"
        );
    }

    // -- memory

    #[test]
    fn memory_url_format() {
        assert_eq!(
            memory_url("http://localhost:8284", "agent-1"),
            "http://localhost:8284/v1/agents/agent-1/memory"
        );
    }

    #[test]
    fn memory_block_url_format() {
        assert_eq!(
            memory_block_url("http://localhost:8284", "agent-1", "human"),
            "http://localhost:8284/v1/agents/agent-1/memory/human"
        );
    }

    #[test]
    fn parse_memory_decodes_blocks() {
        let body = r#"{"blocks":[
            {"label":"human","value":"User loves Rust","description":"User info","tier":"short"},
            {"label":"project","value":"CADE project"}
        ]}"#;
        let blocks = parse_memory(200, body).expect("decode");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].label, "human");
        assert_eq!(blocks[0].value, "User loves Rust");
        assert_eq!(blocks[0].description.as_deref(), Some("User info"));
        assert_eq!(blocks[0].tier.as_deref(), Some("short"));
        assert_eq!(blocks[1].label, "project");
        assert!(blocks[1].description.is_none());
        assert!(blocks[1].tier.is_none());
    }

    #[test]
    fn parse_memory_empty_list_is_ok() {
        let blocks = parse_memory(200, r#"{"blocks":[]}"#).expect("decode");
        assert!(blocks.is_empty());
    }

    #[test]
    fn parse_memory_401_unauthorized() {
        assert_eq!(parse_memory(401, ""), Err(ApiError::Unauthorized));
    }

    #[test]
    fn parse_memory_500_server_error() {
        assert_eq!(
            parse_memory(500, "boom"),
            Err(ApiError::Server { status: 500 })
        );
    }

    #[test]
    fn upsert_memory_body_with_description() {
        let s = upsert_memory_body("hello", Some("desc"));
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["value"], "hello");
        assert_eq!(v["description"], "desc");
    }

    #[test]
    fn upsert_memory_body_without_description() {
        let s = upsert_memory_body("hello", None);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["value"], "hello");
        assert!(v.get("description").is_none());
    }

    #[test]
    fn classify_upsert_accepts_204_and_200() {
        assert!(classify_upsert(200).is_ok());
        assert!(classify_upsert(204).is_ok());
    }

    #[test]
    fn classify_upsert_401_unauthorized() {
        assert_eq!(classify_upsert(401), Err(ApiError::Unauthorized));
    }

    #[test]
    fn classify_upsert_400_is_server() {
        assert_eq!(classify_upsert(400), Err(ApiError::Server { status: 400 }));
    }

    // -- agent config

    #[test]
    fn agent_url_format() {
        assert_eq!(
            agent_url("http://localhost:8284", "agent-1"),
            "http://localhost:8284/v1/agents/agent-1"
        );
    }

    #[test]
    fn patch_agent_model_body_serializes_model() {
        let s = patch_agent_model_body("gpt-4");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["model"], "gpt-4");
    }
}
