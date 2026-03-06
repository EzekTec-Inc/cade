use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response, Sse},
    Json,
};
use axum::response::sse::Event;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::server::{
    llm::{CompletionRequest, LlmMessage, LlmToolCall, StreamChunk, TokenUsage},
    state::AppState,
    storage::sqlite::{self, MessageRow},
};

/// Maximum length for auto-generated conversation titles (chars from first user message).
const CONV_TITLE_MAX: usize = 60;
/// Number of DB message rows to load per turn (~100 full tool-call cycles at 200 rows).
const HISTORY_LIMIT: usize = 100;
/// Hard cap on all memory blocks combined in the system prompt (~2k tokens).
/// Blocks are prioritised by recency (most recently updated first).
/// Blocks whose value is empty are always skipped regardless of this budget.
const MEMORY_CHAR_BUDGET: usize = 8_000;
/// Cap on a single tool-result content string (chars). ~8k tokens.
/// Prevents huge outputs (screenshots, logs) from blowing the context window.
const TOOL_RESULT_MAX_CHARS: usize = 8_192;
/// Chars-per-token approximation used to convert a model's token context window
/// into a character budget. 3 chars ≈ 1 token is conservative across English,
/// code, and mixed content; keeps a ~25% headroom below the hard token limit.
const CHARS_PER_TOKEN: usize = 3;
/// Minimum character budget regardless of model window (guards tiny local models).
const MIN_CONTEXT_CHARS: usize = 8_000;
/// Maximum character budget cap — avoids enormous payloads on multi-million
/// token windows (e.g. Gemini 1.5 Pro at 2 M tokens).
const MAX_CONTEXT_CHARS: usize = 600_000;

// ── Message history sanitizer ─────────────────────────────────────────────────
//
// Anthropic enforces a strict schema:
//   1. Every tool_use in an assistant message must have exactly ONE matching
//      tool_result in the very next user message.
//   2. No tool_result may appear without a preceding tool_use.
//   3. No duplicate tool_result IDs in the same user message.
//
// Previous bugs (SSE reconnect, interrupted sessions) can leave the DB in a
// state that violates these rules.  sanitize_messages() repairs the history
// before it is sent to the LLM so the server never crashes due to DB
// corruption.

fn sanitize_messages(messages: Vec<LlmMessage>) -> Vec<LlmMessage> {
    let mut result: Vec<LlmMessage> = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        let msg = messages[i].clone();

        match msg.role.as_str() {
            "assistant"
                if msg.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty()) =>
            {
                let tool_calls = msg.tool_calls.as_ref().unwrap();
                let expected_ids: Vec<String> =
                    tool_calls.iter().map(|tc| tc.id.clone()).collect();

                // Consume ALL immediately-following tool rows (may be duplicated/partial)
                let mut j = i + 1;
                // id → first content seen (dedup)
                let mut found: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                while j < messages.len() && messages[j].role == "tool" {
                    if let Some(id) = &messages[j].tool_call_id {
                        found.entry(id.clone())
                            .or_insert_with(|| messages[j].content.clone());
                    }
                    j += 1;
                }

                result.push(msg);

                // Emit exactly one tool_result per expected id (synthetic if missing)
                for id in &expected_ids {
                    let content = found
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| "[Tool execution was interrupted]".to_string());
                    result.push(LlmMessage {
                        role: "tool".to_string(),
                        content,
                        tool_call_id: Some(id.clone()),
                        tool_calls: None,
                    });
                }

                i = j; // skip the (possibly messy) original tool rows
            }

            "tool" => {
                // Orphaned tool_result — no preceding assistant with a matching tool_use.
                // Drop it; it would make Anthropic return 400.
                tracing::warn!(
                    "Dropping orphaned tool_result (id={:?})",
                    msg.tool_call_id
                );
                i += 1;
            }

            _ => {
                result.push(msg);
                i += 1;
            }
        }
    }

    result
}

