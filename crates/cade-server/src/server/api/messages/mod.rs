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
use cade_ai::catalogue;
use cade_ai::{CompletionRequest, LlmMessage, LlmToolCall, MessageImage, StreamChunk, TokenUsage};
use cade_store::sqlite::{self, MessageRow};

/// Maximum length for auto-generated conversation titles (chars from first user message).
const CONV_TITLE_MAX: usize = 60;
/// Number of recent messages examined when deciding whether to include MCP
/// tool schemas.  MCP tools (identified by `__` namespace separator) are only
/// sent when actually called within this window, saving prompt tokens.
pub(crate) const RECENT_WINDOW: usize = 20;
/// Minimum character budget for pinned memory blocks (always injected, highest priority).
pub(crate) const PINNED_BUDGET_MIN: usize = 10_000;
/// Minimum character budget for short-term active memory blocks (full fidelity).
pub(crate) const SHORT_BUDGET_MIN: usize = 25_000;
/// Minimum character budget for the long-term archived index (label + 80-char excerpt).
pub(crate) const LONG_BUDGET_MIN: usize = 5_000;

/// Fraction of the message budget at which `build_context` proactively
/// signals the Sleeptime consolidation task.  Lowered from 80% → 70% as
/// F4 of the 2026-04-30 memory-system refactor: consolidation costs an
/// LLM call and runs on a 20 s timer, so we need a wider runway between
/// "summary requested" and "next request actually overflows".
pub(crate) const PROACTIVE_CONSOLIDATION_THRESHOLD: f64 = 0.70;

/// Dynamic memory budgets scaled to the model's context window.
///
/// On small models (32k) the budgets equal the minimums.  On large models
/// (128k+) they scale proportionally so the agent can hold more memory
/// without wasting context capacity.
///
/// Formula per tier:
///   budget = max(MIN, context_window_chars × fraction)
///
/// Fractions:
///   pinned  = 2%   (persona, human, project, working_set)
///   short   = 8%   (active task notes, skills metadata)
///   long    = 1.5% (archived excerpts)
#[derive(Debug, Clone, Copy)]
pub(crate) struct MemoryBudgets {
    pub pinned: usize,
    pub short: usize,
    pub long: usize,
}

impl MemoryBudgets {
    /// Compute budgets from the model's context window (in tokens).
    pub fn for_model(model: &str) -> Self {
        let window_tokens = catalogue::context_window_for_model(model) as usize;
        // Approximate chars: tokens × 4 (slightly generous to avoid under-budgeting)
        let window_chars = window_tokens.saturating_mul(4);
        Self {
            pinned: ((window_chars as f64 * 0.02) as usize).max(PINNED_BUDGET_MIN),
            short: ((window_chars as f64 * 0.08) as usize).max(SHORT_BUDGET_MIN),
            long: ((window_chars as f64 * 0.015) as usize).max(LONG_BUDGET_MIN),
        }
    }

    /// A6: Observation window scaled to the model's context window.
    ///
    /// Returns `(max_observations, char_budget)`.
    ///
    /// | Model window | Observations | Char budget |
    /// |-------------|-------------|-------------|
    /// | 32k tokens  | 30          | 2,000       |
    /// | 128k tokens | 50          | 4,000       |
    /// | 200k tokens | 75          | 6,000       |
    /// | 1M tokens   | 150         | 12,000      |
    pub fn observation_budget(model: &str) -> (usize, usize) {
        let window_tokens = catalogue::context_window_for_model(model) as usize;
        // Scale linearly from the 32k baseline, clamped.
        let obs_count = ((window_tokens as f64 / 32_000.0) * 30.0).round() as usize;
        let obs_chars = ((window_tokens as f64 / 32_000.0) * 2_000.0).round() as usize;
        (obs_count.clamp(30, 200), obs_chars.clamp(2_000, 16_000))
    }

    /// Compute the dynamic archiving idle turn threshold based on the model's context window.
    pub fn stale_threshold_for_model(model: &str) -> i64 {
        let window_tokens = catalogue::context_window_for_model(model) as f64;
        let threshold = (window_tokens / 2500.0).round() as i64;
        threshold.clamp(15, 500)
    }

