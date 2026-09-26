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
//!      `finish_reason` is not `"tool_use"` or the adaptive turn budget is
//!      exhausted (base `MAX_TURNS`, escalated by distinct tool work, see
//!      [`adaptive_turn_budget`]).
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

use super::messages::{err, maybe_set_conv_title, persist, resolve_conversation};
use crate::server::state::AppState;

pub mod runtime;
pub mod storage_impl;
/// Maximum agentic turns per request (prevents infinite loops).
mod subagent;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// Default maximum agentic turns per request (prevents infinite loops).
/// Overridable via the `CADE_MAX_TURNS` env var (see [`max_turns`]).
const MAX_TURNS: usize = 20;

/// Maximum consecutive identical tool invocations (same tool + same
/// arguments) before the agentic loop is classified as degenerate and
/// aborted.  A model that keeps retrying the exact same call instead of
/// converging would otherwise burn turns and tokens all the way to
/// [`MAX_TURNS`].
const MAX_IDENTICAL_REPEAT_TOOL_CALLS: usize = 3;

/// Default cap on the *adaptive* turn budget, expressed as a multiple of
/// [`MAX_TURNS`].  The per-run budget starts at the base cap and grows with
/// genuinely distinct tool work (see [`adaptive_turn_budget`]); this bound
/// prevents a productive-but-unbounded loop from running away.
const TURN_CEILING_MULTIPLIER: usize = 5;

/// Maximum bytes of a tool's `output` to send over SSE before truncation.
mod execution;
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

/// Resolve the session cost cap (in USD).
///
/// Precedence: `CADE_MAX_SESSION_COST_USD` env var > `.cade/settings.json`
/// (`max_session_cost_usd`, project wins over global) > built-in default.
/// When no cap is configured anywhere, the guardrail defaults to `$120.00`.
fn max_session_cost_usd(settings_cap: Option<f64>) -> Option<f64> {
    parse_max_session_cost(std::env::var("CADE_MAX_SESSION_COST_USD").ok().as_deref())
        .or(settings_cap)
        .or(Some(120.0))
}

/// Parse a `CADE_MAX_TURNS`-style env value into an optional cap.
/// Pure function for testability.
///
/// `None`, empty, zero, or non-numeric input → `None` (= fall back to the
/// built-in [`MAX_TURNS`] default).  Positive values clamp the agentic
/// loop to that many LLM turns.
pub(super) fn parse_max_turns(raw: Option<&str>) -> Option<usize> {
    raw.and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
}

/// Read `CADE_MAX_TURNS` env var.
///
/// Sets the *base* agentic turn budget: the loop can always run at least
/// this many turns, and the adaptive mechanism ([`adaptive_turn_budget`])
/// extends it further while the run keeps making genuinely new tool calls.
/// Unset, empty, zero, or non-numeric values keep the [`MAX_TURNS`] default
/// of 20.
fn max_turns() -> usize {
    parse_max_turns(std::env::var("CADE_MAX_TURNS").ok().as_deref()).unwrap_or(MAX_TURNS)
}

/// Parse a `CADE_MAX_TURNS_CEILING`-style env value into an optional cap.
/// Pure function for testability.  Positive values clamp the adaptive turn
/// budget; anything else falls back to [`TURN_CEILING_MULTIPLIER`] × base.
pub(super) fn parse_max_turns_ceiling(raw: Option<&str>) -> Option<usize> {
    raw.and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
}

/// Resolve the absolute ceiling for the adaptive turn budget.
///
/// `CADE_MAX_TURNS_CEILING` overrides the default of
/// [`TURN_CEILING_MULTIPLIER`] × the base budget, so a run can never exceed
/// this many LLM turns.
fn turns_ceiling(base: usize) -> usize {
    parse_max_turns_ceiling(std::env::var("CADE_MAX_TURNS_CEILING").ok().as_deref())
        .unwrap_or_else(|| base.saturating_mul(TURN_CEILING_MULTIPLIER))
}