// ── Context builder ───────────────────────────────────────────────────────────
//
// Key design rule:
//   Callers PERSIST a message to SQLite BEFORE calling build_context.
//   build_context loads everything from SQLite — no new_message parameter.
//   This prevents the double-message bug that breaks tool_use/tool_result ordering.

async fn build_context(
    state: &AppState,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> Result<(String, Vec<LlmMessage>, Vec<Value>), String> {
    let agent = sqlite::get_agent(&state.db, agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;

    // Memory blocks → appended to system prompt.
    // Rules:
    //   1. Skip empty-value blocks (nothing useful to inject).
    //   2. Sort by updated_at DESC so most-recently-written blocks have priority.
    //   3. Greedily include blocks until MEMORY_CHAR_BUDGET is reached; append a
    //      notice when blocks are omitted so the agent knows to use /memory.
    let raw_blocks = sqlite::get_memory_blocks_with_ts(&state.db, agent_id)
        .unwrap_or_default();
    let mut budget_remaining = MEMORY_CHAR_BUDGET;
    let mut included_blocks: Vec<String> = Vec::new();
    let mut omitted_count = 0usize;
    for (label, val, _desc, _ts) in &raw_blocks {
        if val.trim().is_empty() { continue; }  // S6: skip empty blocks
        let entry = format!("[{label}]\n{val}");
        let entry_chars = entry.chars().count();
        if entry_chars <= budget_remaining {
            budget_remaining -= entry_chars;
            included_blocks.push(entry);
        } else {
            omitted_count += 1;
        }
    }
    let memory = if omitted_count > 0 {
        let mut parts = included_blocks;
        parts.push(format!(
            "[…{omitted_count} block(s) omitted — memory budget reached. Use /memory to manage.]"
        ));
        parts.join("\n\n")
    } else {
        included_blocks.join("\n\n")
    };

    let base = agent.system_prompt.clone().unwrap_or_default();
    let system_prompt = if memory.is_empty() {
        base
    } else {
        format!("{base}\n\n# Memory\n{memory}")
    };

    // Message history from DB — oldest first, scoped to conversation
    let history = sqlite::list_messages(&state.db, agent_id, conversation_id, HISTORY_LIMIT)
        .unwrap_or_default();

    let mut messages: Vec<LlmMessage> = vec![LlmMessage {
        role: "system".to_string(),
        content: system_prompt,
        tool_call_id: None,
        tool_calls: None,
    }];

    for row in &history {
        messages.extend(db_row_to_llm(row));
    }

    // Sanitize history: fix orphaned tool_calls, dedup tool_results, drop
    // stray tool_results so Anthropic never sees an invalid sequence.
    if messages.len() > 1 {
        let system_msg = messages.remove(0);
        let sanitized = sanitize_messages(messages);
        messages = std::iter::once(system_msg).chain(sanitized).collect();
    }

    // Character-budget trimming: drop oldest non-system messages until total
    // content fits within the model's context window.
    // Always keeps the system prompt and the last user+assistant turn (≥3 msgs).
    // Count chars (codepoints), not bytes — budget is a char budget.
    let context_char_budget = {
        let window_tokens = crate::server::llm::catalogue::context_window_for_model(&agent.model);
        let raw = (window_tokens as usize).saturating_mul(CHARS_PER_TOKEN);
        raw.clamp(MIN_CONTEXT_CHARS, MAX_CONTEXT_CHARS)
    };
    tracing::debug!(
        "Context budget for model '{}': {} chars ({} tokens * {})",
        agent.model, context_char_budget,
        crate::server::llm::catalogue::context_window_for_model(&agent.model),
        CHARS_PER_TOKEN
    );
    let total_chars = |msgs: &[LlmMessage]| -> usize {
        msgs.iter().map(|m| m.content.chars().count()).sum()
    };
    while total_chars(&messages) > context_char_budget && messages.len() > 3 {
        // messages[0] is always the system prompt — remove messages[1]
        messages.remove(1);
    }

    // Tool schemas — use agent-specific tools if wired, else all tools
    let agent_tool_ids = sqlite::get_agent_tool_ids(&state.db, agent_id)
        .unwrap_or_default();
    let all_tools = sqlite::list_tools(&state.db).unwrap_or_default();
    let tool_schemas: Vec<Value> = if agent_tool_ids.is_empty() {
        // Not yet wired → provide all registered tools (backwards-compatible)
        all_tools.into_iter().filter_map(|t| t.json_schema).collect()
    } else {
        all_tools.into_iter()
            .filter(|t| agent_tool_ids.contains(&t.id))
            .filter_map(|t| t.json_schema)
            .collect()
    };

    Ok((agent.model, messages, tool_schemas))
}

/// Convert a DB MessageRow to one or more LlmMessages.
fn db_row_to_llm(row: &MessageRow) -> Vec<LlmMessage> {
    match row.role.as_str() {
        "tool" => {
            let raw = row.content["content"].as_str().unwrap_or("");
            // Truncate very large tool results (e.g. raw base64 images, enormous logs)
            // to prevent context window overflows.
            // Truncate at a char boundary, not a byte boundary.
            // TOOL_RESULT_MAX_CHARS is a *char* limit; raw.len() is bytes.
            // Slicing `&raw[..N]` at a bare byte index panics when a multibyte
            // codepoint (e.g. '─' = 3 bytes: E2 94 80) straddles position N.
            let content = if raw.len() > TOOL_RESULT_MAX_CHARS {
                // Find the byte offset of the TOOL_RESULT_MAX_CHARS-th char.
                let byte_end = raw
                    .char_indices()
                    .nth(TOOL_RESULT_MAX_CHARS)
                    .map(|(i, _)| i)
                    .unwrap_or(raw.len()); // fewer chars than the limit → keep all
                if byte_end < raw.len() {
                    format!(
                        "{}\n[... truncated: {} bytes total, showing first {} chars]",
                        &raw[..byte_end],
                        raw.len(),
                        TOOL_RESULT_MAX_CHARS,
                    )
                } else {
                    raw.to_string()
                }
            } else {
                raw.to_string()
            };
            vec![LlmMessage {
                role: "tool".to_string(),
                content,
                tool_call_id: row.content["tool_call_id"].as_str().map(String::from),
                tool_calls: None,
            }]
        }
        "assistant" => {
            // A single DB row may have both text content and tool_calls
            let text = row.content["content"].as_str().unwrap_or("").to_string();
            let tool_calls: Option<Vec<LlmToolCall>> =
                row.content["tool_calls"]
                    .as_array()
                    .filter(|arr| !arr.is_empty())  // treat [] same as absent
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|tc| serde_json::from_value(tc.clone()).ok())
                            .collect()
                    });
            vec![LlmMessage {
                role: "assistant".to_string(),
                content: text,
                tool_call_id: None,
                tool_calls,
            }]
        }
        _ => vec![LlmMessage {
            role: row.role.clone(),
            content: row.content["content"].as_str().unwrap_or("").to_string(),
            tool_call_id: None,
            tool_calls: None,
        }],
    }
}

