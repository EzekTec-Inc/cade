use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response, Sse},
    Json,
};
use axum::response::sse::Event;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio_stream::StreamExt as TokioStreamExt;
use uuid::Uuid;

use anyhow::Result;
use crate::server::{
    llm::{CompletionRequest, LlmMessage, StreamChunk},
    state::AppState,
    storage::sqlite::{self, MessageRow},
};

const HISTORY_LIMIT: usize = 40;
const MAX_TOKENS: u32 = 8192;

// ── Shared context builder ────────────────────────────────────────────────────

async fn build_context(
    state: &AppState,
    agent_id: &str,
    new_message: LlmMessage,
) -> Result<(String, Vec<LlmMessage>, Vec<Value>), String> {
    // Load agent
    let agent = sqlite::get_agent(&state.db, agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;

    // Load memory blocks
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

    // Load message history
    let history = sqlite::list_messages(&state.db, agent_id, HISTORY_LIMIT)
        .unwrap_or_default();

    let mut messages: Vec<LlmMessage> = vec![LlmMessage {
        role: "system".to_string(),
        content: system_prompt.clone(),
        tool_call_id: None,
        tool_calls: None,
    }];

    // Replay history
    for row in &history {
        let msg = history_row_to_llm(row);
        messages.extend(msg);
    }

    messages.push(new_message);

    // Load tool schemas for this agent
    let all_tools = sqlite::list_tools(&state.db).unwrap_or_default();
    let tool_schemas: Vec<Value> = all_tools.iter()
        .filter_map(|t| t.json_schema.clone())
        .collect();

    Ok((agent.model, messages, tool_schemas))
}

fn history_row_to_llm(row: &MessageRow) -> Vec<LlmMessage> {
    match row.role.as_str() {
        "tool" => vec![LlmMessage {
            role: "tool".to_string(),
            content: row.content["content"].as_str().unwrap_or("").to_string(),
            tool_call_id: row.content["tool_call_id"].as_str().map(String::from),
            tool_calls: None,
        }],
        "assistant" => vec![LlmMessage {
            role: "assistant".to_string(),
            content: row.content["content"].as_str().unwrap_or("").to_string(),
            tool_call_id: None,
            tool_calls: row.content["tool_calls"].as_array().map(|arr| {
                arr.iter().filter_map(|tc| serde_json::from_value(tc.clone()).ok()).collect()
            }),
        }],
        _ => vec![LlmMessage {
            role: row.role.clone(),
            content: row.content["content"].as_str().unwrap_or("").to_string(),
            tool_call_id: None,
            tool_calls: None,
        }],
    }
}

fn persist_message(state: &AppState, agent_id: &str, role: &str, content: Value) {
    let row = MessageRow {
        id: format!("msg-{}", Uuid::new_v4()),
        agent_id: agent_id.to_string(),
        role: role.to_string(),
        content,
    };
    let _ = sqlite::insert_message(&state.db, &row);
}

// ── Blocking endpoint: POST /v1/agents/:id/messages ──────────────────────────

pub async fn send_message(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    // Handle both user messages and tool returns
    if body["role"].as_str() == Some("tool") {
        return handle_tool_return(&state, &agent_id, &body).await;
    }

    let input = match body["input"].as_str() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return (StatusCode::BAD_REQUEST, Json(json!({"detail": "missing 'input'"}))).into_response(),
    };

    // Persist user message
    persist_message(&state, &agent_id, "user", json!({"content": input}));

    let new_msg = LlmMessage { role: "user".to_string(), content: input, tool_call_id: None, tool_calls: None };

    let (model, messages, tools) = match build_context(&state, &agent_id, new_msg).await {
        Ok(ctx) => ctx,
        Err(e) => return (StatusCode::NOT_FOUND, Json(json!({"detail": e}))).into_response(),
    };

    let req = CompletionRequest { model, messages, tools, max_tokens: MAX_TOKENS };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            let mut out_messages: Vec<Value> = vec![];

            if let Some(text) = &resp.content {
                persist_message(&state, &agent_id, "assistant", json!({"content": text}));
                out_messages.push(json!({
                    "message_type": "assistant_message",
                    "content": text
                }));
            }
            for tc in &resp.tool_calls {
                persist_message(&state, &agent_id, "assistant", json!({
                    "content": "", "tool_calls": [{"id": tc.id, "name": tc.name, "arguments": tc.arguments}]
                }));
                out_messages.push(json!({
                    "message_type": "tool_call_message",
                    "tool_call": {"id": tc.id, "name": tc.name, "arguments": tc.arguments}
                }));
            }
            Json(json!({"messages": out_messages})).into_response()
        }
        Err(e) => {
            tracing::error!("LLM error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e.to_string()}))).into_response()
        }
    }
}

