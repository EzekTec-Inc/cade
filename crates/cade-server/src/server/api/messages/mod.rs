pub mod context;
pub mod persist;
pub(crate) use context::*;
pub(crate) use persist::*;

use axum::response::sse::Event;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response, Sse},
};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::server::state::AppState;
use cade_store::sqlite::{self, MessageRow};
use cade_ai::catalogue;
use cade_ai::{CompletionRequest, LlmMessage, LlmToolCall, MessageImage, StreamChunk, TokenUsage};

/// Maximum length for auto-generated conversation titles (chars from first user message).
const CONV_TITLE_MAX: usize = 60;
/// Appended to every agent's system prompt so the LLM always produces
/// plain-text analysis after tool use, regardless of the stored system_prompt.
pub(crate) const TOOL_RESPONSE_RULE: &str = "\n\n\
After every tool execution, always provide a plain-text response that explains \
the result, what you found, or what you are doing next. \
Never end a turn silently after running a tool. \
Do not include filler phrases like 'Understood' or 'I will adhere to the rules'. Just do the work.";
/// Number of recent messages examined when deciding whether to include MCP
/// tool schemas.  MCP tools (identified by `__` namespace separator) are only
/// sent when actually called within this window, saving prompt tokens.
pub(crate) const RECENT_WINDOW: usize = 20;
/// Tool names that must always appear in the tool-schema list even when extended
/// tools are pruned on long conversations.  These are the agent's primary
/// mechanism for recovering archived context and must never be silently dropped.
pub(crate) const ALWAYS_INCLUDE_TOOL_NAMES: &[&str] = &[
    "search_memory",
    "conversation_search",
    "archival_memory_insert",
    "archival_memory_search",
    "update_memory",
    "memory_apply_patch",
];
/// Character budget for pinned memory blocks (always injected, highest priority).
pub(crate) const PINNED_BUDGET: usize = 10_000;
/// Character budget for short-term active memory blocks (full fidelity).
pub(crate) const SHORT_BUDGET: usize = 25_000;
/// Character budget for the long-term archived index (label + 80-char excerpt).
pub(crate) const LONG_BUDGET: usize = 5_000;
/// Turns of inactivity before a short-term block is promoted to long-term.
pub(crate) const STALE_THRESHOLD: i64 = 80;
/// Awareness footer appended to system prompt when any memory tier is present.
pub(crate) const MEMORY_AWARENESS_FOOTER: &str = "\n\nMemory system: blocks idle for 80+ turns are \
archived. The Archived Memory section above lists them with label + excerpt only. \
To retrieve a full archived block, call the `search_memory` tool with a keyword — \
matched blocks are automatically promoted back to active memory. \
To search dropped conversation history, use the `conversation_search` tool. \
To keep a critical block permanently active, ask the user to run `/memory pin <label>`.";
/// Cap on a single tool-result content string (chars). ~2k tokens.
/// Prevents huge outputs (screenshots, logs) from blowing the context window.
/// 8 192 chars covers the vast majority of useful tool outputs (diffs, file
/// excerpts, command output) while cutting worst-case cost by 75% vs 32 768.
const TOOL_RESULT_MAX_CHARS: usize = 8_192;
/// Chars-per-token approximation used to convert a model's token context window
/// into a character budget.  The budget formula is:
///   char_budget = input_budget_tokens × CHARS_PER_TOKEN
/// A LOWER value is more conservative (allows fewer chars per allocated token).
/// 3 chars/token with typical 3.5–4 c/t text yields ~15–25% headroom below the
/// hard token limit, preventing accidental overflow.
pub(crate) const CHARS_PER_TOKEN: usize = 3;
/// Minimum character budget regardless of model window (guards tiny local models).
pub(crate) const MIN_CONTEXT_CHARS: usize = 8_000;
/// Maximum character budget cap.  6_000_000 chars ≈ 2 M tokens at 3 chars/token,
/// which fully covers Gemini 2 M.  Claude 200 K is unaffected
/// (200_000 × 3 = 600_000 < cap).
pub(crate) const MAX_CONTEXT_CHARS: usize = 6_000_000;

/// Fraction of the context window reserved for the model's output (including
/// reasoning/thinking tokens).  0.15 means 15% of the total window is off-limits
/// to input context.  For a 128k model this reserves ~19k tokens for output,
/// which is enough for max_tokens (8192) + reasoning budget (up to 16k).
pub(crate) const OUTPUT_RESERVE_FRACTION: f64 = 0.15;