    /// Compute the dynamic decay idle turn threshold based on the model's context window.
    /// Typically scales proportionally with the archiving threshold.
    pub fn decay_threshold_for_model(model: &str) -> i64 {
        let archiving_threshold = Self::stale_threshold_for_model(model);
        (archiving_threshold / 4).max(3)
    }
}
/// Awareness footer appended to system prompt when any memory tier is present.
pub(crate) fn memory_awareness_footer(model: &str) -> String {
    let stale_threshold = MemoryBudgets::stale_threshold_for_model(model);
    format!(
        "\n\nMemory system: blocks idle for {}+ turns are \
         archived. The Archived Memory section above lists them with label + excerpt only. \
         To retrieve a full archived block, call the `search_memory` tool with a keyword — \
         matched blocks are automatically promoted back to active memory. \
         To search dropped conversation history, use the `conversation_search` tool. \
         To keep a critical block permanently active, ask the user to run `/memory pin <label>`.",
        stale_threshold
    )
}
/// Cap on a single tool-result content string (chars). ~2k tokens.
/// Prevents huge outputs (screenshots, logs) from blowing the context window.
/// 8 192 chars covers the vast majority of useful tool outputs (diffs, file
/// excerpts, command output) while cutting worst-case cost by 75% vs 32 768.
const TOOL_RESULT_MAX_CHARS: usize = 8_192;

/// Per-tool output limits (chars). Tools not listed here use TOOL_RESULT_MAX_CHARS.
/// Rationale:
///   - bash/shell: logs are noisy, 4k is usually enough for error context
///   - grep/search: results are compact summaries, 3k covers most searches
///   - read_file: files need more space to be useful, 12k allows full small files
///   - archival/conversation search: returns excerpts, 2k is ample
///
/// NOTE: wired into `db_row_to_llm` via the `tool_name` field stored in tool
/// result content. Older DB rows without `tool_name` fall back to the default.
pub(crate) fn tool_output_limit(tool_name: &str) -> usize {
    let base = {
        let mut b = tool_name;
        if let Some(pos) = b.find("__") {
            b = &b[pos + 2..];
        }
        match b {
            "RunShellCommand" => "bash",
            "ReadFileGemini" => "read_file",
            "WriteFileGemini" => "write_file",
            "Replace" => "edit_file",
            "SearchFileContent" => "grep",
            "GlobGemini" => "glob",
            _ => b,
        }
    };
    match base {
        // Shell / command execution
        "bash" | "shell" | "start_process" | "read_process_output" => 4_096,

        // File reading tools — need more room
        "read_file" | "read_multiple_files" => 12_288,

        // Search / grep — compact results
        "grep" | "grep_search" | "start_search" | "get_more_search_results" => 3_072,

        // Memory retrieval — excerpts only
        "archival_memory_search" | "conversation_search" | "search_memory" | "query_event_log" => {
            2_048
        }

        // Glob / list — compact
        "glob" | "list_directory" => 3_072,

        // Everything else: default
        _ => TOOL_RESULT_MAX_CHARS,
    }
}
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

/// Per-message hard cap applied to non-current messages before they enter the
/// turn-selection budget walk.  A single oversized tool result (e.g. a large
/// file dump or log paste) used to be injected verbatim because the
/// "most-recent turn always included" rule has no escape hatch — provider then
/// rejected the request with a context-length error.  Capping per-message
/// guarantees no individual message can wedge the session.
///
/// Set to ~30 000 chars (~10 k tokens at 3 chars/token).  Truncated messages
/// keep the head + tail and append a marker the agent can detect and recover
/// from via `archival_memory_search` / re-running the original tool call.
pub(crate) const PER_MESSAGE_CHAR_CAP: usize = 30_000;
/// Marker string appended when a message is truncated for context fit.
/// Stable text — agents and tests both grep for it.
pub(crate) const TRUNCATION_MARKER: &str = "\n\n[…truncated for context fit. Re-run the original tool or use archival_memory_search to retrieve the full output…]";