async fn handle_tool_return(state: &AppState, agent_id: &str, body: &Value) -> Response {
    let tr = &body["tool_return"];
    let call_id = tr["tool_call_id"].as_str().unwrap_or("").to_string();
    let content = tr["content"].as_str().unwrap_or("").to_string();
    let status = tr["status"].as_str().unwrap_or("success");

    persist_message(state, agent_id, "tool", json!({
        "content": content,
        "tool_call_id": call_id,
        "status": status
    }));

    // Continue the conversation with the tool result
    let tool_msg = LlmMessage {
        role: "tool".to_string(),
        content: content.clone(),
        tool_call_id: Some(call_id),
        tool_calls: None,
    };

    let (model, messages, tools) = match build_context(state, agent_id, tool_msg).await {
        Ok(ctx) => ctx,
        Err(e) => return (StatusCode::NOT_FOUND, Json(json!({"detail": e}))).into_response(),
    };

    let req = CompletionRequest { model, messages, tools, max_tokens: MAX_TOKENS };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            let mut out_messages: Vec<Value> = vec![];
            if let Some(text) = &resp.content {
                persist_message(state, agent_id, "assistant", json!({"content": text}));
                out_messages.push(json!({"message_type": "assistant_message", "content": text}));
            }
            for tc in &resp.tool_calls {
                persist_message(state, agent_id, "assistant", json!({
                    "content": "", "tool_calls": [{"id": tc.id, "name": tc.name, "arguments": tc.arguments}]
                }));
                out_messages.push(json!({
                    "message_type": "tool_call_message",
                    "tool_call": {"id": tc.id, "name": tc.name, "arguments": tc.arguments}
                }));
            }
            Json(json!({"messages": out_messages})).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e.to_string()}))).into_response(),
    }
}

// ── Streaming endpoint: POST /v1/agents/:id/messages/stream ──────────────────

pub async fn stream_message(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let is_tool_return = body["role"].as_str() == Some("tool");

    let new_msg = if is_tool_return {
        let tr = &body["tool_return"];
        let call_id = tr["tool_call_id"].as_str().unwrap_or("").to_string();
        let content = tr["content"].as_str().unwrap_or("").to_string();
        persist_message(&state, &agent_id, "tool", json!({
            "content": &content, "tool_call_id": &call_id
        }));
        LlmMessage { role: "tool".to_string(), content, tool_call_id: Some(call_id), tool_calls: None }
    } else {
        let input = match body["input"].as_str() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return (StatusCode::BAD_REQUEST, Json(json!({"detail": "missing 'input'"}))).into_response(),
        };
        persist_message(&state, &agent_id, "user", json!({"content": &input}));
        LlmMessage { role: "user".to_string(), content: input, tool_call_id: None, tool_calls: None }
    };

    let (model, messages, tools) = match build_context(&state, &agent_id, new_msg).await {
        Ok(ctx) => ctx,
        Err(e) => return (StatusCode::NOT_FOUND, Json(json!({"detail": e}))).into_response(),
    };

    let req = CompletionRequest { model, messages, tools, max_tokens: MAX_TOKENS };
    let state_clone = state.clone();
    let agent_id_clone = agent_id.clone();

    let llm_stream = match state.llm.stream(&req).await {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": e.to_string()}))).into_response(),
    };

    // Map StreamChunk → SSE Events in the CadeMessage format
    // Use futures::StreamExt::map explicitly to avoid ambiguity with tokio_stream::StreamExt
    let sse_stream = futures::StreamExt::map(llm_stream, move |chunk: Result<StreamChunk>| {
        let event = match chunk {
            Ok(StreamChunk::Text(text)) => {
                let data = json!({"message_type": "assistant_message", "content": text});
                persist_message(&state_clone, &agent_id_clone, "assistant", json!({"content": &data["content"]}));
                Event::default().data(data.to_string())
            }
            Ok(StreamChunk::ToolCall(tc)) => {
                persist_message(&state_clone, &agent_id_clone, "assistant", json!({
                    "content": "",
                    "tool_calls": [{"id": tc.id, "name": tc.name, "arguments": tc.arguments}]
                }));
                let data = json!({
                    "message_type": "tool_call_message",
                    "tool_call": {"id": tc.id, "name": tc.name, "arguments": tc.arguments}
                });
                Event::default().data(data.to_string())
            }
            Ok(StreamChunk::Done) => Event::default().data("[DONE]"),
            Err(e)               => Event::default().data(json!({"error": e.to_string()}).to_string()),
        };
        Ok::<Event, std::convert::Infallible>(event)
    });

    Sse::new(sse_stream).into_response()
}
