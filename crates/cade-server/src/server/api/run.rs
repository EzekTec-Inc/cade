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
    extract::{Path, State},
    response::{IntoResponse, Response, Sse, sse::Event},
    Json,
};
use cade_ai::{CompletionRequest, LlmToolCall, StreamChunk, catalogue};
use cade_store::sqlite;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::server::state::AppState;
use super::messages::{
    build_context, err, persist, maybe_set_conv_title, resolve_conversation,
};

/// Maximum agentic turns per request (prevents infinite loops).
const MAX_TURNS: usize = 20;

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
    {
        let mut activity = state.agent_activity.write().await;
        let entry = activity
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

    // ── Persist user message ──────────────────────────────────────────────
    let input = match body["input"].as_str().filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => return err(axum::http::StatusCode::BAD_REQUEST, "missing 'input'"),
    };

    let mut theme_cmd = None;
    if input.starts_with("/theme ") {
        theme_cmd = Some(input.trim_start_matches("/theme ").trim().to_string());
    } else {
        if let Some(cid) = conv_str.as_deref() {
            maybe_set_conv_title(&state, cid, &input);
        }
        persist(&state, &agent_id, conv_str.as_deref(), "user", json!({ "content": input }));
    }


    // ── Create run record ─────────────────────────────────────────────────
    let run_row = sqlite::create_run(&state.db, &agent_id, conv_str.as_deref());
    let run_id = run_row.map(|r| r.id).unwrap_or_else(|_| format!("run-local-{}", chrono::Utc::now().timestamp()));

    // Snapshot for the async stream task
    let state2 = state.clone();
    let agent_id2 = agent_id.clone();
    let conv_id2 = conv_str.clone();
    let run_id2 = run_id.clone();

    // ── Build SSE stream ──────────────────────────────────────────────────
    // We use an mpsc channel to bridge the async loop into an SSE stream.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(128);

    tokio::spawn(async move {
        let send = |data: Value| {
            let tx = tx.clone();
            let ev = Event::default().data(data.to_string());
            async move { let _ = tx.send(Ok(ev)).await; }
        };

        // ── stream_start ──────────────────────────────────────────────────
        send(json!({
            "message_type": "stream_start",
            "conversation_id": conv_id2,
            "run_id": run_id2,
        })).await;

        if let Some(t_name) = theme_cmd {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let agent_dir = dirs::home_dir().map(|h| h.join(".cade")).unwrap_or_else(|| std::path::PathBuf::from(".cade"));

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
            let colors_opt = cade_core::resources::themes::ThemeColors::builtin_by_name(&effective_name)
                .or_else(|| {
                    let all = cade_core::resources::themes::discover_themes(&cwd, &agent_dir);
                    all.iter()
                        .find(|t| t.name == effective_name)
                        .map(cade_core::resources::themes::ThemeColors::from_theme)
                });

            if let Some(colors) = colors_opt {
                // Persist the chosen theme on the agent row so GUI reloads
                // restore it automatically.  (Skip persist for `reload`
                // when the lookup already returned the persisted name —
                // writing the same value back is a no-op but clutters
                // audit trails; check string equality to avoid it.)
                if effective_name != t_name || t_name != "reload" {
                    let _ = cade_store::sqlite::agents::update_agent_theme(
                        &state2.db,
                        &agent_id2,
                        Some(&effective_name),
                    );
                }

                send(json!({
                    "message_type": "theme_update",
                    "theme": colors,
                    "theme_name": effective_name,
                })).await;
            } else {
                let all_themes = cade_core::resources::themes::discover_themes(&cwd, &agent_dir);
                let mut available: Vec<&str> = cade_core::resources::themes::ThemeColors::builtin_names().iter().copied().collect();
                available.extend(all_themes.iter().map(|t| t.name.as_str()));
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

        loop {
            turns += 1;
            if turns > MAX_TURNS {
                send(json!({
                    "message_type": "error",
                    "error": format!("Agentic loop exceeded {MAX_TURNS} turns — stopping"),
                })).await;
                break;
            }

            // ── Build context ─────────────────────────────────────────────
            let (model, messages, tools) =
                match build_context(&state2, &agent_id2, conv_id2.as_deref(), false).await {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        send(json!({ "message_type": "error", "error": e })).await;
                        break;
                    }
                };

            let max_tokens = catalogue::max_tokens_for_model(&model);
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
                    })).await;
                    crate::server::consolidation::consolidate_agent(
                        &state2,
                        &agent_id2,
                        conv_id2.as_deref(),
                    )
                    .await;
                    // Drop cached context entry so build_context recomputes.
                    {
                        let mut cache = state2
                            .context_cache
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        let key = format!("{}:{:?}", agent_id2, conv_id2.as_deref());
                        cache.pop(&key);
                    }
                    let (model2, mut messages2, tools2) =
                        match build_context(&state2, &agent_id2, conv_id2.as_deref(), false).await
                        {
                            Ok(ctx) => ctx,
                            Err(build_err) => {
                                send(json!({ "message_type": "error", "error": build_err }))
                                    .await;
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
                            break;
                        }
                    }
                }
                Err(e) => {
                    send(json!({ "message_type": "error", "error": e.to_string() })).await;
                    break;
                }
            };

            let mut text_acc = String::new();
            let mut tool_calls: Vec<LlmToolCall> = Vec::new();

            while let Some(chunk) = llm_stream.next().await {
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
                        })).await;
                        tool_calls.push(tc);
                    }
                    Ok(StreamChunk::Usage(u)) => {
                        send(json!({
                            "message_type": "usage_statistics",
                            "input_tokens":  u.input_tokens,
                            "output_tokens": u.output_tokens,
                            "cache_read_tokens":  u.cache_read_tokens,
                            "cache_write_tokens": u.cache_write_tokens,
                            "model": u.model,
                        })).await;
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
            for tc in &tool_calls {
                // Intercept `load_skill` — handle server-side instead of dispatching.
                let result = if tc.name == "load_skill" {
                    let args_str = tc.arguments.to_string();
                    handle_load_skill_tool(&state2, &agent_id2, &tc.id, &args_str).await
                } else if tc.name == "unload_skill" {
                    let args_str = tc.arguments.to_string();
                    handle_unload_skill_tool(&state2, &agent_id2, &tc.id, &args_str).await
                } else if tc.name == "run_subagent" {
                    let args: serde_json::Value = serde_json::from_str(
                        &tc.arguments.to_string()
                    ).unwrap_or_default();
                    handle_run_subagent_tool(
                        &state2, &agent_id2, &tc.id, &args, tx.clone(),
                    ).await
                } else {
                    cade_agent::tools::manager::dispatch(
                        tc.id.clone(),
                        &tc.name,
                        &tc.arguments,
                        &state2.mcp,
                    ).await
                };

                let output_trimmed = if result.output.len() > 8_192 {
                    format!(
                        "{}\n[... truncated: {} bytes]",
                        &result.output[..8_192],
                        result.output.len()
                    )
                } else {
                    result.output.clone()
                };

                // Stream the result to the GUI
                send(json!({
                    "message_type": "tool_result_message",
                    "tool_result": {
                        "id":       result.tool_call_id,
                        "name":     result.tool_name,
                        "output":   output_trimmed,
                        "is_error": result.is_error,
                    }
                })).await;

                // Persist into DB so next build_context sees it
                persist(
                    &state2,
                    &agent_id2,
                    conv_id2.as_deref(),
                    "tool",
                    json!({
                        "content":      output_trimmed,
                        "tool_call_id": result.tool_call_id,
                        "tool_name":    result.tool_name,
                    }),
                );
            }

            // Loop → re-invoke LLM with tool results
        }

        let _ = sqlite::finish_run(&state2.db, &run_id2, "done");

        // ── End of stream ─────────────────────────────────────────────────
        let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(stream).into_response()
}