fn new_msg_id() -> String {
    format!("msg-{}", Uuid::new_v4())
}

fn persist(state: &AppState, agent_id: &str, conversation_id: Option<&str>, role: &str, content: Value) {
    let row = MessageRow {
        id:              new_msg_id(),
        agent_id:        agent_id.to_string(),
        conversation_id: conversation_id.map(String::from),
        role:            role.to_string(),
        content,
    };
    let _ = sqlite::insert_message(&state.db, &row);
    // Touch the conversation's updated_at so list order stays current
    if let Some(conv_id) = conversation_id {
        let _ = sqlite::touch_conversation(&state.db, conv_id);
    }
}

/// Extract and validate conversation_id from request body.
/// If present and non-empty, verifies it exists in the DB.
/// Returns Ok(Some(id)) | Ok(None) | Err(response).
fn resolve_conversation<'a>(
    state: &AppState,
    agent_id: &str,
    body: &'a Value,
) -> Result<Option<String>, axum::response::Response> {
    let conv_id = body["conversation_id"].as_str().filter(|s| !s.is_empty());
    match conv_id {
        None => Ok(None),
        Some(id) => {
            match sqlite::get_conversation(&state.db, id) {
                Ok(Some(_)) => Ok(Some(id.to_string())),
                Ok(None) => Err(err(
                    axum::http::StatusCode::NOT_FOUND,
                    &format!("conversation '{id}' not found for agent '{agent_id}'"),
                )),
                Err(e) => Err(err(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())),
            }
        }
    }
}