/// Compute the adaptive turn budget from the number of *distinct* tool
/// calls performed so far.  Pure function for testability.
///
/// Each genuinely new tool fingerprint (tool + arguments) earns 2 extra
/// turns of headroom so legitimate long tasks aren't cut at the base cap,
/// but the budget is clamped to `[base, ceiling]`.  A model that stops
/// producing new work stops earning headroom — repeated or cycling calls
/// are handled by the degenerate-loop detector instead of extending the run.
pub(super) fn adaptive_turn_budget(
    base: usize,
    ceiling: usize,
    distinct_tool_calls: usize,
) -> usize {
    base.saturating_add(distinct_tool_calls.saturating_mul(2))
        .min(ceiling)
        .max(base)
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
async fn model_for_pricing(db: &cade_store::sqlite::Db, agent_id: String) -> String {
    cade_store::sqlite::agents::get_agent(db, &agent_id)
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
    // Default to a safe limit of 4096 tokens on intermediate tool-planning turns if unset.
    parse_tool_turn_max_tokens(std::env::var("CADE_TOOL_TURN_MAX_TOKENS").ok().as_deref())
        .or(Some(4096))
}

// ── Request helpers ───────────────────────────────────────────────────────

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

/// Persist an ordered run event before forwarding it to the active transport.
async fn emit_run_event(db: &sqlite::Db, run_id: &str, tx: &SseTx, mut payload: Value) {
    let serialized = payload.to_string();
    let sequence = match sqlite::append_run_event(db, run_id, &serialized) {
        Ok(sequence) => sequence,
        Err(error) => {
            tracing::error!(%run_id, %error, "failed to persist run event");
            return;
        }
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert("run_id".to_owned(), Value::String(run_id.to_owned()));
        object.insert("seq_id".to_owned(), Value::from(sequence));
    }
    let _ = tx
        .send(Ok(runtime::RunEventEnvelope {
            data: payload.to_string(),
        }))
        .await;
}

/// `POST /v1/agents/:id/run`
pub async fn run_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let conversation_id = match resolve_conversation(&state, &agent_id, &body) {
        Ok(conversation_id) => conversation_id,
        Err(response) => return response,
    };
    let input = match parse_input(&body) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let permission_mode = body
        .get("permission_mode")
        .and_then(|v| v.as_str())
        .map(String::from);

    let runtime = runtime::ServerAgentRuntime::new(state);
    let handle = runtime
        .start(runtime::RunRequest {
            agent_id,
            conversation_id,
            input,
            permission_mode,
        })
        .await;

    tracing::debug!(run_id = %handle.run_id, "agent run accepted by server runtime");
    let stream = tokio_stream::StreamExt::map(
        tokio_stream::wrappers::ReceiverStream::new(handle.events),
        |res| res.map(Event::from),
    );
    // Keep the SSE transport alive during long quiet periods (model thinking,
    // long tool waits).  Nginx/proxies default to ~60 s read timeouts and will
    // tear down an idle stream, which the client surfaces as a body-stream
    // error.  Emit periodic SSE heartbeat comments the client ignores.
    Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new().interval(std::time::Duration::from_secs(15)),
        )
        .into_response()
}

/// Type alias for the SSE sender used by [`run_agent_loop`].
pub(super) type SseTx =
    tokio::sync::mpsc::Sender<Result<runtime::RunEventEnvelope, std::convert::Infallible>>;