/// Handle `load_skill` tool call server-side.
///
/// Parses the `id` argument, finds the skill in `all_skills`, activates it
/// for the agent, invalidates the context cache, and returns the skill body.
async fn handle_load_skill_tool(
    state: &AppState,
    agent_id: &str,
    tool_call_id: &str,
    arguments: &str,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::tools::manager::ToolResult;

    let skill_id = serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|v| v["id"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    if skill_id.is_empty() {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "load_skill".to_string(),
            output: "Error: missing 'id' parameter".to_string(),
            is_error: true,
        };
    }

    // Find skill
    let all = state.all_skills.read().await;
    let skill = all.iter().find(|s| s.id == skill_id).cloned();
    drop(all);

    match skill {
        Some(skill) => {
            // Activate for agent
            {
                let mut agent_skills = state.agent_skills.write().await;
                let loaded = agent_skills.entry(agent_id.to_string()).or_default();
                if !loaded.contains(&skill_id) {
                    loaded.push(skill_id.clone());
                }
            }

            // Invalidate context cache
            if let Ok(mut cache) = state.context_cache.lock() {
                let keys: Vec<String> = cache
                    .iter()
                    .filter(|(k, _)| k.starts_with(&format!("{agent_id}:")))
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in keys {
                    cache.pop(&k);
                }
            }

            ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "load_skill".to_string(),
                output: format!(
                    "Skill '{}' loaded ({} chars). It is now active in your system prompt.",
                    skill.name,
                    skill.body.chars().count()
                ),
                is_error: false,
            }
        }
        None => ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "load_skill".to_string(),
            output: format!("Error: skill '{skill_id}' not found"),
            is_error: true,
        },
    }
}

