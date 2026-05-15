//! `POST /v1/agents/:id/run` — server-side agentic loop.
//!
//! Unlike `/messages/stream` (which fires a single LLM call and expects the
//! client to execute tools and POST results back), this endpoint runs the
//! full multi-turn loop entirely on the server:
//!
//!   1. Persist the user message.
//!   2. Build context → call LLM → stream tokens to the client.
//!   3. If the LLM emits tool calls, execute them (native + MCP) and persist
//!      the results.
//!   4. Rebuild context → call LLM again → stream — repeat until
//!      `finish_reason` is not `"tool_use"` or the turn cap is reached.
//!
//! The client receives a single continuous SSE stream.  All tool_call and
//! tool_result events are included so the GUI can render them inline.
//!
//! ## Request body
//! ```json
//! { "input": "…", "conversation_id": "…" }
//! ```
//!
//! ## SSE event shapes (identical to `/messages/stream`)
//! ```text
//! {"message_type":"stream_start","conversation_id":"…","run_id":"…"}
//! {"message_type":"assistant_message","content":"…"}
//! {"message_type":"reasoning_message","reasoning":"…"}
//! {"message_type":"tool_call_message","tool_call":{"id":"…","name":"…","arguments":"…"}}
//! {"message_type":"tool_result_message","tool_result":{"id":"…","name":"…","output":"…","is_error":false}}
//! {"message_type":"usage_statistics","input_tokens":N,"output_tokens":N,"model":"…"}
//! {"message_type":"finish_reason","reason":"end_turn"}
//! [DONE]
//! ```

use axum::{
    Json,
    extract::{Path, State},
    response::{IntoResponse, Response, Sse, sse::Event},
};
use cade_ai::{CompletionRequest, LlmToolCall, StreamChunk, catalogue};
use cade_store::sqlite;
use futures::StreamExt;
use serde_json::{Value, json};

use super::messages::{build_context, err, maybe_set_conv_title, persist, resolve_conversation};
use crate::server::state::AppState;

pub mod storage_impl;
/// Maximum agentic turns per request (prevents infinite loops).
mod subagent;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

const MAX_TURNS: usize = 20;

/// Maximum bytes of a tool's `output` to send over SSE before truncation.
/// The full output is still persisted to the DB so future turns see complete
/// history; only the SSE payload is capped to keep the GUI responsive.
pub(super) const SSE_OUTPUT_TRUNCATE_BYTES: usize = 8_192;

/// M9r: status the agentic run exited with.  Stored in the `runs.status`
/// column so audit / observability can distinguish a clean termination
/// (`"done"`) from an aborted one (`"error"` — `MAX_TURNS` exceeded, cost
/// cap hit, build_context error, LLM stream error, context-overflow
/// retry exhausted, etc.) or a client-initiated cancel (`"cancelled"` —
/// the SSE channel closed because the user pressed Ctrl+C or the client
/// process exited).  Pure value type; no I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunExitStatus {
    Done,
    Error,
    Cancelled,
}

impl RunExitStatus {
    fn as_str(self) -> &'static str {
        match self {
            RunExitStatus::Done => "done",
            RunExitStatus::Error => "error",
            RunExitStatus::Cancelled => "cancelled",
        }
    }
}

/// Truncate a string at a UTF-8 char boundary at or below `max_bytes`.
///
/// `String::len()` is in bytes, but slicing with `s[..n]` panics if `n` is
/// not on a char boundary.  This helper walks back to the previous char
/// boundary so multi-byte UTF-8 (CJK, emoji, accented Latin) never causes
/// a panic in tool-output truncation.  Returns the original string if it
/// is already shorter than the limit.
pub(super) fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // Walk backwards from `max_bytes` until we land on a char boundary.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// P4: parse a `CADE_MAX_SESSION_COST_USD`-style env value into an optional
/// cap.  Pure function for testing; the production wrapper [`max_session_cost_usd`]
/// reads the live env var and delegates to this.
pub(super) fn parse_max_session_cost(raw: Option<&str>) -> Option<f64> {
    raw.and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| *v > 0.0)
}

/// P4: read `CADE_MAX_SESSION_COST_USD` env var.
///
/// When set to a positive number, the agentic loop aborts as soon as the
/// agent's cumulative cost (across the server's lifetime, computed via
/// `AgentMetrics::compute_cost_usd`) exceeds this value.  Unset, empty, or
/// non-positive values disable the guardrail entirely.
fn max_session_cost_usd() -> Option<f64> {
    parse_max_session_cost(std::env::var("CADE_MAX_SESSION_COST_USD").ok().as_deref())
}

