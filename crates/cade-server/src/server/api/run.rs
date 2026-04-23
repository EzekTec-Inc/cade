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
            let mut llm_stream = match state2.llm.stream(&req).await {
                Ok(s) => s,
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

    let messages = vec![
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

    let llm_req = cade_ai::CompletionRequest {
        model: model.clone(),
        messages,
        tools: Vec::new(),
        max_tokens: 4096,
        reasoning_effort: None,
    };

    let llm_result = state.llm.complete(&llm_req).await;
    let elapsed = start_time.elapsed().as_secs() as u32;
    drop(permit);

    let (output, is_error) = match llm_result {
        Ok(resp) => (resp.content.unwrap_or_default(), false),
        Err(e) => (format!("Subagent error: {e}"), true),
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