/// Async body of [`run_agent`], extracted for readability.
///
/// Handles `/theme <name>` commands and the full multi-turn agentic loop.
/// All SSE events are sent via `tx`; the caller owns the receiver side.
pub(crate) async fn run_agent_loop_with_dependencies(
    state2: AppState,
    request: runtime::LoopRequest,
    tx: SseTx,
    context_builder: std::sync::Arc<dyn runtime::ContextBuilder>,
    capability_executor: std::sync::Arc<dyn runtime::CapabilityExecutor>,
) {
    let runtime::LoopRequest {
        agent_id: agent_id2,
        conversation_id: conv_id2,
        run_id: run_id2,
        theme_command: theme_cmd,
        input,
        permission_mode,
    } = request;
    let send_raw = |json_string: String| {
        let database = state2.db.clone();
        let run_id = run_id2.clone();
        let tx = tx.clone();
        async move {
            let payload = match serde_json::from_str(&json_string) {
                Ok(payload) => payload,
                Err(_) => Value::String(json_string),
            };
            emit_run_event(&database, &run_id, &tx, payload).await;
        }
    };

    let send = |data: Value| {
        let database = state2.db.clone();
        let run_id = run_id2.clone();
        let tx = tx.clone();
        async move {
            emit_run_event(&database, &run_id, &tx, data).await;
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
        crate::server::api::agents::publish_global_event(
            Some(&state2.db),
            "run_finished",
            json!({
                "run_id": run_id2,
                "agent_id": agent_id2,
                "status": "done",
            }),
        );
        emit_run_event(
            &state2.db,
            &run_id2,
            &tx,
            json!({ "message_type": "run_done", "status": "done" }),
        )
        .await;
        let _ = tx
            .send(Ok(runtime::RunEventEnvelope {
                data: "[DONE]".to_string(),
            }))
            .await;
        return;
    }

    let mut turns = 0usize;
    let max_turns = max_turns();
    let turns_ceiling = turns_ceiling(max_turns);
    // P4: resolve the session cost cap from `.cade/settings.json`
    // (`max_session_cost_usd`, project wins over global).  The env var
    // override and the built-in default are applied inside
    // [`max_session_cost_usd`].
    let cwd = std::env::current_dir().unwrap_or_default();
    let session_cost_cap = cade_core::settings::SettingsManager::new(&cwd)
        .ok()
        .and_then(|s| s.max_session_cost_usd());
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

    // ── Adaptive turn budget ─────────────────────────────────────────
    // The loop starts with `max_turns` and grows its budget as the model
    // performs genuinely *new* tool work (fresh fingerprints), so long but
    // productive tasks are not cut at the base cap while idle or repeating
    // runs are.  Bounded by `turns_ceiling`.
    let mut distinct_fingerprints: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut distinct_tool_calls: usize = 0;

    // ── Degenerate-loop detection ─────────────────────────────────────
    // Watch for the same tool being invoked repeatedly with identical
    // arguments.  When that happens `MAX_IDENTICAL_REPEAT_TOOL_CALLS`
    // times in a row, the model is stuck retrying rather than converging —
    // abort early instead of silently burning turns and tokens.
    let mut prev_tool_fingerprint: Option<String> = None;
    let mut identical_repeat_run: usize = 0;
    // Set while iterating tool results; surfaced as an SSE error before
    // the next LLM turn.
    let mut loop_degenerate: Option<String> = None;

    loop {
        turns += 1;
        let budget = adaptive_turn_budget(max_turns, turns_ceiling, distinct_tool_calls);
        if turns > budget {
            send(json!({
                "message_type": "error",
                "error": format!("Agentic loop exceeded {budget} turns (base {max_turns}) — stopping"),
            }))
            .await;
            exit_status = RunExitStatus::Error;
            break;
        }

        // ── Durable cancellation check ─────────────────────────────────
        // Presentation adapters may disconnect and reconnect from an event
        // cursor. Only the explicit durable cancellation command stops this
        // server-owned run; a transport receiver is not execution ownership.
        if sqlite::is_run_cancellation_requested(&state2.db, &run_id2).unwrap_or(false) {
            tracing::info!("agentic loop: cancellation requested at turn {turns}");
            exit_status = RunExitStatus::Cancelled;
            break;
        }

        // ── P4: cost guardrail ────────────────────────────────────────
        // Abort when cumulative session cost (across the server's lifetime
        // for this agent) exceeds the configured cap.  The cap comes from
        // `.cade/settings.json`, the CADE_MAX_SESSION_COST_USD env var, or
        // the built-in $120.00 default (see [`max_session_cost_usd`]).
        // Pricing comes from ~/.cade/pricing.json or the bundled fallback table.
        if let Some(cap) = max_session_cost_usd(session_cost_cap) {
            let map = state2.agent_metrics.clone();
            if let Some(m) = map.get(&agent_id2) {
                let pricing = pricing_registry()
                    .pricing_for_model(&model_for_pricing(&state2.db, agent_id2.clone()).await);
                let cost = m.compute_cost_usd(&pricing);
                if cost >= cap {
                    send(json!({
                        "message_type": "error",
                        "error": format!(
                            "Session cost cap reached (${:.4} ≥ ${:.4}); raise `max_session_cost_usd` in .cade/settings.json (or CADE_MAX_SESSION_COST_USD) to continue.",
                            cost, cap
                        ),
                    })).await;
                    exit_status = RunExitStatus::Error;
                    break;
                }
            }
        }

        // Deliver only outcomes from this conversation, before building the
        // next turn's context (also covers outcomes completed between runs).
        let pending_results = {
            let mut map = state2.pending_subagent_results.write().await;
            map.remove(&(agent_id2.clone(), conv_id2.clone()))
                .unwrap_or_default()
        };
        if !pending_results.is_empty() {
            let mut existing_ids: std::collections::HashSet<String> =
                cade_store::sqlite::list_messages(
                    &state2.db,
                    &agent_id2,
                    conv_id2.as_deref(),
                    10000,
                )
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m.role == "tool")
                .filter_map(|m| m.content["tool_call_id"].as_str().map(String::from))
                .collect();

            for sr in pending_results {
                if !existing_ids.insert(sr.tool_call_id.clone()) {
                    continue;
                }
                let body = format!(
                    "[background subagent {} {}]\n{}",
                    sr.subagent_id,
                    if sr.is_error { "failed" } else { "completed" },
                    sr.result
                );
                persist(
                    &state2,
                    &agent_id2,
                    conv_id2.as_deref(),
                    "tool",
                    json!({
                        "content": body,
                        "tool_call_id": sr.tool_call_id,
                        "tool_name": "run_subagent",
                    }),
                );
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
        let (model, messages, tools) = match Box::pin(context_builder.build(
            agent_id2.clone(),
            conv_id2.clone(),
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
                    state2.clone(),
                    agent_id2.clone(),
                    conv_id2.clone(),
                    None,
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
                let (model2, mut messages2, tools2) = match Box::pin(context_builder.build(
                    agent_id2.clone(),
                    conv_id2.clone(),
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
        let mut turn_usage = cade_ai::TokenUsage::default();
        // Race the LLM stream against tx.closed() so a mid-stream
        // disconnect (Ctrl+C in the TUI, client process exit, network
        // drop) aborts the LLM call instead of letting it run to
        // completion and silently bill tokens.
        let mut stream_cancelled = false;
        loop {
            tokio::select! {
                biased;
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                    if sqlite::is_run_cancellation_requested(&state2.db, &run_id2).unwrap_or(false) {
                        tracing::info!("agentic loop: cancellation requested mid-stream at turn {turns}");
                        stream_cancelled = true;
                        break;
                    }
                }
                chunk_opt = llm_stream.next() => {
                    let Some(chunk) = chunk_opt else { break };
                    match chunk {
                        Ok(StreamChunk::Text(t)) => {
                            text_acc.push_str(&t);
                            // Zero-allocation structured string building for high frequency streaming
                            // Using a pre-allocated format to avoid json! macro dynamic allocations
                            let json_str = format!("{{\"message_type\":\"assistant_message\",\"content\":{}}}", serde_json::to_string(&t).unwrap_or_default());
                            send_raw(json_str).await;
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
                            turn_usage.input_tokens = turn_usage.input_tokens.max(u.input_tokens);
                            turn_usage.output_tokens = turn_usage.output_tokens.max(u.output_tokens);
                            turn_usage.cache_read_tokens = turn_usage.cache_read_tokens.max(u.cache_read_tokens);
                            turn_usage.cache_write_tokens = turn_usage.cache_write_tokens.max(u.cache_write_tokens);
                            turn_usage.model = u.model.clone();
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

        // If provider or gateway omitted usage chunk, compute a fallback token estimate
        // so usage_statistics and costing are never silently lost.
        if turn_usage.input_tokens == 0 && turn_usage.output_tokens == 0 && (has_text || has_tools)
        {
            let est_out = ((text_acc.len()
                + tool_calls
                    .iter()
                    .map(|t| t.arguments.to_string().len())
                    .sum::<usize>())
                / 4)
            .max(1) as u32;
            let est_in =
                (req.messages.iter().map(|m| m.content.len()).sum::<usize>() / 4).max(1) as u32;
            turn_usage.input_tokens = est_in;
            turn_usage.output_tokens = est_out;
            if turn_usage.model.is_empty() {
                turn_usage.model = req.model.clone();
            }
        }

        if turn_usage.input_tokens > 0
            || turn_usage.output_tokens > 0
            || turn_usage.cache_read_tokens > 0
            || turn_usage.cache_write_tokens > 0
        {
            // P2: accumulate into AgentMetrics so cache tokens
            // are not silently dropped server-side.
            {
                let map = state2.agent_metrics.clone();
                map.entry(agent_id2.clone())
                    .or_default()
                    .accumulate_usage(&turn_usage);
            }
            send(json!({
                "message_type": "usage_statistics",
                "input_tokens":  turn_usage.input_tokens,
                "output_tokens": turn_usage.output_tokens,
                "cache_read_tokens":  turn_usage.cache_read_tokens,
                "cache_write_tokens": turn_usage.cache_write_tokens,
                "model": turn_usage.model,
            }))
            .await;
        }
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
        let turn_results = capability_executor
            .execute(
                runtime::TurnExecutionInput {
                    agent_id: agent_id2.clone(),
                    conversation_id: conv_id2.clone(),
                    run_id: run_id2.clone(),
                    input: input.clone(),
                    permission_mode: permission_mode.clone(),
                },
                tool_calls,
                tx.clone(),
            )
            .await;

        for (result, arguments) in turn_results {
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
                    build_observation_summary(&result.tool_name, &arguments, &result.output);
                let importance = rate_observation_importance(&result.tool_name, result.is_error);
                let files = extract_file_paths(&arguments);
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
            if result.tool_name == "update_memory" || result.tool_name == "memory_apply_patch" {
                let label = arguments["label"].as_str().unwrap_or("");
                if label == "active_goal" {
                    tool_calls_since_goal_update = 0;
                }
            }

            // ── Degenerate-loop fingerprint tracking ────────────────────
            // Fingerprint = tool name + normalized arguments (truncated so
            // large payloads don't bloat memory).  Identical consecutive
            // fingerprints mean the model is retrying the exact same call.
            let fingerprint = {
                let args = serde_json::to_string(&arguments).unwrap_or_default();
                format!(
                    "{}|{}",
                    result.tool_name,
                    truncate_at_char_boundary(&args, 256)
                )
            };
            // Adaptive budget: genuinely new fingerprints extend headroom.
            if distinct_fingerprints.insert(fingerprint.clone()) {
                distinct_tool_calls += 1;
            }
            if prev_tool_fingerprint.as_deref() == Some(fingerprint.as_str()) {
                identical_repeat_run += 1;
            } else {
                prev_tool_fingerprint = Some(fingerprint);
                identical_repeat_run = 1;
            }
            if identical_repeat_run >= MAX_IDENTICAL_REPEAT_TOOL_CALLS {
                loop_degenerate = Some(result.tool_name.clone());
            }
        }

        // ── Abort on a degenerate repeated-tool loop ────────────────────
        if let Some(tool_name) = loop_degenerate.take() {
            let msg = format!(
                "Agentic loop detected: `{tool_name}` invoked \
                 {MAX_IDENTICAL_REPEAT_TOOL_CALLS} times in a row with identical arguments \
                 — stopping instead of repeating"
            );
            tracing::warn!(agent_id = %agent_id2, run_id = %run_id2, "{msg}");
            send(json!({ "message_type": "error", "error": msg })).await;
            exit_status = RunExitStatus::Error;
            break;
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
    crate::server::api::agents::publish_global_event(
        Some(&state2.db),
        "run_finished",
        json!({
            "run_id": run_id2,
            "agent_id": agent_id2,
            "status": exit_status.as_str(),
        }),
    );

    // ── Durable terminal outcome ───────────────────────────────────────
    emit_run_event(
        &state2.db,
        &run_id2,
        &tx,
        json!({
            "message_type": "run_done",
            "status": exit_status.as_str(),
        }),
    )
    .await;

    // ── End of transport stream ────────────────────────────────────────
    let _ = tx
        .send(Ok(runtime::RunEventEnvelope {
            data: "[DONE]".to_string(),
        }))
        .await;
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

#[derive(serde::Deserialize)]
pub struct SteerPayload {
    pub message: String,
}

pub async fn steer_subagent_handler(
    State(_state): State<AppState>,
    Path(subagent_id): Path<String>,
    Json(payload): Json<SteerPayload>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    if subagent::steer_subagent(&subagent_id, payload.message) {
        Ok(Json(
            json!({ "status": "success", "subagent_id": subagent_id }),
        ))
    } else {
        Err((
            axum::http::StatusCode::NOT_FOUND,
            format!(
                "Subagent '{}' not active or steering queue closed.",
                subagent_id
            ),
        ))
    }
}

#[derive(serde::Deserialize)]
pub struct SwapModelPayload {
    pub model: String,
}

pub async fn swap_subagent_model_handler(
    State(_state): State<AppState>,
    Path(subagent_id): Path<String>,
    Json(payload): Json<SwapModelPayload>,
) -> Result<Json<Value>, (axum::http::StatusCode, String)> {
    if subagent::swap_subagent_model(&subagent_id, payload.model.clone()) {
        Ok(Json(
            json!({ "status": "success", "subagent_id": subagent_id, "model": payload.model }),
        ))
    } else {
        Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("Subagent '{}' not active.", subagent_id),
        ))
    }
}