/// P4: shared `ModelRegistry` used to price token totals against the
/// bundled / user-customised pricing rules.  Loaded once at first call;
/// subsequent calls reuse the same instance.
fn pricing_registry() -> &'static cade_ai::ModelRegistry {
    use std::sync::OnceLock;
    static REG: OnceLock<cade_ai::ModelRegistry> = OnceLock::new();
    REG.get_or_init(|| {
        let path = dirs::home_dir().map(|h| h.join(".cade").join("pricing.json"));
        cade_ai::ModelRegistry::load_or_default(path.as_deref())
    })
}

/// P7: lookup the agent's current model id for pricing.  Returns empty
/// string on lookup failure so `pricing_for_model` falls back to the
/// zero default (= no guardrail trigger, fail-open).
async fn model_for_pricing(db: &cade_store::sqlite::Db, agent_id: &str) -> String {
    cade_store::sqlite::agents::get_agent(db, agent_id)
        .ok()
        .flatten()
        .map(|r| r.model)
        .unwrap_or_default()
}

/// P6: parse a `CADE_TOOL_TURN_MAX_TOKENS`-style env value into an optional
/// cap.  Pure function for testability.
///
/// `None`, empty, zero, or non-numeric input → `None` (= no cap, use the
/// model's full max_tokens).  Positive values are returned as-is so callers
/// can `.min()` against the model's hard cap.
pub(super) fn parse_tool_turn_max_tokens(raw: Option<&str>) -> Option<u32> {
    raw.and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|v| *v > 0)
}

/// P6: read `CADE_TOOL_TURN_MAX_TOKENS` env var.
///
/// When set, all agentic-loop iterations *except the first* (= tool-dispatch
/// turns following a `tool_result`) cap their output at this many tokens.
/// First turns and final-answer turns receive the model's full
/// `max_tokens_for_model` budget.  Verbose models can spend 2-4× more output
/// tokens explaining tool selection than they need to; capping the
/// tool-dispatch-only turns saves output cost without losing quality on the
/// answer turns.
fn tool_turn_max_tokens() -> Option<u32> {
    parse_tool_turn_max_tokens(std::env::var("CADE_TOOL_TURN_MAX_TOKENS").ok().as_deref())
}

// ── Pre-spawn helpers ─────────────────────────────────────────────────────

/// Record that the agent is active and update its conversation pointer.
async fn update_activity(state: &AppState, agent_id: &str, conv_id: Option<String>) {
    let mut activity = state.agent_activity.write().await;
    let entry =
        activity
            .entry(agent_id.to_owned())
            .or_insert(crate::server::state::AgentActivity {
                last_active_ts: 0,
                needs_consolidation: false,
                conversation_id: conv_id.clone(),
                last_consolidation_turn: 0,
            });
    entry.last_active_ts = chrono::Utc::now().timestamp();
    entry.conversation_id = conv_id;
}

/// Extract and validate the `input` field from the request body.
fn parse_input(body: &Value) -> Result<String, Response> {
    body["input"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| err(axum::http::StatusCode::BAD_REQUEST, "missing 'input'"))
}

/// Return the theme name when the input is a `/theme <name>` command.
fn detect_theme_cmd(input: &str) -> Option<String> {
    input.strip_prefix("/theme ").map(|s| s.trim().to_string())
}

/// Create a run record in the DB, falling back to a timestamp-based local ID
/// if the DB write fails.
fn make_run_id(state: &AppState, agent_id: &str, conv_str: Option<&str>) -> String {
    sqlite::create_run(&state.db, agent_id, conv_str)
        .map(|r| r.id)
        .unwrap_or_else(|_| format!("run-local-{}", chrono::Utc::now().timestamp()))
}

/// `POST /v1/agents/:id/run`
pub async fn run_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    // ── Resolve / create conversation ─────────────────────────────────────
    let conv_id: Option<String> = match resolve_conversation(&state, &agent_id, &body) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let conv_str = conv_id.clone();

    // ── Update activity ───────────────────────────────────────────────────
    update_activity(&state, &agent_id, conv_id.clone()).await;

    // ── Parse & persist user message ──────────────────────────────────────
    let input = match parse_input(&body) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let theme_cmd = detect_theme_cmd(&input);
    if theme_cmd.is_none() {
        if let Some(cid) = conv_str.as_deref() {
            maybe_set_conv_title(&state, cid, &input);
        }
        persist(
            &state,
            &agent_id,
            conv_str.as_deref(),
            "user",
            json!({ "content": input }),
        );
    }

    // ── Create run record ─────────────────────────────────────────────────
    let run_id = make_run_id(&state, &agent_id, conv_str.as_deref());

    // Snapshot for the async stream task
    let state2 = state.clone();
    let agent_id2 = agent_id.clone();
    let conv_id2 = conv_str.clone();
    let run_id2 = run_id.clone();

    // ── Build SSE stream ──────────────────────────────────────────────────
    // We use an mpsc channel to bridge the async loop into an SSE stream.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(128);

    tokio::spawn(run_agent_loop(
        state2, agent_id2, conv_id2, run_id2, theme_cmd, tx,
    ));

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(stream).into_response()
}

