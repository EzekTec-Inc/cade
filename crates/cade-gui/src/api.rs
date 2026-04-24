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
    /// The server executed a tool and this is its result (agentic loop only).
    ToolResult {
        id: String,
        name: String,
        output: String,
        is_error: bool,
    },
    /// Token usage statistics for the turn.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        model: Option<String>,
    },
    /// The reason the stream ended (e.g. "stop", "length").
    FinishReason(String),
    /// A dynamic theme update from the server (e.g. via `/theme` slash command).
    ThemeUpdate(cade_core::resources::themes::ThemeColors),
    /// A subagent was spawned and started running.
    SubagentStarted {
        subagent_id: String,
        task: String,
        mode: String,
        model: String,
    },
    /// Periodic progress update from a running subagent.
    SubagentProgress {
        subagent_id: String,
        status: String,
        tool_calls: u32,
        output_lines: u32,
        elapsed_secs: u32,
    },
    /// A subagent finished (success or error).
    SubagentComplete {
        subagent_id: String,
        status: String,
        result_preview: String,
        elapsed_secs: u32,
        is_error: bool,
    },
    /// Server-side notice surfaced to the user as a toast.  Used by
    /// Phase-3 overflow recovery and `/compact` results.
    SystemNotice {
        level: String,
        code: String,
        message: String,
    },
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

/// `POST /v1/agents/:id/compact` URL.
pub fn compact_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}/compact"))
}

/// Build the URL for a single conversation (DELETE).
pub fn conversation_url(server: &str, agent_id: &str, conv_id: &str) -> String {
    build_url(
        server,
        &format!("/v1/agents/{agent_id}/conversations/{conv_id}"),
    )
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
        "tool_result_message" => {
            let tr = v.get("tool_result")?;
            Some(StreamEvent::ToolResult {
                id: tr.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                name: tr.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                output: tr.get("output").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                is_error: tr.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false),
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
        "theme_update" => {
            let t = v.get("theme")?;
            let colors: cade_core::resources::themes::ThemeColors = serde_json::from_value(t.clone()).ok()?;
            Some(StreamEvent::ThemeUpdate(colors))
        }
        "subagent_started" => Some(StreamEvent::SubagentStarted {
            subagent_id: v.get("subagent_id")?.as_str()?.to_string(),
            task: v.get("task").and_then(|t| t.as_str()).unwrap_or("").to_string(),
            mode: v.get("mode").and_then(|m| m.as_str()).unwrap_or("build").to_string(),
            model: v.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string(),
        }),
        "subagent_progress" => Some(StreamEvent::SubagentProgress {
            subagent_id: v.get("subagent_id")?.as_str()?.to_string(),
            status: v.get("status").and_then(|s| s.as_str()).unwrap_or("running").to_string(),
            tool_calls: v.get("tool_calls").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
            output_lines: v.get("output_lines").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
            elapsed_secs: v.get("elapsed_secs").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
        }),
        "subagent_complete" => Some(StreamEvent::SubagentComplete {
            subagent_id: v.get("subagent_id")?.as_str()?.to_string(),
            status: v.get("status").and_then(|s| s.as_str()).unwrap_or("success").to_string(),
            result_preview: v.get("result_preview").and_then(|r| r.as_str()).unwrap_or("").to_string(),
            elapsed_secs: v.get("elapsed_secs").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
            is_error: v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false),
        }),
        "system_notice" => Some(StreamEvent::SystemNotice {
            level: v.get("level").and_then(|s| s.as_str()).unwrap_or("info").to_string(),
            code: v.get("code").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            message: v.get("message").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        }),
        _ => None,
    }
}

// ── Checkpoints ────────────────────────────────────────────────────────

/// A checkpoint row as returned by
/// `GET /v1/agents/:id/checkpoints`.
///
/// `created_at` is a Unix timestamp (seconds).  All optional fields may
/// be `null` on the server side — we default to `None`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct CheckpointRow {
    pub id: String,
    pub agent_id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    pub branch_id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub git_stash_ref: Option<String>,
    #[serde(default)]
    pub git_commit_hash: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
}

/// Parse the response from `GET /v1/agents/:id/checkpoints`.  The
/// server returns a bare JSON array.
pub fn parse_checkpoints(status: u16, body: &str) -> Result<Vec<CheckpointRow>, ApiError> {
    decode_or_error(status, body)
}