/// Handle `unload_skill` tool call server-side.
///
/// Removes the skill from the agent's active set. Does **not** invalidate
/// the context cache — the stale entry expires naturally on the next turn
/// when message history changes.
async fn handle_unload_skill_tool(
    state: &AppState,
    agent_id: &str,
    tool_call_id: &str,
    arguments: &str,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::tools::manager::ToolResult;

    let skill_id = serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|v| v["id"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    if skill_id.is_empty() {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "unload_skill".to_string(),
            output: "Error: missing 'id' parameter".to_string(),
            is_error: true,
        };
    }

    let removed = {
        let mut agent_skills = state.agent_skills.write().await;
        if let Some(loaded) = agent_skills.get_mut(agent_id) {
            let before = loaded.len();
            loaded.retain(|id| id != &skill_id);
            before != loaded.len()
        } else {
            false
        }
    };

    if removed {
        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "unload_skill".to_string(),
            output: format!(
                "Skill '{}' unloaded. It will no longer appear in your system prompt on the next turn.",
                skill_id
            ),
            is_error: false,
        }
    } else {
        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "unload_skill".to_string(),
            output: format!("Skill '{}' is not currently loaded", skill_id),
            is_error: true,
        }
    }
}

/// Handle `run_subagent` tool call server-side.
///
/// Runs a single-turn LLM call as a child subagent and streams lifecycle
/// events (started/complete) to the parent's SSE connection so the GUI
/// Strip `run_subagent` from a list of tool JSON schemas.
///
/// Second line of defence against runaway recursion (first is the depth
/// guard in [`handle_run_subagent_tool`]).  When a subagent is handed the
/// parent's full tool list (Approach C), we remove `run_subagent` here so
/// the subagent's LLM never even sees the tool advertised — defence in
/// depth alongside the runtime depth check.
fn filter_subagent_tools(schemas: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    schemas
        .into_iter()
        .filter(|s| s["name"].as_str() != Some("run_subagent"))
        .collect()
}

