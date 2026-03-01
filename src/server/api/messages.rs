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
    llm::{CompletionRequest, LlmMessage, LlmToolCall, StreamChunk},
    state::AppState,
    storage::sqlite::{self, MessageRow},
};

const HISTORY_LIMIT: usize = 40;
const MAX_TOKENS: u32 = 8192;

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
) -> Result<(String, Vec<LlmMessage>, Vec<Value>), String> {
    let agent = sqlite::get_agent(&state.db, agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;

    // Memory blocks → appended to system prompt
    let memory = sqlite::get_memory_blocks(&state.db, agent_id)
        .unwrap_or_default()
        .into_iter()
        .map(|(label, val)| format!("[{label}]\n{val}"))
        .collect::<Vec<_>>()
        .join("\n\n");

    let system_prompt = if memory.is_empty() {
        agent.system_prompt.clone().unwrap_or_default()
    } else {
        format!("{}\n\n# Memory\n{memory}", agent.system_prompt.unwrap_or_default())
    };

    // Message history from DB — oldest first
    let history = sqlite::list_messages(&state.db, agent_id, HISTORY_LIMIT)
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
        "tool" => vec![LlmMessage {
            role: "tool".to_string(),
            content: row.content["content"].as_str().unwrap_or("").to_string(),
            tool_call_id: row.content["tool_call_id"].as_str().map(String::from),
            tool_calls: None,
        }],
        "assistant" => {
            // A single DB row may have both text content and tool_calls
            let text = row.content["content"].as_str().unwrap_or("").to_string();
            let tool_calls: Option<Vec<LlmToolCall>> =
                row.content["tool_calls"].as_array().map(|arr| {
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

fn persist(state: &AppState, agent_id: &str, role: &str, content: Value) {
    let row = MessageRow {
        id: new_msg_id(),
        agent_id: agent_id.to_string(),
        role: role.to_string(),
        content,
    };
    let _ = sqlite::insert_message(&state.db, &row);
}

// ── POST /v1/agents/:id/messages  (blocking) ─────────────────────────────────

pub async fn send_message(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if body["role"].as_str() == Some("tool") {
        return handle_tool_return_blocking(&state, &agent_id, &body).await;
    }

    let input = match body["input"].as_str().filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "missing 'input'"),
    };

    // 1. Persist user message FIRST
    persist(&state, &agent_id, "user", json!({ "content": input }));

    // 2. Build context from DB (includes the message we just persisted)
    let (model, messages, tools) = match build_context(&state, &agent_id).await {
        Ok(ctx) => ctx,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };

    // 3. Call LLM
    let req = CompletionRequest { model, messages, tools, max_tokens: MAX_TOKENS };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            // 4. Persist LLM response as ONE assistant message (text + tool_calls together)
            let tool_calls_json: Vec<Value> = resp.tool_calls.iter().map(|tc| json!({
                "id": tc.id, "name": tc.name, "arguments": tc.arguments
            })).collect();
            persist(&state, &agent_id, "assistant", json!({
                "content": resp.content.clone().unwrap_or_default(),
                "tool_calls": tool_calls_json
            }));

            // 5. Build response events for CLI
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
            Json(json!({ "messages": out })).into_response()
        }
        Err(e) => {
            tracing::error!("LLM error: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        }
    }
}

async fn handle_tool_return_blocking(state: &AppState, agent_id: &str, body: &Value) -> Response {
    let tr = &body["tool_return"];
    let call_id = tr["tool_call_id"].as_str().unwrap_or("").to_string();
    let content = tr["content"].as_str().unwrap_or("").to_string();

    // 1. Persist tool result FIRST
    persist(state, agent_id, "tool", json!({
        "content": content,
        "tool_call_id": call_id
    }));

    // 2. Check if all tool results for this turn have arrived.
    //    Anthropic requires ALL tool_results in ONE user message — we must
    //    wait until the CLI has sent every result before calling the LLM.
    match sqlite::pending_tool_results(&state.db, agent_id) {
        Ok((received, expected)) if received < expected => {
            tracing::debug!(
                "Tool results: {received}/{expected} received — waiting for more"
            );
            // Return empty — CLI will send the next tool result shortly
            return Json(json!({ "messages": [] })).into_response();
        }
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        _ => {} // all results in — proceed to LLM
    }

    // 3. Build context from DB (all tool results now present)
    let (model, messages, tools) = match build_context(state, agent_id).await {
        Ok(ctx) => ctx,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };

    // 4. Call LLM
    let req = CompletionRequest { model, messages, tools, max_tokens: MAX_TOKENS };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            let tool_calls_json: Vec<Value> = resp.tool_calls.iter().map(|tc| json!({
                "id": tc.id, "name": tc.name, "arguments": tc.arguments
            })).collect();
            persist(state, agent_id, "assistant", json!({
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
            Json(json!({ "messages": out })).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ── POST /v1/agents/:id/messages/stream  (SSE) ───────────────────────────────

pub async fn stream_message(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let is_tool_return = body["role"].as_str() == Some("tool");

    // 1. Persist incoming message FIRST
    if is_tool_return {
        let tr = &body["tool_return"];
        persist(&state, &agent_id, "tool", json!({
            "content": tr["content"].as_str().unwrap_or(""),
            "tool_call_id": tr["tool_call_id"].as_str().unwrap_or("")
        }));
    } else {
        let input = match body["input"].as_str().filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => return err(StatusCode::BAD_REQUEST, "missing 'input'"),
        };
        persist(&state, &agent_id, "user", json!({ "content": input }));
    }

    // 2. If this was a tool return, check if all results for this turn have arrived.
    if is_tool_return {
        match sqlite::pending_tool_results(&state.db, &agent_id) {
            Ok((received, expected)) if received < expected => {
                tracing::debug!("Stream: tool results {received}/{expected} — waiting");
                // Return a trivial SSE stream that immediately closes
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
    let (model, messages, tools) = match build_context(&state, &agent_id).await {
        Ok(ctx) => ctx,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };

    let req = CompletionRequest { model, messages, tools, max_tokens: MAX_TOKENS };
    let state_clone = state.clone();
    let agent_id_clone = agent_id.clone();

    // 3. Open LLM stream
    let llm_stream = match state.llm.stream(&req).await {
        Ok(s) => s,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    // 4. Track accumulated response for persistence
    let _acc_text = String::new();
    let _acc_tools: Vec<Value> = Vec::new();

    // We can't mutate acc_* inside the closure directly (moved into stream),
    // so we use a channel to collect them after streaming.
    // Instead: collect accumulation via Arc<Mutex> shared state.
    let acc = std::sync::Arc::new(std::sync::Mutex::new((String::new(), Vec::<Value>::new())));
    let acc_clone = acc.clone();

    let sse_stream = futures::StreamExt::map(llm_stream, move |chunk: Result<StreamChunk>| {
        let event = match chunk {
            Ok(StreamChunk::Text(text)) => {
                if let Ok(mut g) = acc_clone.lock() { g.0.push_str(&text); }
                let data = json!({ "message_type": "assistant_message", "content": text });
                Event::default().data(data.to_string())
            }
            Ok(StreamChunk::ToolCall(tc)) => {
                if let Ok(mut g) = acc_clone.lock() {
                    g.1.push(json!({ "id": tc.id, "name": tc.name, "arguments": tc.arguments }));
                }
                let data = json!({
                    "message_type": "tool_call_message",
                    "tool_call": { "id": tc.id, "name": tc.name, "arguments": tc.arguments }
                });
                Event::default().data(data.to_string())
            }
            Ok(StreamChunk::Done) => {
                // Persist the complete assistant response (text + tools) as ONE row
                if let Ok(g) = acc_clone.lock() {
                    persist(&state_clone, &agent_id_clone, "assistant", json!({
                        "content": g.0,
                        "tool_calls": g.1
                    }));
                }
                Event::default().data("[DONE]")
            }
            Err(e) => Event::default().data(json!({ "error": e.to_string() }).to_string()),
        };
        Ok::<Event, std::convert::Infallible>(event)
    });

    // Suppress unused-variable warning (acc used via acc_clone inside closure)
    drop(acc);

    Sse::new(sse_stream).into_response()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "detail": msg }))).into_response()
}