/// Build the URL for the checkpoint-collection endpoint.
pub fn checkpoints_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}/checkpoints"))
}

/// Build the URL for a single checkpoint (get/delete).
pub fn checkpoint_url(server: &str, agent_id: &str, cp_id: &str) -> String {
    build_url(
        server,
        &format!("/v1/agents/{agent_id}/checkpoints/{cp_id}"),
    )
}

/// Build the URL for the `POST …/checkpoints/:cp_id/restore` endpoint.
pub fn checkpoint_restore_url(server: &str, agent_id: &str, cp_id: &str) -> String {
    build_url(
        server,
        &format!("/v1/agents/{agent_id}/checkpoints/{cp_id}/restore"),
    )
}

/// Build the body for `POST /v1/agents/:id/checkpoints`.
///
/// All fields are optional; missing fields default server-side (e.g.
/// `branch_id` → `"main"`).
pub fn create_checkpoint_body(
    label: Option<&str>,
    description: Option<&str>,
    conversation_id: Option<&str>,
) -> String {
    let mut map = serde_json::Map::new();
    if let Some(l) = label {
        map.insert("label".into(), serde_json::Value::String(l.into()));
    }
    if let Some(d) = description {
        map.insert("description".into(), serde_json::Value::String(d.into()));
    }
    if let Some(c) = conversation_id {
        map.insert(
            "conversation_id".into(),
            serde_json::Value::String(c.into()),
        );
    }
    serde_json::Value::Object(map).to_string()
}

// ── Artifacts ──────────────────────────────────────────────────────────

/// Summary of an artifact as returned by
/// `GET /v1/agents/:id/artifacts`.  The list endpoint omits `data_text`
/// and `metadata` to keep responses small — fetch by id for detail.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ArtifactInfo {
    pub id: String,
    pub kind: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub created_at: i64,
    #[serde(default)]
    pub run_id: Option<String>,
}

/// Full artifact detail including the text blob (if any).
///
/// `GET /v1/agents/:id/artifacts/:art_id` returns this shape.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ArtifactDetail {
    pub id: String,
    pub kind: String,
    pub content_type: String,
    #[serde(default)]
    pub data_text: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub size_bytes: i64,
    pub created_at: i64,
}

/// Parse the response from `GET /v1/agents/:id/artifacts`.
pub fn parse_artifacts(status: u16, body: &str) -> Result<Vec<ArtifactInfo>, ApiError> {
    decode_or_error(status, body)
}

/// Parse the response from `GET /v1/agents/:id/artifacts/:art_id`.
pub fn parse_artifact(status: u16, body: &str) -> Result<ArtifactDetail, ApiError> {
    decode_or_error(status, body)
}

/// Build the URL for the artifact-collection endpoint.
pub fn artifacts_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}/artifacts"))
}

/// Build the URL for a single artifact (get/delete).
pub fn artifact_url(server: &str, agent_id: &str, art_id: &str) -> String {
    build_url(
        server,
        &format!("/v1/agents/{agent_id}/artifacts/{art_id}"),
    )
}

// ── Agent tools (MCP / skills panel) ──────────────────────────────────

/// A tool entry returned by `GET /v1/agents/:id/tools`.
///
/// Each row represents an MCP tool that has been registered with the
/// agent (id = stable tool uuid, name = MCP tool name as seen by the LLM).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct AgentTool {
    pub id: String,
    pub name: String,
}

/// Parse the response from `GET /v1/agents/:id/tools`.
/// Server returns a bare JSON array `[{"id":"…","name":"…"}, …]`.
pub fn parse_tools(status: u16, body: &str) -> Result<Vec<AgentTool>, ApiError> {
    decode_or_error(status, body)
}

/// Build the URL for the agent-tools endpoint.
pub fn tools_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}/tools"))
}

// ── Question widget types (ask_user_question) ──────────────────────────

/// One selectable option inside an `ask_user_question` tool call.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

/// A parsed `ask_user_question` invocation extracted from a
/// `ToolCall` SSE event.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Question {
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    #[serde(rename = "multiSelect", default)]
    pub multi_select: bool,
}