// ── POST /v1/agents/:id/messages  (blocking) ─────────────────────────────────

pub async fn send_message(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let conv_id = match resolve_conversation(&state, &agent_id, &body) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let conv_id_ref = conv_id.as_deref();

    if body["role"].as_str() == Some("tool") {
        return handle_tool_return_blocking(&state, &agent_id, conv_id_ref, &body).await;
    }

    let input = match body["input"].as_str().filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "missing 'input'"),
    };

    // 1. Persist user message FIRST
    persist(&state, &agent_id, conv_id_ref, "user", json!({ "content": input }));

    // 2. Build context from DB (includes the message we just persisted)
    let (model, messages, tools) = match build_context(&state, &agent_id, conv_id_ref).await {
        Ok(ctx) => ctx,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };

    // 3. Call LLM
    let max_tokens = crate::server::llm::catalogue::max_tokens_for_model(&model);
    let req = CompletionRequest { model, messages, tools, max_tokens };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            let tool_calls_json: Vec<Value> = resp.tool_calls.iter().map(|tc| json!({
                "id": tc.id, "name": tc.name, "arguments": tc.arguments
            })).collect();
            persist(&state, &agent_id, conv_id_ref, "assistant", json!({
                "content": resp.content.clone().unwrap_or_default(),
                "tool_calls": tool_calls_json
            }));

            let mut out: Vec<Value> = vec![];
            if let Some(text) = &resp.content {
                out.push(json!({ "message_type": "assistant_message", "content": text }));
            }
            for tc in &resp.tool_calls {
                out.push(json!({
                    "message_type": "tool_call_message",
                    "tool_call": { "id": tc.id, "name": tc.name, "arguments": tc.arguments }
                }));
            }
            Json(json!({ "messages": out, "conversation_id": conv_id })).into_response()
        }
        Err(e) => {
            tracing::error!("LLM error: {e}");
            err(StatusCode::BAD_GATEWAY, &e.to_string())
        }
    }
}

