//! Subagent spawning and execution within the server-side agentic loop.

use serde_json::json;

use crate::server::state::AppState;

pub(super) fn filter_subagent_tools(schemas: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    schemas
        .into_iter()
        .filter(|s| s["name"].as_str() != Some("run_subagent"))
        .collect()
}

/// can render progress cards.
pub(super) async fn handle_run_subagent_tool(
    state: &AppState,
    parent_agent_id: &str,
    tool_call_id: &str,
    args: &serde_json::Value,
    sse_tx: tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::subagents::SubagentConfig;
    use cade_agent::tools::manager::ToolResult;
    use cade_ai::LlmMessage;

    // -- Parse + validate args through shared SubagentConfig -----------------
    let cfg = SubagentConfig::from_args(args);

    // Recursion-depth guard.  When a subagent spawns another subagent the
    // dispatcher injects `_subagent_depth = parent_depth + 1` into the
    // arguments before re-entering this function.  Default cap is 3.
    let max_depth: usize = std::env::var("CADE_SUBAGENT_MAX_DEPTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    if cfg.depth >= max_depth {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_subagent".to_string(),
            output: format!(
                "error: subagent recursion depth {} exceeds CADE_SUBAGENT_MAX_DEPTH ({max_depth}). \
                 Refusing to spawn deeper. Restructure the task or raise the limit if intentional.",
                cfg.depth
            ),
            is_error: true,
        };
    }

    if let Err(reason) = cfg.validate() {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_subagent".to_string(),
            output: reason,
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

    let subagent_id   = format!("sa_{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let task_preview: String = cfg.prompt.chars().take(80).collect();
    let prompt        = cfg.prompt_with_test_command();

    // Resolve subagent definition + model via shared helpers
    let cwd_for_defs = std::env::current_dir().unwrap_or_default();
    let all_defs = cade_agent::subagents::discover_all_subagents(&cwd_for_defs);
    let def_opt  = cade_agent::subagents::resolve_subagent_def(&cfg.mode, &all_defs);

    let model = cfg
        .resolve_model(def_opt)
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
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
        "mode": &cfg.mode,
        "model": &model,
    });
    let ev = axum::response::sse::Event::default().data(started_event.to_string());
    let _ = sse_tx.send(Ok(ev)).await;

    let start_time = std::time::Instant::now();

    // Build system prompt via shared resolution chain
    let system_prompt_base = cfg.resolve_system_prompt(def_opt);
    // Append "Task: <prompt>" so the subagent sees it in the system context
    // (the prompt is also sent as a separate user message below).
    let system_prompt = format!("{system_prompt_base}\n\nTask: {prompt}");

    // Seed the parent agent's pinned + short-tier memory blocks into the
    // subagent's system prompt so it inherits project context, persona,
    // and the active goal.  Uses the shared SubagentConfig helper to
    // ensure filtering and capping are identical in both paths.
    let seed_section: String = {
        let raw_blocks = cade_store::sqlite::get_active_blocks(&state.db, parent_agent_id)
            .unwrap_or_default();
        let seed: Vec<cade_agent::agent::client::MemoryBlock> = raw_blocks
            .into_iter()
            .map(|(label, value, description, tier, _last_turn)| {
                cade_agent::agent::client::MemoryBlock {
                    label,
                    value,
                    description: if description.is_empty() { None } else { Some(description) },
                    tier: if tier.is_empty() { None } else { Some(tier) },
                }
            })
            .collect();
        let filtered = SubagentConfig::build_seed_memory(seed);
        SubagentConfig::format_seed_section(&filtered)
    };
    let system_prompt_full = format!("{system_prompt}{seed_section}");

    let messages_init = vec![
        LlmMessage {
            role: "system".to_string(),
            content: system_prompt_full,
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
    let next_depth = cfg.depth + 1;

    // Create a lightweight ephemeral DB row for the subagent so its
    // meta-tool calls (update_memory, load_skill, etc.) are scoped to
    // its own namespace rather than writing into the parent agent's
    // memory store (memory isolation fix).
    let _ = cade_store::sqlite::create_agent(
        &state.db,
        &cade_store::sqlite::AgentRow {
            id: subagent_id.clone(),
            name: cfg.ephemeral_agent_name(&subagent_id),
            model: model.clone(),
            description: Some(cfg.ephemeral_description()),
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
        },
    );

    for _iter in 0..max_iters {
        let llm_req = cade_ai::CompletionRequest {
            model: model.clone(),
            messages: messages.clone(),
            tools: parent_tool_schemas.clone(),
            // Bug 4 fix: raised from 4096 → 8192. 4k was too low for coding
            // subagents; complex outputs (file writes, detailed analysis)
            // were silently truncated mid-response. 8k matches the typical
            // per-turn budget for the parent agent loop.
            max_tokens: 8192,
            reasoning_effort: None,
        };

        let resp = match state.llm.complete(&llm_req).await {
            Ok(r) => r,
            Err(e) => {
                llm_err = Some(e.to_string());
                break;
            }
        };

        // Accumulate the assistant's prose across iterations.
        //
        // Previously this overwrote `last_text` on every iter, which silently
        // discarded all intermediate explanation/working/findings the
        // subagent produced before its final tool call.  The parent only
        // ever saw the LAST iteration's text — typically empty, since the
        // last iter is often a tool call with no prose.
        //
        // Now we append, joining iterations with a blank line so the parent
        // receives the full investigation log as a single coherent message.
        if let Some(t) = &resp.content
            && !t.is_empty()
        {
            if !last_text.is_empty() {
                last_text.push_str("\n\n");
            }
            last_text.push_str(t);
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
            } else if let Some(intercepted) =
                Box::pin(super::meta_tools::intercept_meta_tool(state, &subagent_id, tc, sse_tx.clone())).await
            {
                // Meta-tools (memory, skills, checkpoints, artifacts)
                // are dispatched against the subagent's own DB row so its
                // memory writes don't pollute the parent agent's store.
                intercepted
            } else {
                cade_agent::tools::manager::dispatch(
                    tc.id.clone(),
                    &tc.name,
                    &tc.arguments,
                    &state.mcp,
                )
                .await
            };

            // Track file-editing tools in the parent agent's recent_edits
            // memory block so the parent knows what the subagent changed.
            if !tool_result.is_error
                && cade_agent::tools::manager::is_file_edit_tool(&tc.name)
                && let Some(path) = tc.arguments["path"]
                    .as_str()
                    .or_else(|| tc.arguments["file_path"].as_str())
                {
                    super::record_recent_edit_db(&state.db, parent_agent_id, path);
                }

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

    // Clean up the ephemeral subagent DB row — memory it wrote is no
    // longer needed once the result has been returned to the parent.
    let _ = cade_store::sqlite::delete_agent(&state.db, &subagent_id);

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

    if cfg.background {
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

    // C2: truncate at a UTF-8 char boundary, never at a raw byte index.
    let output_final = if output.len() > super::SSE_OUTPUT_TRUNCATE_BYTES {
        let head = super::truncate_at_char_boundary(&output, super::SSE_OUTPUT_TRUNCATE_BYTES);
        format!(
            "{}…\n[truncated: {} chars total]",
            head,
            output.len()
        )
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