/// Try to extract a [`Question`] from the raw `arguments` JSON string of a
/// `ToolCall` event whose `name` is `"ask_user_question"`.
///
/// The tool schema sends a `questions` array; we handle both single-item and
/// multi-item arrays but only surface the *first* unanswered question (the
/// TUI follows the same convention).
///
/// Returns `None` when the arguments are unparseable or the tool name is
/// something else.
pub fn parse_ask_question(arguments: &str) -> Option<Question> {
    let v: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let arr = v.get("questions")?.as_array()?;
    let first = arr.first()?;
    serde_json::from_value(first.clone()).ok()
}

// ── Agent metrics ──────────────────────────────────────────────────────

/// Server-side consolidation / compaction metrics for one agent.
/// Mirrors `AgentMetrics` in `cade-server/src/server/state.rs`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, Default)]
pub struct AgentMetrics {
    #[serde(default)]
    pub tool_outputs_compacted: u64,
    #[serde(default)]
    pub consolidation_runs: u64,
    #[serde(default)]
    pub chars_summarised: u64,
    #[serde(default)]
    pub chars_produced: u64,
    #[serde(default)]
    pub inflation_guard_hits: u64,
}

/// Parse `GET /v1/agents/:id/metrics`.
pub fn parse_metrics(status: u16, body: &str) -> Result<AgentMetrics, ApiError> {
    decode_or_error(status, body)
}

/// Build the URL for the metrics endpoint.
pub fn metrics_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}/metrics"))
}

// ── Context-window stats ───────────────────────────────────────────────

/// Context-window statistics returned by `GET /v1/agents/:id/context`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, Default)]
pub struct ContextStats {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub window_tokens: u64,
    #[serde(default)]
    pub turns_total: u64,
    #[serde(default)]
    pub turns_included: u64,
    #[serde(default)]
    pub turns_omitted: u64,
    #[serde(default)]
    pub chars_used: u64,
    #[serde(default)]
    pub message_budget_chars: u64,
    #[serde(default)]
    pub memory_chars: u64,
    #[serde(default)]
    pub system_prompt_chars: u64,
    #[serde(default)]
    pub tool_count: u64,
    #[serde(default)]
    pub tool_schema_reserve_chars: u64,
    #[serde(default)]
    pub needs_consolidation: bool,
}

/// Parse `GET /v1/agents/:id/context`.
pub fn parse_context_stats(status: u16, body: &str) -> Result<ContextStats, ApiError> {
    decode_or_error(status, body)
}

/// Build the URL for the context-stats endpoint.
pub fn context_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}/context"))
}

// ── Context breakdown (per-category) ──────────────────────────────────

/// A single category in the context-window breakdown.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ContextCategory {
    pub name: String,
    pub tokens: u64,
}

/// Per-category context-window breakdown from `GET /v1/agents/:id/context-breakdown`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ContextBreakdown {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub window_tokens: u64,
    #[serde(default)]
    pub pct: u8,
    #[serde(default)]
    pub categories: Vec<ContextCategory>,
}

/// Parse `GET /v1/agents/:id/context-breakdown`.
pub fn parse_context_breakdown(status: u16, body: &str) -> Result<ContextBreakdown, ApiError> {
    decode_or_error(status, body)
}

/// Build the URL for the context-breakdown endpoint.
pub fn context_breakdown_url(server: &str, agent_id: &str) -> String {
    build_url(server, &format!("/v1/agents/{agent_id}/context-breakdown"))
}

// ── Models listing ────────────────────────────────────────────────────

/// A model entry from the `GET /v1/models` response.
///
/// Mirrors `cade_ai::catalogue::ModelEntry` shape but only the fields
/// the GUI needs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ModelInfo {
    pub provider: String,
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub context_window: u32,
}

// ── Provider info ─────────────────────────────────────────────────────

/// Provider info for the providers overlay.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    #[serde(default)]
    pub is_connected: bool,
    #[serde(default)]
    pub model_count: usize,
}

// ── Hook info ─────────────────────────────────────────────────────────

/// Hook info for the hooks overlay.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct HookInfo {
    pub event: String,
    pub command: String,
}

// ── Skill entry ───────────────────────────────────────────────────────

/// Skill summary for the skills overlay.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
pub struct SkillEntry {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub body_chars: usize,
}

/// Envelope for `GET /v1/models`.
#[derive(Debug, serde::Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    dynamic: Vec<ModelInfo>,
    #[serde(default)]
    custom_providers: Vec<String>,
}