async fn handle_tool_return_blocking(
    state: &AppState,
    agent_id: &str,
    conv_id: Option<&str>,
    body: &Value,
) -> Response {
    let tr = &body["tool_return"];
    let call_id = tr["tool_call_id"].as_str().unwrap_or("").to_string();
    let content  = tr["content"].as_str().unwrap_or("").to_string();

    persist(state, agent_id, conv_id, "tool", json!({
        "content": content, "tool_call_id": call_id
    }));

    match sqlite::pending_tool_results(&state.db, agent_id, conv_id) {
        Ok((received, expected)) if received < expected => {
            tracing::debug!("Tool results: {received}/{expected} — waiting");
            return Json(json!({ "messages": [], "conversation_id": conv_id })).into_response();
        }
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        _ => {}
    }

    let (model, messages, tools) = match build_context(state, agent_id, conv_id).await {
        Ok(ctx) => ctx,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };

    let max_tokens = crate::server::llm::catalogue::max_tokens_for_model(&model);
    let req = CompletionRequest { model, messages, tools, max_tokens };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            let tool_calls_json: Vec<Value> = resp.tool_calls.iter().map(|tc| json!({
                "id": tc.id, "name": tc.name, "arguments": tc.arguments
            })).collect();
            persist(state, agent_id, conv_id, "assistant", json!({
                "content": resp.content.clone().unwrap_or_default(),
                "tool_calls": tool_calls_json
            }));
            let mut out: Vec<Value> = vec![];
            if let Some(text) = &resp.content {
                out.push(json!({ "message_type": "assistant_message", "content": text }));
            }
            for tc in &resp.tool_calls {
                out.push(json!({
                    "message_type": "tool_call_message",
                    "tool_call": { "id": tc.id, "name": tc.name, "arguments": tc.arguments }
                }));
            }
            Json(json!({ "messages": out, "conversation_id": conv_id })).into_response()
        }
        Err(e) => err(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

// ── POST /v1/agents/:id/messages/stream  (SSE) ───────────────────────────────

pub async fn stream_message(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let conv_id: Option<String> = match resolve_conversation(&state, &agent_id, &body) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let conv_str = conv_id.clone();
    let conv_id_ref = conv_str.as_deref();

    let is_tool_return = body["role"].as_str() == Some("tool");

    // 1. Persist incoming message FIRST
    if is_tool_return {
        let tr = &body["tool_return"];
        persist(&state, &agent_id, conv_id_ref, "tool", json!({
            "content": tr["content"].as_str().unwrap_or(""),
            "tool_call_id": tr["tool_call_id"].as_str().unwrap_or("")
        }));
    } else {
        let input = match body["input"].as_str().filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => return err(StatusCode::BAD_REQUEST, "missing 'input'"),
        };
        // Auto-title new conversations from the first user message
        if let Some(cid) = conv_id_ref {
            let _ = maybe_set_conv_title(&state, cid, &input);
        }
        persist(&state, &agent_id, conv_id_ref, "user", json!({ "content": input }));
    }

    // 2. If this was a tool return, check if all results for this turn have arrived.
    if is_tool_return {
        match sqlite::pending_tool_results(&state.db, &agent_id, conv_id_ref) {
            Ok((received, expected)) if received < expected => {
                tracing::debug!("Stream: tool results {received}/{expected} — waiting");
                let s = futures::stream::once(async {
                    Ok::<Event, std::convert::Infallible>(Event::default().data("[DONE]"))
                });
                return Sse::new(s).into_response();
            }
            Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            _ => {}
        }
    }

    // 3. Build context from DB
    let (model, messages, tools) = match build_context(&state, &agent_id, conv_id_ref).await {
        Ok(ctx) => ctx,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };

    let background = body["background"].as_bool().unwrap_or(false);
    // Create a run for background (and also for foreground — keeps history for reconnect)
    let run = sqlite::create_run(&state.db, &agent_id, conv_id_ref);
    let run_id: Option<String> = run.ok().map(|r| r.id);

    let max_tokens = crate::server::llm::catalogue::max_tokens_for_model(&model);
    let req = CompletionRequest { model, messages, tools, max_tokens };
    let state_clone = state.clone();
    let agent_id_clone = agent_id.clone();
    let conv_id_clone = conv_str.clone();
    let run_id_clone  = run_id.clone();
    let db_clone      = state.db.clone();

    // Open LLM stream.
    // On failure, return a well-formed SSE stream with an error event + [DONE]
    // instead of a raw HTTP 502. This prevents reqwest_eventsource from
    // triggering the client's SSE fallback (which would re-persist the user
    // message and call the blocking endpoint — duplicating DB entries).
    let llm_stream = match state.llm.stream(&req).await {
        Ok(s) => s,
        Err(e) => {
            let err_msg = e.to_string();
            tracing::error!("LLM stream open failed: {err_msg}");
            if let Some(rid) = &run_id {
                let _ = sqlite::finish_run(&state.db, rid, "failed");
            }
            let s = futures::stream::iter([
                Ok::<Event, std::convert::Infallible>(
                    Event::default().data(json!({ "error": err_msg }).to_string())
                ),
                Ok::<Event, std::convert::Infallible>(
                    Event::default().data("[DONE]")
                ),
            ]);
            return Sse::new(s).into_response();
        }
    };

    let acc = std::sync::Arc::new(std::sync::Mutex::new((String::new(), Vec::<Value>::new())));
    let acc_clone = acc.clone();
    // Accumulate token usage across chunks
    let usage_acc = std::sync::Arc::new(std::sync::Mutex::new(TokenUsage::default()));
    let usage_acc2 = usage_acc.clone();

    // First SSE event: metadata (conversation_id + run_id)
    let meta_event = {
        let data = json!({
            "message_type": "stream_start",
            "conversation_id": conv_str,
            "run_id": run_id,
        });
        futures::stream::once(async move {
            Ok::<Event, std::convert::Infallible>(Event::default().data(data.to_string()))
        })
    };

    let sse_stream = futures::StreamExt::map(llm_stream, move |chunk: Result<StreamChunk>| {
        // Persist each event to run_events so the stream is resumable
        let emit = |data: Value| -> Event {
            if let Some(rid) = &run_id_clone {
                if let Ok(seq) = sqlite::append_run_event(&db_clone, rid, &data.to_string()) {
                    let mut d = data.clone();
                    if let Some(obj) = d.as_object_mut() {
                        obj.insert("run_id".to_string(),  serde_json::Value::String(rid.clone()));
                        obj.insert("seq_id".to_string(),  serde_json::Value::Number(seq.into()));
                    }
                    return Event::default().data(d.to_string());
                }
            }
            Event::default().data(data.to_string())
        };

        let event = match chunk {
            Ok(StreamChunk::Text(text)) => {
                if let Ok(mut g) = acc_clone.lock() { g.0.push_str(&text); }
                emit(json!({ "message_type": "assistant_message", "content": text }))
            }
            Ok(StreamChunk::ToolCall(tc)) => {
                if let Ok(mut g) = acc_clone.lock() {
                    g.1.push(json!({ "id": tc.id, "name": tc.name, "arguments": tc.arguments }));
                }
                emit(json!({
                    "message_type": "tool_call_message",
                    "tool_call": { "id": tc.id, "name": tc.name, "arguments": tc.arguments }
                }))
            }
            Ok(StreamChunk::Usage(u)) => {
                if let Ok(mut acc) = usage_acc2.lock() {
                    acc.input_tokens  += u.input_tokens;
                    acc.output_tokens += u.output_tokens;
                }
                // Emit usage_statistics event for client-side display
                emit(json!({
                    "message_type":     "usage_statistics",
                    "input_tokens":     u.input_tokens,
                    "output_tokens":    u.output_tokens,
                    "cache_read_tokens": u.cache_read_tokens,
                    "model":            u.model,
                }))
            }
            Ok(StreamChunk::Done) => {
                if let Ok(g) = acc_clone.lock() {
                    persist(&state_clone, &agent_id_clone, conv_id_clone.as_deref(), "assistant", json!({
                        "content": g.0,
                        "tool_calls": g.1
                    }));
                }
                if let Some(rid) = &run_id_clone {
                    let _ = sqlite::finish_run(&db_clone, rid, "completed");
                }
                Event::default().data("[DONE]")
            }
            Err(e) => {
                if let Some(rid) = &run_id_clone {
                    let _ = sqlite::finish_run(&db_clone, rid, "failed");
                }
                Event::default().data(json!({ "error": e.to_string() }).to_string())
            }
        };
        Ok::<Event, std::convert::Infallible>(event)
    });

    drop(acc);
    drop(usage_acc);
    let _ = background;

    Sse::new(futures::StreamExt::chain(meta_event, sse_stream)).into_response()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Set conversation title from first user message if title is still empty.
fn maybe_set_conv_title(state: &AppState, conv_id: &str, text: &str) {
    if let Ok(Some(c)) = sqlite::get_conversation(&state.db, conv_id) {
        if c.title.is_empty() {
            let title: String = text.chars().take(CONV_TITLE_MAX).collect();
            let title = title.trim().to_string();
            if !title.is_empty() {
                let _ = sqlite::update_conversation_title(&state.db, conv_id, &title);
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "detail": msg }))).into_response()
}