/// Total character budget for the "# Loaded Skills" section injected into the
/// dynamic system prompt.  Skill bodies vary wildly (5 K–50 K each); without a
/// cap, loading 3-4 large skills could swallow the entire memory section
/// before any history fits.
///
/// Skills are emitted in order: full body for the first skills that fit, then
/// summary-only entries (`## Skill: name (id) [summary-only — call load_skill
/// to inject full body]`) for the rest.  Agents can promote a summary to full
/// fidelity at any time via the `load_skill` tool.
pub(crate) const SKILLS_INJECTION_BUDGET: usize = 30_000;
/// Cap on a single skill body before it counts as "summary-only".  Bodies
/// larger than this are always rendered as summary entries even when budget is
/// available — keeps any one oversized skill from dominating the section.
pub(crate) const SKILL_BODY_INDIVIDUAL_CAP: usize = 12_000;

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
                    last_consolidation_turn: 0,
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
        match build_context(state.clone(), agent_id.clone(), conv_id.clone(), false).await {
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
    match complete_with_overflow_recovery(&state, &agent_id, conv_id_ref, false, req).await {
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
    let tool_name = tr["tool_name"].as_str().unwrap_or("").to_string();

    persist(
        state,
        agent_id,
        conv_id,
        "tool",
        json!({
            "content": content,
            "tool_call_id": call_id,
            "tool_name": tool_name
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

    let (model, messages, tools) = match build_context(
        state.clone(),
        agent_id.to_string(),
        conv_id.map(String::from),
        true,
    )
    .await
    {
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
    match complete_with_overflow_recovery(state, agent_id, conv_id, true, req).await {
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
                    last_consolidation_turn: 0,
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
                "tool_call_id": tr["tool_call_id"].as_str().unwrap_or(""),
                "tool_name": tr["tool_name"].as_str().unwrap_or("")
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
    let (model, mut messages, tools) = match build_context(
        state.clone(),
        agent_id.clone(),
        conv_id.clone(),
        is_tool_return,
    )
    .await
    {
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
                    Event::default()
                        .data(json!({ "message_type": "error", "error": err_msg }).to_string()),
                ),
                Ok::<Event, std::convert::Infallible>(Event::default().data("[DONE]")),
            ]);
            return Sse::new(s).into_response();
        }
    };

    let acc = std::sync::Arc::new(parking_lot::Mutex::new((
        String::new(),
        Vec::<Value>::new(),
        String::new(),
    )));
    let acc_clone = acc.clone();
    // Accumulate token usage across chunks
    let usage_acc = std::sync::Arc::new(parking_lot::Mutex::new(TokenUsage::default()));
    let usage_acc2 = usage_acc.clone();
    let usage_acc3 = usage_acc.clone();

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
                    {
                        let mut g = acc_clone.lock();
                        g.2.push_str(&text);
                    }
                    emit(json!({ "message_type": "reasoning_message", "reasoning": text }))
                }
                Ok(StreamChunk::Text(text)) => {
                    {
                        let mut g = acc_clone.lock();
                        g.0.push_str(&text);
                    }
                    emit(json!({ "message_type": "assistant_message", "content": text }))
                }
                Ok(StreamChunk::ToolCall(tc)) => {
                    if let Ok(v) = serde_json::to_value(&tc) {
                        acc_clone.lock().1.push(v);
                    }
                    emit(json!({
                        "message_type": "tool_call_message",
                        "tool_call": { "id": tc.id, "name": tc.name, "arguments": tc.arguments }
                    }))
                }
                Ok(StreamChunk::Usage(u)) => {
                    {
                        let mut acc = usage_acc2.lock();
                        acc.input_tokens = acc.input_tokens.max(u.input_tokens);
                        acc.output_tokens = acc.output_tokens.max(u.output_tokens);
                        acc.cache_read_tokens = acc.cache_read_tokens.max(u.cache_read_tokens);
                        acc.cache_write_tokens = acc.cache_write_tokens.max(u.cache_write_tokens);
                        acc.model = u.model.clone();
                    }
                    Event::default().comment("usage_updated")
                }
                Ok(StreamChunk::FinishReason(reason)) => emit(json!({
                    "message_type": "finish_reason",
                    "reason": reason,
                })),
                Ok(StreamChunk::Done) => {
                    {
                        let g = acc_clone.lock();
                        // Skip persisting empty assistant responses — they clutter
                        // the conversation and produce invalid turn ordering on
                        // next context load (e.g. Gemini consecutive-user-turn 400).
                        if !g.0.is_empty() || !g.1.is_empty() || !g.2.is_empty() {
                            let mut content = g.0.clone();
                            if !g.2.is_empty() {
                                content =
                                    format!("<reasoning>\n{}\n</reasoning>\n\n{}", g.2, content);
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
                    // P2: flush accumulated token usage into AgentMetrics so
                    // server-side cost dashboards / future cost guardrails see
                    // cache_read + cache_write tokens (previously dropped).
                    {
                        let u = usage_acc3.lock();
                        let snap = u.clone();
                        let agent_metrics = state_clone.agent_metrics.clone();
                        let agent_id_for_metrics = agent_id_clone.clone();
                        tokio::spawn(async move {
                            let map = agent_metrics;
                            map.entry(agent_id_for_metrics)
                                .or_default()
                                .accumulate_usage(&snap);
                        });
                    }
                    let u = usage_acc3.lock();
                    let snap = u.clone();
                    emit(json!({
                        "message_type":      "usage_statistics",
                        "input_tokens":      snap.input_tokens,
                        "output_tokens":     snap.output_tokens,
                        "cache_read_tokens":  snap.cache_read_tokens,
                        "cache_write_tokens": snap.cache_write_tokens,
                        "model":             snap.model,
                    }))
                }
                Err(e) => {
                    if let Some(rid) = &run_id_clone {
                        let _ = sqlite::finish_run(&db_clone, rid, "failed");
                    }
                    Event::default().data(
                        json!({ "message_type": "error", "error": e.to_string() }).to_string(),
                    )
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
pub(crate) fn maybe_set_conv_title(state: &AppState, conv_id: &str, text: &str) {
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