/// Parse the response from `GET /v1/models`.
///
/// Returns `(dynamic_models, custom_provider_names)`.
pub fn parse_models(
    status: u16,
    body: &str,
) -> Result<(Vec<ModelInfo>, Vec<String>), ApiError> {
    if status >= 400 {
        return Err(ApiError::Server { status });
    }
    let resp: ModelsResponse =
        serde_json::from_str(body).map_err(|e| ApiError::Decode { message: e.to_string() })?;
    Ok((resp.dynamic, resp.custom_providers))
}

/// Build the URL for the models endpoint.
pub fn models_url(server: &str) -> String {
    build_url(server, "/v1/models")
}

// ── MCP ──────────────────────────────────────────────────────────────────────

/// One MCP server as returned by `GET /v1/mcp`.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct McpServerInfo {
    /// The key used in `settings.toml` / `cade.toml` (e.g. `"desktop-commander"`).
    pub key: String,
    /// The launch command (e.g. `"npx @desktop-commander/mcp-server"`).
    pub command: String,
    /// Prefixed tool names exposed by this server (e.g. `"desktop-commander__bash"`).
    #[serde(default)]
    pub tools: Vec<String>,
    /// Whether the server has been disabled in config.
    #[serde(default)]
    pub disabled: bool,
}

/// Envelope for `GET /v1/mcp`.
#[derive(Debug, serde::Deserialize)]
struct McpResponse {
    #[serde(default)]
    servers: Vec<McpServerInfo>,
}

/// Parse the response from `GET /v1/mcp`.
pub fn parse_mcp_status(status: u16, body: &str) -> Result<Vec<McpServerInfo>, ApiError> {
    if status >= 400 {
        return Err(ApiError::Server { status });
    }
    let resp: McpResponse =
        serde_json::from_str(body).map_err(|e| ApiError::Decode { message: e.to_string() })?;
    Ok(resp.servers)
}