/// Type alias for the SSE sender used by [`run_agent_loop`].
pub(super) type SseTx = tokio::sync::mpsc::Sender<Result<Event, std::convert::Infallible>>;

/// Async body of [`run_agent`], extracted for readability.
///
/// Handles `/theme <name>` commands and the full multi-turn agentic loop.
/// All SSE events are sent via `tx`; the caller owns the receiver side.
async fn run_agent_loop(
    state2: AppState,
    agent_id2: String,
    conv_id2: Option<String>,
    run_id2: String,
    theme_cmd: Option<String>,
    tx: SseTx,
) {
    let send = |data: Value| {
        let tx = tx.clone();
        let ev = Event::default().data(data.to_string());
        async move {
            let _ = tx.send(Ok(ev)).await;
        }
    };

    // ── stream_start ──────────────────────────────────────────────────
    send(json!({
        "message_type": "stream_start",
        "conversation_id": conv_id2,
        "run_id": run_id2,
    }))
    .await;

    if let Some(t_name) = theme_cmd {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let agent_dir = dirs::home_dir()
            .map(|h| h.join(".cade"))
            .unwrap_or_else(|| std::path::PathBuf::from(".cade"));

        // `/theme reload` — re-resolve the agent's persisted theme from
        // disk.  Useful after editing a JSON/tmTheme file: the user
        // doesn't need to retype the name.  If no theme was persisted,
        // fall through to literal name resolution (which will fail
        // loudly with "not found").
        let effective_name = if t_name == "reload" {
            cade_store::sqlite::agents::get_agent(&state2.db, &agent_id2)
                .ok()
                .flatten()
                .and_then(|row| row.theme)
                .unwrap_or_else(|| t_name.clone())
        } else {
            t_name.clone()
        };

        // Resolution order: built-in registry first, then on-disk themes.
        let colors_opt = cade_core::resources::get_theme(&effective_name)
            .or_else(|| {
                let all = cade_core::resources::discover_themes(&cwd, &agent_dir);
                all.into_iter().find(|t| t.meta.name == effective_name)
            })
            .or_else(|| {
                let name_lower = effective_name.to_lowercase();
                let builtins = cade_core::resources::list_available_themes();
                if let Some(bn) = builtins.iter().find(|n| {
                    n.name.to_lowercase().contains(&name_lower)
                        || n.display_name.to_lowercase().contains(&name_lower)
                }) {
                    cade_core::resources::get_theme(&bn.name)
                } else {
                    let all = cade_core::resources::discover_themes(&cwd, &agent_dir);
                    all.into_iter()
                        .find(|t| t.meta.name.to_lowercase().contains(&name_lower))
                }
            });

        if let Some(colors) = colors_opt {
            // Persist the chosen theme on the agent row so GUI reloads
            // restore it automatically.  (Skip persist for `reload`
            // when the lookup already returned the persisted name —
            // writing the same value back is a no-op but clutters
            // audit trails; check string equality to avoid it.)
            let true_name = colors.meta.name.clone();
            if true_name != t_name || t_name != "reload" {
                let _ = cade_store::sqlite::agents::update_agent_theme(
                    &state2.db,
                    &agent_id2,
                    Some(&true_name),
                );
            }

            send(json!({
                "message_type": "theme_update",
                "theme_name": true_name,
            }))
            .await;
        } else {
            let all_themes = cade_core::resources::discover_themes(&cwd, &agent_dir);
            let builtins = cade_core::resources::list_available_themes();
            let mut available: Vec<String> = builtins.into_iter().map(|b| b.name).collect();
            available.extend(all_themes.into_iter().map(|t| t.meta.name));
            send(json!({
                "message_type": "assistant_message",
                "content": format!("Theme '{}' not found. Available themes: {}", t_name, available.join(", ")),
            })).await;
        }

        let _ = sqlite::finish_run(&state2.db, &run_id2, "done");
        let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
        return;
    }

    let mut turns = 0usize;
    // M9r: track the loop exit reason so `finish_run` records the right
    // status.  Any break preceded by an `"message_type": "error"` SSE
    // event flips this to `Error`; the natural "no more tool calls"
    // termination keeps `Done`.
    let mut exit_status = RunExitStatus::Done;

    // A5: Track tool calls since last active_goal update.
    // When this exceeds the threshold, inject a system nudge into the
    // next LLM turn so the agent remembers to update its working state.
    const ACTIVE_GOAL_NUDGE_INTERVAL: usize = 5;
    let mut tool_calls_since_goal_update: usize = 0;

    loop {
        turns += 1;
        if turns > MAX_TURNS {
            send(json!({
                "message_type": "error",
                "error": format!("Agentic loop exceeded {MAX_TURNS} turns — stopping"),
            }))
            .await;
            exit_status = RunExitStatus::Error;
            break;
        }

        // ── Client disconnect check ───────────────────────────────────
        // If the SSE receiver has been dropped (user pressed Ctrl+C in the
        // TUI, the cade CLI exited, or the network connection died), bail
        // out before doing more work. This prevents the server from
        // continuing to call the LLM and execute tools after the client
        // has gone away — saving tokens and avoiding unwanted side
        // effects from in-flight tool calls.
        if tx.is_closed() {
            tracing::info!("agentic loop: client disconnected at turn {turns} — cancelling");
            exit_status = RunExitStatus::Cancelled;
            break;
        }

        // ── P4: cost guardrail ────────────────────────────────────────
        // Abort when cumulative session cost (across the server's lifetime
        // for this agent) exceeds the configured cap.  Disabled by default
        // (unset env var → no cap).  Pricing comes from ~/.cade/pricing.json
        // or the bundled fallback table.
        if let Some(cap) = max_session_cost_usd() {
            let map = state2.agent_metrics.read().await;
            if let Some(m) = map.get(&agent_id2) {
                let pricing = pricing_registry()
                    .pricing_for_model(&model_for_pricing(&state2.db, &agent_id2).await);
                let cost = m.compute_cost_usd(&pricing);
                if cost >= cap {
                    send(json!({
                        "message_type": "error",
                        "error": format!(
                            "Session cost cap reached (${:.4} ≥ ${:.4}); set CADE_MAX_SESSION_COST_USD to a higher value to continue.",
                            cost, cap
                        ),
                    })).await;
                    exit_status = RunExitStatus::Error;
                    break;
                }
            }
        }

        // ── Build context ─────────────────────────────────────────────
        // Fix: only increment the turn counter on the first iteration
        // (the actual user message). Subsequent iterations are tool-return
        // re-invocations — they should read, not advance, the staleness clock.
        let is_tool_return = turns > 1;
        // Box::pin: build_context's compiled Future holds ~600 lines of
        // locals (Vec<Vec<LlmMessage>>, HashMap, multiple Strings, etc.)
        // across 23 await points.  Without boxing, this state machine is
        // embedded in run_agent_loop's Future, which combined with the
        // consolidation + LLM streaming futures overflows the tokio worker
        // thread stack when processing large archival/historic queries.
        let (model, messages, tools) = match Box::pin(build_context(
            &state2,
            &agent_id2,
            conv_id2.as_deref(),
            is_tool_return,
        ))
        .await
        {
            Ok(ctx) => ctx,
            Err(e) => {
                send(json!({ "message_type": "error", "error": e })).await;
                exit_status = RunExitStatus::Error;
                break;
            }
        };

        let max_tokens_cap = catalogue::max_tokens_for_model(&model);
        // P6: on iterations after the first (= tool-dispatch turns
        // continuing the loop after a tool_result), apply the optional
        // CADE_TOOL_TURN_MAX_TOKENS cap.  First turn and turns where the
        // model returns no further tool_calls (final answer) get full budget.
        let max_tokens = if turns > 1
            && let Some(tool_cap) = tool_turn_max_tokens()
        {
            tool_cap.min(max_tokens_cap)
        } else {
            max_tokens_cap
        };
        let req = CompletionRequest {
            model: model.clone(),
            messages,
            tools,
            max_tokens,
            reasoning_effort: None,
        };

        // ── Stream LLM response ───────────────────────────────────────
        // First attempt; if the provider rejects with a context-overflow
        // error before any chunks arrive, run synchronous consolidation
        // and rebuild the context once, then retry exactly once.
        let stream_result = state2.llm.stream(&req).await;
        let mut llm_stream = match stream_result {
            Ok(s) => s,
            Err(e) if e.is_context_overflow() => {
                tracing::warn!(
                    "stream [{}]: context overflow ({}); consolidating and retrying once",
                    agent_id2,
                    e
                );
                // Phase 3: surface a user-visible toast so the user
                // knows their session was *automatically* recovered
                // rather than failing silently or with a cryptic
                // provider error.
                send(json!({
                    "message_type": "system_notice",
                    "level":        "warning",
                    "code":         "context_overflow_recovering",
                    "message":      "Context window full — compacting older turns and retrying…"
                }))
                .await;
                // Box::pin the consolidation future to move its large
                // state machine (~500 lines of locals) to the heap.
                // Without this, consolidate_agent's Future is embedded
                // in run_agent_loop's state machine, contributing to
                // the stack overflow on archival/historic content access.
                Box::pin(crate::server::consolidation::consolidate_agent(
                    &state2,
                    &agent_id2,
                    conv_id2.as_deref(),
                ))
                .await;
                // Drop cached context entry so build_context recomputes.
                {
                    let mut cache = state2.context_cache.lock();
                    let key = format!("{}:{:?}", agent_id2, conv_id2.as_deref());
                    cache.pop(&key);
                }
                // Box::pin the rebuild future — build_context's state
                // machine holds Vec<Vec<LlmMessage>>, Vec<MessageRow>,
                // multiple HashMaps, etc. Boxing moves them to the heap
                // and prevents the overflow recovery path from doubling
                // the stack pressure of the main build_context call.
                let (model2, mut messages2, tools2) = match Box::pin(build_context(
                    &state2,
                    &agent_id2,
                    conv_id2.as_deref(),
                    is_tool_return, // reuse — never double-increment on retry
                ))
                .await
                {
                    Ok(ctx) => ctx,
                    Err(build_err) => {
                        send(json!({ "message_type": "error", "error": build_err })).await;
                        exit_status = RunExitStatus::Error;
                        break;
                    }
                };
                // Belt-and-suspenders: drop the older half of trailing
                // (non-system) messages on retry.
                let split_idx = messages2
                    .iter()
                    .position(|m| m.role != "system")
                    .unwrap_or(messages2.len());
                let trail_len = messages2.len().saturating_sub(split_idx);
                if trail_len > 2 {
                    let drop_n = trail_len / 2;
                    messages2.drain(split_idx..split_idx + drop_n);
                }
                let retry_req = CompletionRequest {
                    model: model2,
                    messages: messages2,
                    tools: tools2,
                    max_tokens,
                    reasoning_effort: None,
                };
                match state2.llm.stream(&retry_req).await {
                    Ok(s) => {
                        // Phase 3: tell the user that the recovery
                        // worked and the conversation continues.
                        send(json!({
                            "message_type": "system_notice",
                            "level":        "info",
                            "code":         "context_overflow_recovered",
                            "message":      "Context recovered — older turns are now in session_summary."
                        })).await;
                        s
                    }
                    Err(e2) => {
                        send(json!({
                            "message_type": "error",
                            "error": format!("Context overflow persisted after consolidation: {e2}"),
                        }))
                        .await;
                        exit_status = RunExitStatus::Error;
                        break;
                    }
                }
            }
            Err(e) => {
                send(json!({ "message_type": "error", "error": e.to_string() })).await;
                exit_status = RunExitStatus::Error;
                break;
            }
        };

        let mut text_acc = String::new();
        let mut tool_calls: Vec<LlmToolCall> = Vec::new();
        // Race the LLM stream against tx.closed() so a mid-stream
        // disconnect (Ctrl+C in the TUI, client process exit, network
        // drop) aborts the LLM call instead of letting it run to
        // completion and silently bill tokens.
        let mut stream_cancelled = false;
        loop {
            tokio::select! {
                biased;
                _ = tx.closed() => {
                    tracing::info!(
                        "agentic loop: client disconnected mid-stream at turn {turns} — aborting LLM"
                    );
                    stream_cancelled = true;
                    break;
                }
                chunk_opt = llm_stream.next() => {
                    let Some(chunk) = chunk_opt else { break };
                    match chunk {
                        Ok(StreamChunk::Text(t)) => {
                            text_acc.push_str(&t);
                            send(json!({ "message_type": "assistant_message", "content": t })).await;
                        }
                        Ok(StreamChunk::Reasoning(r)) => {
                            send(json!({ "message_type": "reasoning_message", "reasoning": r })).await;
                        }
                        Ok(StreamChunk::ToolCall(tc)) => {
                            send(json!({
                                "message_type": "tool_call_message",
                                "tool_call": {
                                    "id": tc.id,
                                    "name": tc.name,
                                    "arguments": tc.arguments,
                                }
                            }))
                            .await;
                            tool_calls.push(tc);
                        }
                        Ok(StreamChunk::Usage(u)) => {
                            // P2: accumulate into AgentMetrics so cache tokens
                            // are not silently dropped server-side.
                            {
                                let mut map = state2.agent_metrics.write().await;
                                map.entry(agent_id2.clone())
                                    .or_default()
                                    .accumulate_usage(&u);
                            }
                            send(json!({
                                "message_type": "usage_statistics",
                                "input_tokens":  u.input_tokens,
                                "output_tokens": u.output_tokens,
                                "cache_read_tokens":  u.cache_read_tokens,
                                "cache_write_tokens": u.cache_write_tokens,
                                "model": u.model,
                            }))
                            .await;
                        }
                        Ok(StreamChunk::FinishReason(r)) => {
                            send(json!({ "message_type": "finish_reason", "reason": r })).await;
                        }
                        Err(e) => {
                            send(json!({ "message_type": "error", "error": e.to_string() })).await;
                        }
                        Ok(StreamChunk::Done) => {
                            // Stream ended cleanly (some providers emit Done before FinishReason)
                        }
                    }
                }
            }
        }
        if stream_cancelled {
            // Drop the stream explicitly so the underlying HTTP connection
            // to the LLM provider closes — this aborts upstream generation
            // and stops token billing.
            drop(llm_stream);
            exit_status = RunExitStatus::Cancelled;
            break;
        }

        // ── Persist assistant message ─────────────────────────────────
        let tool_calls_json: Vec<Value> = tool_calls
            .iter()
            .filter_map(|tc| serde_json::to_value(tc).ok())
            .collect();
        let has_text = !text_acc.is_empty();
        let has_tools = !tool_calls.is_empty();
        if has_text || has_tools {
            persist(
                &state2,
                &agent_id2,
                conv_id2.as_deref(),
                "assistant",
                json!({
                    "content": text_acc,
                    "tool_calls": tool_calls_json,
                }),
            );
        }

        // ── Done if no tool calls ──────────────────────────────────────
        if tool_calls.is_empty() {
            break;
        }

        // ── Execute tools and persist results ─────────────────────────
        // RC5-FIX: Hoist ToolRuntime creation outside per-tool-call loop.
        // One runtime instance is reused across all tool calls in this turn,
        // avoiding redundant Arc::new + AppState clones per tool call.
        let storage_backend = std::sync::Arc::new(storage_impl::ServerStorageBackend {
            state: state2.clone(),
        });
        let runtime = cade_agent::tools::runtime::ToolRuntime::new(
            storage_backend,
            std::sync::Arc::clone(&state2.mcp),
            agent_id2.clone(),
            std::env::current_dir().unwrap_or_default(),
        );
        for tc in &tool_calls {
            // For interactive tools (run_subagent, etc) that ToolRuntime doesn't handle,
            // we intercept them server-side using the remaining meta_tools logic or subagent dispatcher.
            let result = if tc.name == "run_subagent" {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
                subagent::handle_run_subagent_tool(&state2, &agent_id2, &tc.id, &args, tx.clone())
                    .await
            } else if tc.name == "run_parallel_subagents" {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
                subagent::handle_run_parallel_subagents_tool(
                    &state2,
                    &agent_id2,
                    &tc.id,
                    &args,
                    tx.clone(),
                )
                .await
            } else if tc.name == "cancel_subagent" {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
                subagent::handle_cancel_subagent_tool(&state2, &tc.id, &args).await
            } else if tc.name == "set_plan" {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
                let steps = args
                    .get("steps")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .enumerate()
                            .filter_map(|(i, v)| {
                                v.as_str().map(|s| {
                                    serde_json::json!({
                                        "id": i + 1,
                                        "description": s,
                                        "is_done": false
                                    })
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let n = steps.len();
                let plan_json =
                    serde_json::json!({ "steps": steps, "is_visible": true }).to_string();
                let _ = sqlite::agents::update_agent_active_plan(
                    &state2.db,
                    &agent_id2,
                    Some(&plan_json),
                );

                send(serde_json::json!({
                    "message_type": "plan_update",
                    "plan": serde_json::json!({ "steps": steps, "is_visible": true })
                }))
                .await;

                cade_agent::tools::manager::ToolResult {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    output: format!("Plan set with {n} step(s)."),
                    is_error: false,
                }
            } else if tc.name == "UpdatePlan" {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
                let step_id = args.get("step_id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let done = args.get("done").and_then(|v| v.as_bool()).unwrap_or(true);

                let mut found = false;
                if let Ok(Some(agent)) = sqlite::agents::get_agent(&state2.db, &agent_id2) {
                    if let Some(plan_str) = agent.active_plan_json {
                        if let Ok(mut plan) = serde_json::from_str::<serde_json::Value>(&plan_str) {
                            if let Some(steps) =
                                plan.get_mut("steps").and_then(|v| v.as_array_mut())
                            {
                                for step in steps.iter_mut() {
                                    if step.get("id").and_then(|id| id.as_u64())
                                        == Some(step_id as u64)
                                    {
                                        step["is_done"] = serde_json::json!(done);
                                        found = true;
                                        break;
                                    }
                                }
                            }
                            if found {
                                let _ = sqlite::agents::update_agent_active_plan(
                                    &state2.db,
                                    &agent_id2,
                                    Some(&plan.to_string()),
                                );
                                send(serde_json::json!({
                                    "message_type": "plan_update",
                                    "plan": plan
                                }))
                                .await;
                            }
                        }
                    }
                }

                cade_agent::tools::manager::ToolResult {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    output: if found {
                        format!(
                            "Step {step_id} marked {}.",
                            if done { "done" } else { "not done" }
                        )
                    } else {
                        format!("error: Step {step_id} not found in the active plan.")
                    },
                    is_error: !found,
                }
            } else if tc.name == "finish_task" {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.arguments.to_string()).unwrap_or_default();
                let summary = args
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let reason = args
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let output = std::process::Command::new("git")
                    .args(&["status", "--porcelain"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();

                let files_modified = if output.trim().is_empty() {
                    "None".to_string()
                } else {
                    output
                        .lines()
                        .map(|l| format!("- {}", l.trim()))
                        .collect::<Vec<_>>()
                        .join("\n")
                };

                let timestamp =
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

                let log_entry = format!(
                    "\n## {} — {}\n\n**Reason:** {}\n\n**Files modified:**\n{}\n\n---\n",
                    timestamp, summary, reason, files_modified
                );

                let path = std::path::Path::new("CADE_AUDIT.md");
                let existing = std::fs::read_to_string(path)
                    .unwrap_or_else(|_| "# CADE Audit Log\n\n".to_string());
                let _ = std::fs::write(path, format!("{}{}", existing, log_entry));

                cade_agent::tools::manager::ToolResult {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    output: format!("Task finished. Audit log appended to CADE_AUDIT.md."),
                    is_error: false,
                }
            } else if let Some(executed) = runtime
                .execute(tc.id.clone(), &tc.name, &tc.arguments)
                .await
            {
                cade_agent::tools::manager::ToolResult {
                    tool_call_id: executed.tool_call_id,
                    tool_name: executed.tool_name,
                    output: executed.output,
                    is_error: executed.is_error,
                }
            } else {
                // Any other interactive tools that reach here but shouldn't
                cade_agent::tools::manager::ToolResult {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    output: format!(
                        "Tool '{}' requires interactive TUI context and is not supported in background server loop.",
                        tc.name
                    ),
                    is_error: true,
                }
            };

            // H3: persist the FULL output to the DB so future build_context
            // calls feed complete tool results back to the LLM.  Only the
            // SSE payload is truncated for GUI responsiveness.
            //
            // C2: truncate at a UTF-8 char boundary, never at a raw byte
            // index — multi-byte chars (emoji, CJK, accented Latin) at the
            // boundary would otherwise panic.
            let output_for_sse = if result.output.len() > SSE_OUTPUT_TRUNCATE_BYTES {
                let head = truncate_at_char_boundary(&result.output, SSE_OUTPUT_TRUNCATE_BYTES);
                format!("{}\n[... truncated: {} bytes]", head, result.output.len())
            } else {
                result.output.clone()
            };

            // Stream the (possibly truncated) result to the GUI
            send(json!({
                "message_type": "tool_result_message",
                "tool_result": {
                    "id":       result.tool_call_id,
                    "name":     result.tool_name,
                    "output":   output_for_sse,
                    "is_error": result.is_error,
                }
            }))
            .await;

            // Persist the FULL output into DB so next build_context sees it.
            persist(
                &state2,
                &agent_id2,
                conv_id2.as_deref(),
                "tool",
                json!({
                    "content":      result.output,
                    "tool_call_id": result.tool_call_id,
                    "tool_name":    result.tool_name,
                }),
            );

            // ── P1: Record observation for this tool call ─────────────────
            // Summarise the tool invocation into a lightweight observation so
            // the context builder can inject a compressed trail of past actions
            // even after the original messages have been dropped.
            {
                let turn = sqlite::get_turn_counter(&state2.db, &agent_id2).unwrap_or(0);
                let summary =
                    build_observation_summary(&result.tool_name, &tc.arguments, &result.output);
                let importance = rate_observation_importance(&result.tool_name, result.is_error);
                let files = extract_file_paths(&tc.arguments);
                let _ = sqlite::observations::insert_observation(
                    &state2.db,
                    &agent_id2,
                    turn,
                    &result.tool_name,
                    "tool_call",
                    &summary,
                    &files,
                    "[]",
                    importance,
                );
            }

            // ── A5: Track active_goal freshness ───────────────────────────
            tool_calls_since_goal_update += 1;
            if tc.name == "update_memory" || tc.name == "memory_apply_patch" {
                let label = tc.arguments["label"].as_str().unwrap_or("");
                if label == "active_goal" {
                    tool_calls_since_goal_update = 0;
                }
            }
        }

        // ── A5: Inject freshness nudge if active_goal hasn't been updated ──
        if tool_calls_since_goal_update >= ACTIVE_GOAL_NUDGE_INTERVAL {
            let nudge = format!(
                "⚠️ Your active_goal memory block has not been updated in {} tool calls. \
                 Update it now with your current task, status, and next steps to prevent context loss.",
                tool_calls_since_goal_update
            );
            persist(
                &state2,
                &agent_id2,
                conv_id2.as_deref(),
                "system",
                json!({ "content": nudge }),
            );
        }

        // Loop → re-invoke LLM with tool results
    }

    let _ = sqlite::finish_run(&state2.db, &run_id2, exit_status.as_str());

    // ── End of stream ─────────────────────────────────────────────────
    let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
}

pub(super) fn record_recent_edit_db(db: &cade_store::sqlite::Db, agent_id: &str, path: &str) {
    let label = "recent_edits";
    let target_line = format!("Recently edited: {path}");

    let existing = cade_store::sqlite::get_memory_blocks(db, agent_id)
        .ok()
        .unwrap_or_default();
    let current_value = existing
        .iter()
        .find(|(l, _, _)| l == label)
        .map(|(_, v, _)| v.as_str())
        .unwrap_or("");

    let mut lines: Vec<String> = current_value.lines().map(String::from).collect();

    // Remove any existing identical "Recently edited:" lines (dedup)
    lines.retain(|l| l != &target_line);
    lines.push(target_line);

    // Keep only the last 10 "Recently edited:" entries
    let mut edit_indices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("Recently edited:"))
        .map(|(i, _)| i)
        .collect();
    while edit_indices.len() > 10 {
        let oldest_idx = edit_indices.remove(0);
        lines.remove(oldest_idx);
        for idx in edit_indices.iter_mut() {
            *idx -= 1;
        }
    }

    let new_value = lines.join("\n");
    if let Err(e) =
        cade_store::sqlite::upsert_memory_block(db, agent_id, label, &new_value, None, Some(2000))
    {
        tracing::warn!("record_recent_edit_db failed for agent={agent_id} path={path}: {e}");
    }
}

// ── P1: Observation helpers ──────────────────────────────────────────────────

/// Build a one-line summary of a tool call for observation storage.
///
/// Extracts the most informative argument (path, command, query) and
/// truncates the output to a short excerpt.
pub(super) fn build_observation_summary(
    tool_name: &str,
    arguments: &serde_json::Value,
    output: &str,
) -> String {
    let key_arg = arguments
        .get("path")
        .or_else(|| arguments.get("command"))
        .or_else(|| arguments.get("query"))
        .or_else(|| arguments.get("pattern"))
        .or_else(|| arguments.get("old_string"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let key_excerpt = if key_arg.len() > 80 {
        format!(
            "{}…",
            &key_arg[..key_arg
                .char_indices()
                .take(77)
                .last()
                .map(|(i, _)| i)
                .unwrap_or(77)]
        )
    } else {
        key_arg.to_string()
    };

    let output_head = if output.len() > 60 {
        let end = output
            .char_indices()
            .take(57)
            .last()
            .map(|(i, _)| i)
            .unwrap_or(57);
        format!("{}…", &output[..end])
    } else {
        output.to_string()
    };
    // Collapse newlines for compact storage
    let output_head = output_head.replace('\n', " ");

    if key_excerpt.is_empty() {
        format!("{tool_name} → {output_head}")
    } else {
        format!("{tool_name}({key_excerpt}) → {output_head}")
    }
}

/// Rate observation importance (1=routine, 5=critical).
///
/// - Errors always get 5
/// - File writes/edits get 4
/// - Builds/tests get 4
/// - File reads get 2
/// - Everything else gets 3
pub(super) fn rate_observation_importance(tool_name: &str, is_error: bool) -> i64 {
    if is_error {
        return 5;
    }
    match tool_name {
        n if n.contains("write") || n.contains("edit") || n.contains("replace") => 4,
        n if n.contains("bash") || n.contains("shell") || n.contains("test") => 4,
        n if n.contains("commit") || n.contains("push") => 5,
        n if n.contains("read") || n.contains("glob") || n.contains("grep") => 2,
        n if n.contains("search") => 2,
        _ => 3,
    }
}

/// Extract file paths from tool arguments as a JSON array string.
pub(super) fn extract_file_paths(arguments: &serde_json::Value) -> String {
    let mut paths = Vec::new();
    if let Some(p) = arguments.get("path").and_then(|v| v.as_str()) {
        paths.push(p.to_string());
    }
    if let Some(p) = arguments.get("source").and_then(|v| v.as_str()) {
        paths.push(p.to_string());
    }
    if let Some(p) = arguments.get("destination").and_then(|v| v.as_str()) {
        paths.push(p.to_string());
    }
    serde_json::to_string(&paths).unwrap_or_else(|_| "[]".to_string())
}