/// can render progress cards.
async fn handle_run_subagent_tool(
    state: &AppState,
    parent_agent_id: &str,
    tool_call_id: &str,
    args: &serde_json::Value,
    sse_tx: tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::tools::manager::ToolResult;
    use cade_ai::LlmMessage;

    let prompt = args["prompt"].as_str().unwrap_or("").trim().to_string();
    let mode = args["mode"].as_str().unwrap_or("build").to_string();
    let background = args["background"].as_bool().unwrap_or(false);
    let model_override = args["model"].as_str().map(|s| s.to_string());
    let _description = args["description"].as_str().unwrap_or("subagent task").to_string();

    // Recursion-depth guard.  When a subagent spawns another subagent, the
    // dispatching code injects `_subagent_depth = parent_depth + 1` into the
    // arguments before re-entering this function (see Approach C).  The
    // global semaphore (default 4 permits) only bounds *concurrent*
    // subagents, not *nested* ones — without this guard a subagent tree
    // could grow unboundedly deep until permits run out.  Default cap is
    // intentionally small (3) because deep nesting is almost always a
    // prompt-engineering bug, not legitimate use.
    let max_depth: usize = std::env::var("CADE_SUBAGENT_MAX_DEPTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let depth: usize = args["_subagent_depth"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or(0);
    if depth >= max_depth {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_subagent".to_string(),
            output: format!(
                "error: subagent recursion depth {depth} exceeds CADE_SUBAGENT_MAX_DEPTH ({max_depth}). \
                 Refusing to spawn deeper. Restructure the task or raise the limit if intentional."
            ),
            is_error: true,
        };
    }

    if prompt.is_empty() {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_subagent".to_string(),
            output: "error: 'prompt' is required".to_string(),
            is_error: true,
        };
    }

    // Acquire semaphore permit
    let permit = match state.subagent_semaphore.try_acquire() {
        Ok(p) => p,
        Err(_) => {
            return ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "run_subagent".to_string(),
                output: "error: subagent concurrency limit reached. Try again later.".to_string(),
                is_error: true,
            };
        }
    };

    let subagent_id = format!("sa_{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let task_preview: String = prompt.chars().take(80).collect();

    // Resolve model
    let model = model_override.unwrap_or_else(|| {
        cade_store::sqlite::get_agent(&state.db, parent_agent_id)
            .ok()
            .flatten()
            .map(|a| a.model)
            .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string())
    });

    // Stream subagent_started event
    let started_event = json!({
        "message_type": "subagent_started",
        "subagent_id": &subagent_id,
        "task": &task_preview,
        "mode": &mode,
        "model": &model,
    });
    let ev = axum::response::sse::Event::default().data(started_event.to_string());
    let _ = sse_tx.send(Ok(ev)).await;

    let start_time = std::time::Instant::now();

    let system_prompt = if mode == "plan" {
        format!(
            "You are a read-only planning subagent. Analyze and report. \
             Do NOT modify files.\n\nTask: {prompt}"
        )
    } else {
        format!(
            "You are a subagent. Complete the task and return a concise summary.\n\nTask: {prompt}"
        )
    };

    let messages_init = vec![
        LlmMessage {
            role: "system".to_string(),
            content: system_prompt,
            tool_calls: None,
            tool_call_id: None,
            images: None,
        },
        LlmMessage {
            role: "user".to_string(),
            content: prompt.clone(),
            tool_calls: None,
            tool_call_id: None,
            images: None,
        },
    ];

    // ── Subagent agentic loop (Approach C) ──────────────────────────────
    //
    // Iterates LLM → tool dispatch → LLM with tool result, up to
    // `max_iters` rounds.  Tools are loaded from the parent agent's tool
    // list (with `run_subagent` stripped — see `filter_subagent_tools`)
    // and dispatched through the same `cade_agent::tools::manager::dispatch`
    // helper the parent loop uses.  No SSE streaming inside the loop and
    // no per-iteration DB persistence — subagents are ephemeral and only
    // their final result flows back to the parent.
    //
    // The loop terminates when either:
    //   (a) the LLM returns no tool_calls (assistant produced a final answer),
    //   (b) `max_iters` is reached (safety cap),
    //   (c) an LLM or dispatch error surfaces.
    let max_iters: usize = std::env::var("CADE_SUBAGENT_MAX_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    // Snapshot the parent agent's tool schemas, stripped of `run_subagent`
    // for defence-in-depth alongside the depth counter.  If the parent is
    // not yet wired (no rows), `agent_tool_ids` is empty meaning "all
    // registered tools".
    let parent_tool_schemas: Vec<serde_json::Value> = {
        let parent_tool_ids =
            cade_store::sqlite::get_agent_tool_ids(&state.db, parent_agent_id).unwrap_or_default();
        let all = cade_store::sqlite::list_tools(&state.db).unwrap_or_default();
        let raw: Vec<serde_json::Value> = if parent_tool_ids.is_empty() {
            all.into_iter().filter_map(|t| t.json_schema).collect()
        } else {
            all.into_iter()
                .filter(|t| parent_tool_ids.contains(&t.id))
                .filter_map(|t| t.json_schema)
                .collect()
        };
        filter_subagent_tools(raw)
    };

    let mut messages = messages_init;
    let mut last_text = String::new();
    let mut llm_err: Option<String> = None;
    let next_depth = depth + 1;

    for _iter in 0..max_iters {
        let llm_req = cade_ai::CompletionRequest {
            model: model.clone(),
            messages: messages.clone(),
            tools: parent_tool_schemas.clone(),
            max_tokens: 4096,
            reasoning_effort: None,
        };

        let resp = match state.llm.complete(&llm_req).await {
            Ok(r) => r,
            Err(e) => {
                llm_err = Some(e.to_string());
                break;
            }
        };

        // Persist any text the assistant produced this iteration.
        if let Some(t) = &resp.content
            && !t.is_empty()
        {
            last_text = t.clone();
        }

        if resp.tool_calls.is_empty() {
            // Final answer reached.
            break;
        }

        // Append the assistant message (with tool_calls) so the next iter
        // sees it in conversational context.
        messages.push(LlmMessage {
            role: "assistant".to_string(),
            content: resp.content.clone().unwrap_or_default(),
            tool_calls: Some(resp.tool_calls.clone()),
            tool_call_id: None,
            images: None,
        });

        // Dispatch each tool call and append the result back into messages.
        for tc in &resp.tool_calls {
            // Hard re-entry guard: even if `run_subagent` somehow leaked
            // into the schema list, refuse to recurse without a depth
            // bump.  We forward the same dispatch path the parent uses,
            // but inject `_subagent_depth: next_depth` so the recursive
            // call sees the updated counter.
            let tool_result = if tc.name == "run_subagent" {
                let mut nested_args = tc.arguments.clone();
                if let Some(obj) = nested_args.as_object_mut() {
                    obj.insert(
                        "_subagent_depth".to_string(),
                        serde_json::Value::from(next_depth as u64),
                    );
                }
                // Re-enter via a Box::pin to satisfy async recursion.
                Box::pin(handle_run_subagent_tool(
                    state,
                    parent_agent_id,
                    &tc.id,
                    &nested_args,
                    sse_tx.clone(),
                ))
                .await
            } else {
                cade_agent::tools::manager::dispatch(
                    tc.id.clone(),
                    &tc.name,
                    &tc.arguments,
                    &state.mcp,
                )
                .await
            };

            messages.push(LlmMessage {
                role: "tool".to_string(),
                content: tool_result.output.clone(),
                tool_calls: None,
                tool_call_id: Some(tool_result.tool_call_id.clone()),
                images: None,
            });
        }
    }

    let elapsed = start_time.elapsed().as_secs() as u32;
    drop(permit);

    let (output, is_error) = match llm_err {
        Some(e) => (format!("Subagent error: {e}"), true),
        None => (last_text, false),
    };

    // Stream subagent_complete event
    let result_preview: String = output.chars().take(200).collect();
    let complete_event = json!({
        "message_type": "subagent_complete",
        "subagent_id": &subagent_id,
        "status": if is_error { "error" } else { "success" },
        "result_preview": &result_preview,
        "elapsed_secs": elapsed,
        "is_error": is_error,
    });
    let ev = axum::response::sse::Event::default().data(complete_event.to_string());
    let _ = sse_tx.send(Ok(ev)).await;

    if background {
        let sr = crate::server::state::SubagentResult {
            subagent_id: subagent_id.clone(),
            tool_call_id: tool_call_id.to_string(),
            task_preview: task_preview.clone(),
            result: output.clone(),
            is_error,
            elapsed_secs: elapsed,
        };
        let mut pending = state.pending_subagent_results.write().await;
        pending
            .entry(parent_agent_id.to_string())
            .or_default()
            .push(sr);
    }

    let output_final = if output.len() > 8_192 {
        format!("{}…\n[truncated: {} chars total]", &output[..8_192], output.len())
    } else {
        output
    };

    ToolResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name: "run_subagent".to_string(),
        output: output_final,
        is_error,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    /// A mock LlmProvider that panics if called.  Used to assert that an early
    /// return path (e.g. depth-limit guard) never reaches the LLM at all.
    struct PanicOnCallLlm;
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for PanicOnCallLlm {
        async fn complete(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            panic!("LLM should not be called when depth limit is hit");
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            panic!("LLM stream should not be called");
        }
    }

    fn build_state_with_llm(llm: std::sync::Arc<dyn cade_ai::LlmProvider>) -> AppState {
        let db = cade_store::sqlite::open(":memory:").unwrap();
        let config = std::sync::Arc::new(crate::server::config::ServerConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            db_path: ":memory:".into(),
            llm_provider: crate::server::config::LlmProviderKind::Anthropic,
            default_model: "test".into(),
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            api_key: None,
            allowed_origin: None,
            max_context_budget: None,
        });
        AppState {
            db,
            llm,
            llm_router: std::sync::Arc::new(tokio::sync::RwLock::new(cade_ai::LlmRouter::build(
                &cade_ai::AiConfig {
                    anthropic_api_key: None,
                    openai_api_key: None,
                    google_api_key: None,
                    ollama_base_url: String::new(),
                    llm_provider: String::new(),
                },
            ))),
            config,
            mcp: std::sync::Arc::new(crate::server::state::McpManager::empty()),
            rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
            memory_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            agent_activity: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            agent_metrics: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            agent_context_telemetry: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            context_cache: std::sync::Arc::new(std::sync::Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(20).unwrap(),
            ))),
            all_skills: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            agent_skills: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            pending_subagent_results: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
        }
    }

    /// Stateful mock: returns `run_subagent` exactly once per loop level
    /// (when the message list has just system+user) and a final text on
    /// the next iter (after a tool result has been appended).  This keeps
    /// the test fast while still exercising depth recursion.
    struct OneRecurseLlm {
        call_count: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for OneRecurseLlm {
        async fn complete(
            &self,
            r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            self.call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Initial state for a fresh subagent loop is exactly 2 msgs
            // (system + user).  Anything more means we are post-tool-result.
            let is_initial = r.messages.len() == 2;
            if is_initial {
                Ok(cade_ai::CompletionResponse {
                    content: Some("recursing".into()),
                    tool_calls: vec![cade_ai::LlmToolCall {
                        id: "tc_rec".into(),
                        name: "run_subagent".into(),
                        arguments: serde_json::json!({"prompt": "deeper"}),
                        thought_signature: None,
                    }],
                    finish_reason: "tool_use".into(),
                })
            } else {
                Ok(cade_ai::CompletionResponse {
                    content: Some("done".into()),
                    tool_calls: vec![],
                    finish_reason: "stop".into(),
                })
            }
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unimplemented!()
        }
    }

    /// Recursion bound: a subagent that recurses once per level must hit
    /// the depth cap (default 3) and return without deadlock.  At depth 3
    /// the call is refused before acquiring a permit, so the chain
    /// terminates.  Asserts (a) outer call succeeds, (b) LLM call count
    /// is small (linear in depth, not exponential).
    #[tokio::test]
    async fn recursive_subagent_calls_are_bounded_by_depth() {
        let llm = std::sync::Arc::new(OneRecurseLlm {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        });
        let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(64);

        // depth 0 → 1 → 2 → 3 (refused).  Each level: 2 LLM calls
        // (initial recurse + final).  Depth-2's recurse to depth 3 is
        // refused (tool result = error), then depth-2's next iter sees
        // post-tool-result state and returns final text.
        let args = serde_json::json!({ "prompt": "start", "_subagent_depth": 0 });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            handle_run_subagent_tool(&state, "parent_x", "tc_outer", &args, tx),
        )
        .await
        .expect("must not deadlock — chain must terminate via depth guard");

        assert!(!result.is_error, "outer subagent should complete: {}", result.output);
        let calls = llm.call_count.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            calls > 0 && calls < 20,
            "LLM call count must be small (linear in depth), got: {calls}"
        );
    }

    /// Approach C deliberately runs the subagent loop in-memory without
    /// creating ephemeral `agent`/`message` rows.  That keeps the parent
    /// agent's conversation history clean and avoids cross-contamination.
    /// This test is a watchdog: if a future change accidentally persists
    /// subagent traffic it will fail loudly.
    #[tokio::test]
    async fn subagent_run_does_not_pollute_parent_db() {
        let llm = std::sync::Arc::new(ScriptedLlm {
            call_count: std::sync::atomic::AtomicUsize::new(0),
            captured_iter2_messages: std::sync::Mutex::new(Vec::new()),
        });
        let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let agents_before: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
            .unwrap();
        let messages_before: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();

        let args = serde_json::json!({ "prompt": "do thing" });
        let _ = handle_run_subagent_tool(&state, "parent_x", "tc_outer", &args, tx).await;

        let agents_after: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
            .unwrap();
        let messages_after: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
            .unwrap();

        assert_eq!(
            agents_before, agents_after,
            "subagent must not create agent rows"
        );
        assert_eq!(
            messages_before, messages_after,
            "subagent must not persist messages to parent DB"
        );
    }

    /// A stateful mock that on the FIRST call returns a tool_call (forcing a
    /// loop iteration), and on the SECOND call returns plain text.  The
    /// LLM messages it receives are recorded so tests can verify that the
    /// subagent loop fed back the tool result.
    struct ScriptedLlm {
        call_count: std::sync::atomic::AtomicUsize,
        captured_iter2_messages: std::sync::Mutex<Vec<cade_ai::LlmMessage>>,
    }
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for ScriptedLlm {
        async fn complete(
            &self,
            r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            let n = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(cade_ai::CompletionResponse {
                    content: None,
                    tool_calls: vec![cade_ai::LlmToolCall {
                        id: "tc_inner_1".into(),
                        name: "fake_tool".into(),
                        arguments: serde_json::json!({}),
                        thought_signature: None,
                    }],
                    finish_reason: "tool_use".into(),
                })
            } else {
                let mut g = self.captured_iter2_messages.lock().unwrap();
                *g = r.messages.clone();
                Ok(cade_ai::CompletionResponse {
                    content: Some("subagent done".into()),
                    tool_calls: vec![],
                    finish_reason: "stop".into(),
                })
            }
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unimplemented!()
        }
    }

    /// RED: subagent currently does a single `complete()` and returns the
    /// text.  When the LLM returns a tool_call instead, the subagent loop
    /// must dispatch it and feed the result back in a second LLM call.
    /// Asserts (a) the LLM was called exactly twice, (b) the second call
    /// saw a "tool" role message containing the dispatch result.
    #[tokio::test]
    async fn subagent_dispatches_tool_calls_and_loops() {
        let llm = std::sync::Arc::new(ScriptedLlm {
            call_count: std::sync::atomic::AtomicUsize::new(0),
            captured_iter2_messages: std::sync::Mutex::new(Vec::new()),
        });
        let llm_dyn = llm.clone() as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm_dyn);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let args = serde_json::json!({ "prompt": "do thing" });
        let result =
            handle_run_subagent_tool(&state, "parent_x", "tc_outer", &args, tx).await;

        assert!(!result.is_error, "loop must succeed, got: {}", result.output);
        assert_eq!(
            llm.call_count.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "LLM must be called twice (first tool_call, then completion)"
        );
        let iter2 = llm.captured_iter2_messages.lock().unwrap().clone();
        let has_tool_msg = iter2
            .iter()
            .any(|m| m.role == "tool" && m.content.contains("fake_tool"));
        assert!(
            has_tool_msg,
            "iteration-2 messages must include a tool-role message echoing dispatch result, got roles: {:?}",
            iter2.iter().map(|m| &m.role).collect::<Vec<_>>()
        );
        assert!(
            result.output.contains("subagent done"),
            "final output must be from second LLM call, got: {}",
            result.output
        );
    }

    /// Subagents must NOT receive `run_subagent` in their tool list — this
    /// is the second line of defence against runaway recursion (the first
    /// being the depth guard in `handle_run_subagent_tool`).  Removing the
    /// schema means the subagent's LLM never sees the tool advertised.
    #[test]
    fn filter_subagent_tools_strips_run_subagent_schema() {
        let schemas = vec![
            serde_json::json!({"name": "bash"}),
            serde_json::json!({"name": "run_subagent"}),
            serde_json::json!({"name": "read_file"}),
        ];
        let filtered = filter_subagent_tools(schemas);
        let names: Vec<String> = filtered
            .iter()
            .filter_map(|s| s["name"].as_str().map(String::from))
            .collect();
        assert!(
            !names.iter().any(|n| n == "run_subagent"),
            "run_subagent must be stripped, got: {names:?}"
        );
        assert!(names.iter().any(|n| n == "bash"));
        assert!(names.iter().any(|n| n == "read_file"));
    }

    /// RED: at depth >= CADE_SUBAGENT_MAX_DEPTH (default 3), the tool must
    /// short-circuit with an error and never call the LLM.  Currently fails
    /// because no depth guard exists.
    #[tokio::test]
    async fn depth_limit_blocks_recursion_before_llm_call() {
        let llm = std::sync::Arc::new(PanicOnCallLlm) as std::sync::Arc<dyn cade_ai::LlmProvider>;
        let state = build_state_with_llm(llm);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let args = serde_json::json!({
            "prompt": "do thing",
            "_subagent_depth": 3,
        });

        // Should NOT panic — i.e. LLM is never called.
        let result =
            handle_run_subagent_tool(&state, "parent_agent_x", "tc_1", &args, tx).await;

        assert!(result.is_error, "depth-limit must produce an error result");
        assert!(
            result.output.to_lowercase().contains("depth"),
            "error message must mention depth, got: {}",
            result.output
        );
    }
}