/// Build the URL for the MCP status endpoint.
pub fn mcp_url(server: &str) -> String {
    build_url(server, "/v1/mcp")
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

    #[test]
    fn parse_system_notice_returns_payload() {
        let v = serde_json::json!({
            "message_type": "system_notice",
            "level": "warning",
            "code": "context_overflow_recovering",
            "message": "Context window full — compacting older turns…"
        });
        assert_eq!(
            parse_stream_event(&v),
            Some(StreamEvent::SystemNotice {
                level: "warning".to_string(),
                code: "context_overflow_recovering".to_string(),
                message: "Context window full — compacting older turns…".to_string(),
            })
        );
    }

    #[test]
    fn parse_system_notice_defaults_when_fields_missing() {
        // Only a message_type — defaults must populate.
        let v = serde_json::json!({"message_type": "system_notice"});
        assert_eq!(
            parse_stream_event(&v),
            Some(StreamEvent::SystemNotice {
                level: "info".to_string(),
                code: String::new(),
                message: String::new(),
            })
        );
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

    #[test]
    fn conversation_url_format() {
        assert_eq!(
            conversation_url("http://localhost:8284", "agent-1", "conv-42"),
            "http://localhost:8284/v1/agents/agent-1/conversations/conv-42"
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

    // -- checkpoints

    #[test]
    fn checkpoints_url_format() {
        assert_eq!(
            checkpoints_url("http://localhost:8284", "agent-1"),
            "http://localhost:8284/v1/agents/agent-1/checkpoints"
        );
    }

    #[test]
    fn checkpoint_url_format() {
        assert_eq!(
            checkpoint_url("http://localhost:8284", "agent-1", "cp-abc"),
            "http://localhost:8284/v1/agents/agent-1/checkpoints/cp-abc"
        );
    }

    #[test]
    fn checkpoint_restore_url_format() {
        assert_eq!(
            checkpoint_restore_url("http://localhost:8284", "agent-1", "cp-abc"),
            "http://localhost:8284/v1/agents/agent-1/checkpoints/cp-abc/restore"
        );
    }

    #[test]
    fn parse_checkpoints_decodes_rows() {
        let body = r#"[
            {"id":"cp-1","agent_id":"agent-1","branch_id":"main","created_at":1700000000,
             "label":"before-refactor","description":"safe",
             "conversation_id":null,"git_stash_ref":"stash@{0}","git_commit_hash":null,"parent_id":null},
            {"id":"cp-2","agent_id":"agent-1","branch_id":"main","created_at":1700001000,
             "label":null,"description":null,
             "conversation_id":null,"git_stash_ref":null,"git_commit_hash":null,"parent_id":null}
        ]"#;
        let rows = parse_checkpoints(200, body).expect("decode");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "cp-1");
        assert_eq!(rows[0].label.as_deref(), Some("before-refactor"));
        assert_eq!(rows[0].git_stash_ref.as_deref(), Some("stash@{0}"));
        assert_eq!(rows[1].label, None);
    }

    #[test]
    fn parse_checkpoints_empty_is_ok() {
        let rows = parse_checkpoints(200, "[]").expect("decode");
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_checkpoints_401_unauthorized() {
        assert_eq!(parse_checkpoints(401, ""), Err(ApiError::Unauthorized));
    }

    #[test]
    fn parse_checkpoints_500_server_error() {
        assert_eq!(
            parse_checkpoints(500, "boom"),
            Err(ApiError::Server { status: 500 })
        );
    }

    #[test]
    fn parse_checkpoints_tolerates_missing_optional_fields() {
        // Server may omit default-null fields.
        let body = r#"[{"id":"cp-1","agent_id":"a","branch_id":"main","created_at":1}]"#;
        let rows = parse_checkpoints(200, body).expect("decode");
        assert_eq!(rows[0].label, None);
        assert_eq!(rows[0].parent_id, None);
    }

    #[test]
    fn create_checkpoint_body_all_fields() {
        let s = create_checkpoint_body(Some("label"), Some("desc"), Some("conv-1"));
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["label"], "label");
        assert_eq!(v["description"], "desc");
        assert_eq!(v["conversation_id"], "conv-1");
    }

    #[test]
    fn create_checkpoint_body_no_fields() {
        let s = create_checkpoint_body(None, None, None);
        assert_eq!(s, "{}");
    }

    #[test]
    fn create_checkpoint_body_partial_fields() {
        let s = create_checkpoint_body(Some("just-label"), None, None);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["label"], "just-label");
        assert!(v.get("description").is_none());
        assert!(v.get("conversation_id").is_none());
    }

    // -- artifacts

    #[test]
    fn artifacts_url_format() {
        assert_eq!(
            artifacts_url("http://localhost:8284", "agent-1"),
            "http://localhost:8284/v1/agents/agent-1/artifacts"
        );
    }

    #[test]
    fn artifact_url_format() {
        assert_eq!(
            artifact_url("http://localhost:8284", "agent-1", "art-xyz"),
            "http://localhost:8284/v1/agents/agent-1/artifacts/art-xyz"
        );
    }

    #[test]
    fn parse_artifacts_decodes_list() {
        let body = r#"[
            {"id":"art-1","kind":"log","content_type":"text/plain",
             "size_bytes":42,"created_at":1700000000,"run_id":"run-1"},
            {"id":"art-2","kind":"diff","content_type":"text/x-diff",
             "size_bytes":128,"created_at":1700001000,"run_id":null}
        ]"#;
        let rows = parse_artifacts(200, body).expect("decode");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "log");
        assert_eq!(rows[0].size_bytes, 42);
        assert_eq!(rows[0].run_id.as_deref(), Some("run-1"));
        assert_eq!(rows[1].run_id, None);
    }

    #[test]
    fn parse_artifacts_empty_is_ok() {
        let rows = parse_artifacts(200, "[]").expect("decode");
        assert!(rows.is_empty());
    }

    #[test]
    fn parse_artifacts_401_unauthorized() {
        assert_eq!(parse_artifacts(401, ""), Err(ApiError::Unauthorized));
    }

    #[test]
    fn parse_artifact_decodes_detail_with_text() {
        let body = r#"{"id":"art-1","kind":"log","content_type":"text/plain",
                       "data_text":"hello world","metadata":{"k":"v"},
                       "size_bytes":11,"created_at":1700000000}"#;
        let d = parse_artifact(200, body).expect("decode");
        assert_eq!(d.id, "art-1");
        assert_eq!(d.data_text.as_deref(), Some("hello world"));
        assert_eq!(d.metadata["k"], "v");
    }

    #[test]
    fn parse_artifact_tolerates_null_data_text() {
        let body = r#"{"id":"art-1","kind":"pdf","content_type":"application/pdf",
                       "data_text":null,"metadata":{},"size_bytes":0,"created_at":1}"#;
        let d = parse_artifact(200, body).expect("decode");
        assert_eq!(d.data_text, None);
    }

    #[test]
    fn parse_artifact_404_server_error() {
        assert_eq!(
            parse_artifact(404, ""),
            Err(ApiError::Server { status: 404 })
        );
    }

    // -- tools

    #[test]
    fn tools_url_format() {
        assert_eq!(
            tools_url("http://localhost:8284", "agent-1"),
            "http://localhost:8284/v1/agents/agent-1/tools"
        );
    }

    #[test]
    fn parse_tools_decodes_list() {
        let body = r#"[{"id":"t1","name":"read_file"},{"id":"t2","name":"bash"}]"#;
        let tools = parse_tools(200, body).expect("decode");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "read_file");
        assert_eq!(tools[1].id, "t2");
    }

    #[test]
    fn parse_tools_empty_is_ok() {
        assert!(parse_tools(200, "[]").expect("decode").is_empty());
    }

    #[test]
    fn parse_tools_401_unauthorized() {
        assert_eq!(parse_tools(401, ""), Err(ApiError::Unauthorized));
    }

    // -- ask_user_question

    #[test]
    fn parse_ask_question_single_question() {
        let args = r#"{"questions":[{
            "header":"Auth",
            "question":"Which method?",
            "options":[
                {"label":"JWT","description":"Stateless"},
                {"label":"Sessions","description":"Stateful"}
            ],
            "multiSelect":false
        }]}"#;
        let q = parse_ask_question(args).expect("parse");
        assert_eq!(q.header, "Auth");
        assert_eq!(q.question, "Which method?");
        assert_eq!(q.options.len(), 2);
        assert_eq!(q.options[0].label, "JWT");
        assert!(!q.multi_select);
    }

    #[test]
    fn parse_ask_question_multi_select() {
        let args = r#"{"questions":[{
            "header":"Choose",
            "question":"Pick any",
            "options":[{"label":"A","description":""},{"label":"B","description":""}],
            "multiSelect":true
        }]}"#;
        let q = parse_ask_question(args).expect("parse");
        assert!(q.multi_select);
    }

    #[test]
    fn parse_ask_question_returns_first_only() {
        let args = r#"{"questions":[
            {"header":"Q1","question":"First?","options":[{"label":"Yes","description":""}],"multiSelect":false},
            {"header":"Q2","question":"Second?","options":[{"label":"No","description":""}],"multiSelect":false}
        ]}"#;
        let q = parse_ask_question(args).expect("parse");
        assert_eq!(q.header, "Q1");
    }

    #[test]
    fn parse_ask_question_none_on_invalid_json() {
        assert!(parse_ask_question("not json").is_none());
    }

    #[test]
    fn parse_ask_question_none_on_missing_questions_key() {
        assert!(parse_ask_question(r#"{"other":"value"}"#).is_none());
    }

    #[test]
    fn parse_ask_question_none_on_empty_array() {
        assert!(parse_ask_question(r#"{"questions":[]}"#).is_none());
    }

    #[test]
    fn parse_ask_question_description_defaults_to_empty() {
        let args = r#"{"questions":[{
            "header":"H","question":"Q?",
            "options":[{"label":"Only"}]
        }]}"#;
        let q = parse_ask_question(args).expect("parse");
        assert_eq!(q.options[0].description, "");
    }

    // -- metrics

    #[test]
    fn metrics_url_format() {
        assert_eq!(
            metrics_url("http://localhost:8284", "agent-1"),
            "http://localhost:8284/v1/agents/agent-1/metrics"
        );
    }

    #[test]
    fn parse_metrics_decodes_all_fields() {
        let body = r#"{"tool_outputs_compacted":3,"consolidation_runs":2,
                       "chars_summarised":1000,"chars_produced":800,
                       "inflation_guard_hits":1}"#;
        let m = parse_metrics(200, body).expect("decode");
        assert_eq!(m.tool_outputs_compacted, 3);
        assert_eq!(m.consolidation_runs, 2);
        assert_eq!(m.chars_summarised, 1000);
        assert_eq!(m.chars_produced, 800);
        assert_eq!(m.inflation_guard_hits, 1);
    }

    #[test]
    fn parse_metrics_defaults_missing_fields() {
        let m = parse_metrics(200, "{}").expect("decode");
        assert_eq!(m.consolidation_runs, 0);
    }

    #[test]
    fn parse_metrics_401_unauthorized() {
        assert_eq!(parse_metrics(401, ""), Err(ApiError::Unauthorized));
    }

    // -- context stats

    #[test]
    fn context_url_format() {
        assert_eq!(
            context_url("http://localhost:8284", "agent-1"),
            "http://localhost:8284/v1/agents/agent-1/context"
        );
    }

    #[test]
    fn parse_context_stats_decodes_shape() {
        let body = r#"{"model":"gpt-4o","window_tokens":128000,
                       "turns_total":10,"turns_included":8,"turns_omitted":2,
                       "chars_used":5000,"message_budget_chars":6000,
                       "memory_chars":200,"system_prompt_chars":100,
                       "tool_count":3,"tool_schema_reserve_chars":300,
                       "needs_consolidation":false}"#;
        let s = parse_context_stats(200, body).expect("decode");
        assert_eq!(s.model.as_deref(), Some("gpt-4o"));
        assert_eq!(s.window_tokens, 128000);
        assert_eq!(s.turns_included, 8);
        assert!(!s.needs_consolidation);
    }

    #[test]
    fn parse_context_stats_tolerates_missing_fields() {
        let s = parse_context_stats(200, "{}").expect("decode");
        assert_eq!(s.model, None);
        assert_eq!(s.window_tokens, 0);
    }

    #[test]
    fn parse_context_stats_500_server_error() {
        assert_eq!(
            parse_context_stats(500, ""),
            Err(ApiError::Server { status: 500 })
        );
    }

    // ── Models ────────────────────────────────────────────────────────

    #[test]
    fn parse_models_decodes_dynamic_list() {
        let body = r#"{
            "supported": [],
            "dynamic": [
                {"provider":"anthropic","id":"claude-3-5-sonnet","display_name":"Claude 3.5 Sonnet","context_window":200000},
                {"provider":"openai","id":"gpt-4o","display_name":"GPT-4o","context_window":128000}
            ],
            "custom_providers": ["my-local"]
        }"#;
        let (models, custom) = parse_models(200, body).expect("decode");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "claude-3-5-sonnet");
        assert_eq!(models[0].provider, "anthropic");
        assert_eq!(models[1].context_window, 128000);
        assert_eq!(custom, vec!["my-local"]);
    }

    #[test]
    fn parse_models_empty_response() {
        let (models, custom) = parse_models(200, r#"{"supported":[],"dynamic":[],"custom_providers":[]}"#).expect("decode");
        assert!(models.is_empty());
        assert!(custom.is_empty());
    }

    #[test]
    fn parse_models_500_error() {
        assert_eq!(
            parse_models(500, ""),
            Err(ApiError::Server { status: 500 })
        );
    }

    #[test]
    fn models_url_builds_correctly() {
        let u = models_url("http://localhost:3000");
        assert_eq!(u, "http://localhost:3000/v1/models");
    }

    // -- MCP

    #[test]
    fn mcp_url_builds_correctly() {
        assert_eq!(mcp_url("http://localhost:8284"), "http://localhost:8284/v1/mcp");
    }

    #[test]
    fn parse_mcp_status_empty() {
        let servers = parse_mcp_status(200, r#"{"servers":[]}"#).expect("decode");
        assert!(servers.is_empty());
    }

    #[test]
    fn parse_mcp_status_one_server() {
        let body = r#"{
            "servers": [{
                "key": "desktop-commander",
                "command": "npx @desktop-commander/mcp-server",
                "tools": ["desktop-commander__bash", "desktop-commander__read_file"],
                "disabled": false
            }]
        }"#;
        let servers = parse_mcp_status(200, body).expect("decode");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].key, "desktop-commander");
        assert_eq!(servers[0].tools.len(), 2);
        assert!(!servers[0].disabled);
    }

    #[test]
    fn parse_mcp_status_disabled_server() {
        let body = r#"{"servers":[{"key":"old","command":"old-cmd","tools":[],"disabled":true}]}"#;
        let s = parse_mcp_status(200, body).expect("decode");
        assert!(s[0].disabled);
    }

    #[test]
    fn parse_mcp_status_500_error() {
        assert_eq!(
            parse_mcp_status(500, ""),
            Err(ApiError::Server { status: 500 })
        );
    }
}