/// Estimated per-tool schema overhead in characters.  Used to subtract tool
/// schema cost from the context budget *before* filling with message history.
/// A typical tool schema is ~400–800 chars; 600 is a reasonable average.
pub(crate) const TOOL_SCHEMA_CHARS_ESTIMATE: usize = 600;

// -- Auto-compaction constants

/// Context usage ratio at which auto-compaction (summarization) triggers.
/// 0.85 means: when the assembled messages use ≥ 85% of the (already-reserved)
/// char budget.  Triggering earlier than the old 0.98 gives the summarizer
/// room to run without the hard-trim loop immediately evicting the summary.
/// Emergency compaction threshold — bypasses cooldown.  When context hits this
/// level, compaction runs regardless of how recently the last one happened.
/// Minimum number of user+assistant messages that must exist before
/// compaction is even considered (avoids summarizing trivially short sessions).
/// Number of recent messages to keep at full fidelity (never summarized).
/// Must be ≥ 4 so the model always sees the latest user+assistant exchange.
/// Cooldown: minimum number of turns between successive compactions for
/// the same agent.  Prevents re-summarizing every turn once the threshold
/// is crossed.  Bypassed when usage ≥ COMPACT_EMERGENCY_THRESHOLD.

// -- Message history sanitizer
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

    // Track last-active timestamp; needs_consolidation is set inside build_context
    // when turns are actually dropped — not unconditionally on every message.
    {
        let mut activity = state.agent_activity.write().await;
        let entry =
            activity
                .entry(agent_id.clone())
                .or_insert(crate::server::state::AgentActivity {
                    last_active_ts: 0,
                    needs_consolidation: false,
                    conversation_id: conv_id.clone(),
                });
        entry.last_active_ts = chrono::Utc::now().timestamp();
        entry.conversation_id = conv_id.clone();
    }

    if body["role"].as_str() == Some("tool") {
        return handle_tool_return_blocking(&state, &agent_id, conv_id_ref, &body).await;
    }

    let input = match body["input"].as_str().filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => return err(StatusCode::BAD_REQUEST, "missing 'input'"),
    };

    // Collect optional inline images from the request body.
    // Each element must have "media_type" and "data" (base64) fields.
    let req_images: Option<Vec<MessageImage>> = body["images"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .filter(|v: &Vec<_>| !v.is_empty());

    // 1. Persist user message FIRST (skip for ephemeral system injections)
    let is_ephemeral = body["ephemeral"].as_bool().unwrap_or(false);
    if !is_ephemeral {
        let mut user_content = json!({ "content": input });
        if let Some(imgs) = &req_images {
            user_content["images"] = serde_json::to_value(imgs).unwrap_or(Value::Null);
        }
        persist(&state, &agent_id, conv_id_ref, "user", user_content);
    }

    // 2. Build context from DB (includes the message we just persisted)
    let (model, mut messages, tools) =
        match build_context(&state, &agent_id, conv_id_ref, false).await {
            Ok(ctx) => ctx,
            Err(e) => return err(StatusCode::NOT_FOUND, &e),
        };

    // 2b. Ephemeral messages were not persisted — inject into context so the
    // LLM actually sees them.  Without this the re-prompt text is silently lost.
    if is_ephemeral {
        messages.push(LlmMessage {
            role: "user".to_string(),
            content: input.clone(),
            tool_call_id: None,
            tool_calls: None,
            images: req_images.clone(),
        });
    }

    // 3. Call LLM
    let max_tokens = catalogue::max_tokens_for_model(&model);
    let reasoning_effort = body
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(String::from);
    let req = CompletionRequest {
        model,
        messages,
        tools,
        max_tokens,
        reasoning_effort,
    };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            let tool_calls_json: Vec<Value> = resp
                .tool_calls
                .iter()
                .filter_map(|tc| serde_json::to_value(tc).ok())
                .collect();
            // Skip persisting empty assistant responses — they clutter the
            // conversation and can produce invalid turn ordering on next load.
            let has_content = resp.content.as_ref().is_some_and(|s| !s.is_empty());
            let has_tools = !resp.tool_calls.is_empty();
            if has_content || has_tools {
                persist(
                    &state,
                    &agent_id,
                    conv_id_ref,
                    "assistant",
                    json!({
                        "content": resp.content.clone().unwrap_or_default(),
                        "tool_calls": tool_calls_json
                    }),
                );
            }

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
    let content = tr["content"].as_str().unwrap_or("").to_string();

    persist(
        state,
        agent_id,
        conv_id,
        "tool",
        json!({
            "content": content, "tool_call_id": call_id
        }),
    );

    match sqlite::pending_tool_results(&state.db, agent_id, conv_id) {
        Ok((received, expected)) if received < expected => {
            tracing::debug!("Tool results: {received}/{expected} — waiting");
            return Json(json!({ "messages": [], "conversation_id": conv_id })).into_response();
        }
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        _ => {}
    }

    let (model, messages, tools) = match build_context(state, agent_id, conv_id, true).await {
        Ok(ctx) => ctx,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };

    let max_tokens = catalogue::max_tokens_for_model(&model);
    let reasoning_effort = body
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(String::from);
    let req = CompletionRequest {
        model,
        messages,
        tools,
        max_tokens,
        reasoning_effort,
    };
    match state.llm.complete(&req).await {
        Ok(resp) => {
            let tool_calls_json: Vec<Value> = resp
                .tool_calls
                .iter()
                .filter_map(|tc| serde_json::to_value(tc).ok())
                .collect();
            let has_content = resp.content.as_ref().is_some_and(|s| !s.is_empty());
            let has_tools = !resp.tool_calls.is_empty();
            if has_content || has_tools {
                persist(
                    state,
                    agent_id,
                    conv_id,
                    "assistant",
                    json!({
                        "content": resp.content.clone().unwrap_or_default(),
                        "tool_calls": tool_calls_json
                    }),
                );
            }
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

// -- POST /v1/agents/:id/messages/stream  (SSE)

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

    // Track last-active timestamp; needs_consolidation is set inside build_context
    // when turns are actually dropped — not unconditionally on every message.
    {
        let mut activity = state.agent_activity.write().await;
        let entry =
            activity
                .entry(agent_id.clone())
                .or_insert(crate::server::state::AgentActivity {
                    last_active_ts: 0,
                    needs_consolidation: false,
                    conversation_id: conv_id.clone(),
                });
        entry.last_active_ts = chrono::Utc::now().timestamp();
        entry.conversation_id = conv_id.clone();
    }

    let is_tool_return = body["role"].as_str() == Some("tool");

    // 1. Persist incoming message FIRST
    if is_tool_return {
        let tr = &body["tool_return"];
        persist(
            &state,
            &agent_id,
            conv_id_ref,
            "tool",
            json!({
                "content": tr["content"].as_str().unwrap_or(""),
                "tool_call_id": tr["tool_call_id"].as_str().unwrap_or("")
            }),
        );
    } else {
        let input = match body["input"].as_str().filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => return err(StatusCode::BAD_REQUEST, "missing 'input'"),
        };
        // ephemeral=true: system-injected re-prompt — send to LLM but don't
        // persist to the DB so it never appears in conversation history.
        let is_ephemeral = body["ephemeral"].as_bool().unwrap_or(false);
        if !is_ephemeral {
            // Auto-title new conversations from the first user message
            if let Some(cid) = conv_id_ref {
                maybe_set_conv_title(&state, cid, &input);
            }
            persist(
                &state,
                &agent_id,
                conv_id_ref,
                "user",
                json!({ "content": input }),
            );
        }
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
    let (model, mut messages, tools) =
        match build_context(&state, &agent_id, conv_id_ref, is_tool_return).await {
            Ok(ctx) => ctx,
            Err(e) => return err(StatusCode::NOT_FOUND, &e),
        };

    // 3b. Ephemeral messages were not persisted — inject into context so the
    // LLM actually sees them.  Without this the re-prompt text is silently
    // lost and the LLM is called with the same context that already produced
    // an empty response.
    if !is_tool_return {
        let is_ephemeral = body["ephemeral"].as_bool().unwrap_or(false);
        if is_ephemeral && let Some(input) = body["input"].as_str().filter(|s| !s.is_empty()) {
            messages.push(LlmMessage {
                role: "user".to_string(),
                content: input.to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
            });
        }
    }

    let background = body["background"].as_bool().unwrap_or(false);
    // Create a run for background (and also for foreground — keeps history for reconnect)
    let run = sqlite::create_run(&state.db, &agent_id, conv_id_ref);
    let run_id: Option<String> = run.ok().map(|r| r.id);

    let max_tokens = catalogue::max_tokens_for_model(&model);
    let reasoning_effort = body
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(String::from);
    let req = CompletionRequest {
        model,
        messages,
        tools,
        max_tokens,
        reasoning_effort,
    };
    let state_clone = state.clone();
    let agent_id_clone = agent_id.clone();
    let conv_id_clone = conv_str.clone();
    let run_id_clone = run_id.clone();
    let db_clone = state.db.clone();

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
                    Event::default().data(json!({ "error": err_msg }).to_string()),
                ),
                Ok::<Event, std::convert::Infallible>(Event::default().data("[DONE]")),
            ]);
            return Sse::new(s).into_response();
        }
    };

    let acc = std::sync::Arc::new(std::sync::Mutex::new((String::new(), Vec::<Value>::new(), String::new())));
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

    let sse_stream =
        futures::StreamExt::map(llm_stream, move |chunk: cade_ai::Result<StreamChunk>| {
            // Persist each event to run_events so the stream is resumable
            let emit = |data: Value| -> Event {
                if let Some(rid) = &run_id_clone
                    && let Ok(seq) = sqlite::append_run_event(&db_clone, rid, &data.to_string())
                {
                    let mut d = data.clone();
                    if let Some(obj) = d.as_object_mut() {
                        obj.insert("run_id".to_string(), serde_json::Value::String(rid.clone()));
                        obj.insert("seq_id".to_string(), serde_json::Value::Number(seq.into()));
                    }
                    return Event::default().data(d.to_string());
                }
                Event::default().data(data.to_string())
            };

            let event = match chunk {
                Ok(StreamChunk::Reasoning(text)) => {
                    if let Ok(mut g) = acc_clone.lock() {
                        g.2.push_str(&text);
                    }
                    emit(json!({ "message_type": "reasoning_message", "reasoning": text }))
                }
                Ok(StreamChunk::Text(text)) => {
                    if let Ok(mut g) = acc_clone.lock() {
                        g.0.push_str(&text);
                    }
                    emit(json!({ "message_type": "assistant_message", "content": text }))
                }
                Ok(StreamChunk::ToolCall(tc)) => {
                    if let Ok(mut g) = acc_clone.lock()
                        && let Ok(v) = serde_json::to_value(&tc)
                    {
                        g.1.push(v);
                    }
                    emit(json!({
                        "message_type": "tool_call_message",
                        "tool_call": { "id": tc.id, "name": tc.name, "arguments": tc.arguments }
                    }))
                }
                Ok(StreamChunk::Usage(u)) => {
                    if let Ok(mut acc) = usage_acc2.lock() {
                        acc.input_tokens += u.input_tokens;
                        acc.output_tokens += u.output_tokens;
                    }
                    // Emit usage_statistics event for client-side display
                    emit(json!({
                        "message_type":      "usage_statistics",
                        "input_tokens":      u.input_tokens,
                        "output_tokens":     u.output_tokens,
                        "cache_read_tokens":  u.cache_read_tokens,
                        "cache_write_tokens": u.cache_write_tokens,
                        "model":             u.model,
                    }))
                }
                Ok(StreamChunk::FinishReason(reason)) => emit(json!({
                    "message_type": "finish_reason",
                    "reason": reason,
                })),
                Ok(StreamChunk::Done) => {
                    if let Ok(g) = acc_clone.lock() {
                        // Skip persisting empty assistant responses — they clutter
                        // the conversation and produce invalid turn ordering on
                        // next context load (e.g. Gemini consecutive-user-turn 400).
                        if !g.0.is_empty() || !g.1.is_empty() || !g.2.is_empty() {
                            let mut content = g.0.clone();
                            if !g.2.is_empty() {
                                content = format!("<reasoning>\n{}\n</reasoning>\n\n{}", g.2, content);
                            }
                            persist(
                                &state_clone,
                                &agent_id_clone,
                                conv_id_clone.as_deref(),
                                "assistant",
                                json!({
                                    "content": content,
                                    "tool_calls": g.1
                                }),
                            );
                        }
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

// -- Helpers

/// Set conversation title from first user message if title is still empty.
fn maybe_set_conv_title(state: &AppState, conv_id: &str, text: &str) {
    if let Ok(Some(c)) = sqlite::get_conversation(&state.db, conv_id)
        && c.title.is_empty()
    {
        let title: String = text.chars().take(CONV_TITLE_MAX).collect();
        let title = title.trim().to_string();
        if !title.is_empty() {
            let _ = sqlite::update_conversation_title(&state.db, conv_id, &title);
        }
    }
}

// -- Helpers

pub(crate) fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "detail": msg }))).into_response()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
