//! Subagent spawning and execution within the server-side agentic loop.

use crate::server::state::AppState;

/// REC-2: Drop guard that ensures the ephemeral agent DB row is cleaned
/// up even if the agentic loop panics or returns early.  On drop it:
///   1. Writes back any subagent findings to the parent (A15).
///   2. Deletes the ephemeral agent row.
///
/// The `writeback_count` field is set during drop so callers that need
/// the count can read it *before* drop (by calling `write_back_and_delete`
/// manually) or accept that the Drop path returns nothing.
pub(super) struct EphemeralEnvironment {
    db: cade_store::sqlite::Db,
    subagent_id: String,
    parent_agent_id: String,
    /// Set to `true` once the guard has already run (e.g. manual call).
    defused: bool,
}

impl EphemeralEnvironment {
    pub(super) fn new(
        db: cade_store::sqlite::Db,
        subagent_id: String,
        parent_agent_id: String,
    ) -> Self {
        Self {
            db,
            subagent_id,
            parent_agent_id,
            defused: false,
        }
    }

    /// Async write-back that supports Smart Memory Merge.
    pub(super) async fn write_back_and_delete_async(&mut self, state: &AppState) -> usize {
        if self.defused {
            return 0;
        }
        self.defused = true;

        let facts = cade_store::sqlite::memory::extract_subagent_memory_for_writeback(
            &self.db,
            &self.subagent_id,
        );

        let parent_blocks = cade_store::sqlite::get_memory_blocks(&self.db, &self.parent_agent_id)
            .unwrap_or_default();

        let mut written = 0;
        for fact in &facts {
            let parent_label = format!("subagent:{}", fact.label);
            let desc = if fact.description.is_empty() {
                Some(format!("Written back from subagent {}", self.subagent_id))
            } else {
                Some(format!(
                    "{} (from subagent {})",
                    fact.description, self.subagent_id
                ))
            };

            // Smart Memory Merge: If the parent already has this label, do an LLM merge
            if let Some((_, old_value, _)) =
                parent_blocks.iter().find(|(l, _, _)| l == &parent_label)
            {
                // REC-6/G6: Await the merge with a bounded timeout so that
                // memory conflicts are resolved synchronously before teardown.
                // Fire-and-forget spawns previously risked silently losing data
                // when the merge LLM call failed.
                let merge_result = tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    smart_memory_merge(
                        state.clone(),
                        self.parent_agent_id.clone(),
                        parent_label.clone(),
                        old_value.clone(),
                        fact.value.clone(),
                        fact.memory_type.clone(),
                        fact.confidence,
                    ),
                )
                .await;
                if merge_result.is_err() {
                    tracing::warn!(
                        label = %parent_label,
                        subagent_id = %self.subagent_id,
                        "smart_memory_merge timed out; retaining old value"
                    );
                }
                written += 1;
            } else {
                if cade_store::sqlite::upsert_memory_block_typed(
                    &self.db,
                    &self.parent_agent_id,
                    &parent_label,
                    &fact.value,
                    desc.as_deref(),
                    None,
                    Some(&fact.memory_type),
                    Some(fact.confidence),
                )
                .is_ok()
                {
                    written += 1;
                }
            }
        }

        let _ = cade_store::sqlite::delete_agent(&self.db, &self.subagent_id);
        written
    }
}

impl Drop for EphemeralEnvironment {
    fn drop(&mut self) {
        if !self.defused {
            self.defused = true;
            let _ = cade_store::sqlite::memory::write_back_subagent_memory(
                &self.db,
                &self.subagent_id,
                &self.parent_agent_id,
            );
            let _ = cade_store::sqlite::delete_agent(&self.db, &self.subagent_id);
        }
    }
}

pub(super) fn filter_subagent_tools(
    schemas: Vec<serde_json::Value>,
    allowed: &cade_agent::subagents::SubagentTools,
) -> Vec<serde_json::Value> {
    schemas
        .into_iter()
        .filter(|s| {
            let name = s["name"].as_str().unwrap_or("");
            // Strip tools that must never appear in a subagent's inherited schema:
            // - run_subagent / run_parallel_subagents: prevent runaway recursion
            // - finish: injected fresh by the executor; stripping here prevents
            //   the parent's stale schema from leaking in or causing double routing
            if matches!(name, "run_subagent" | "run_parallel_subagents" | "finish") {
                return false;
            }
            match allowed {
                cade_agent::subagents::SubagentTools::All => true,
                cade_agent::subagents::SubagentTools::Readonly => {
                    matches!(
                        name,
                        "read_file"
                            | "glob"
                            | "grep"
                            | "search_memory"
                            | "conversation_search"
                            | "archival_memory_search"
                            | "recall"
                    )
                }
                cade_agent::subagents::SubagentTools::List(names) => {
                    names.iter().any(|n| n == name)
                }
                cade_agent::subagents::SubagentTools::Restricted { allowed_tools, .. } => {
                    allowed_tools.iter().any(|n| n == name)
                }
            }
        })
        .collect()
}

/// REC-1: Wall-clock timeout for the subagent agentic loop.
///
/// In production reads `CADE_SUBAGENT_TIMEOUT_SECS` (default 300).
/// Under `cfg(test)` returns 2 seconds so tests run fast.
fn subagent_timeout_secs() -> u64 {
    #[cfg(test)]
    {
        2
    }
    #[cfg(not(test))]
    {
        std::env::var("CADE_SUBAGENT_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300)
    }
}

pub trait SubagentEventEmitter: Send + Sync {
    fn emit_started<'a>(
        &'a self,
        subagent_id: &'a str,
        task_preview: &'a str,
        mode: &'a str,
        model: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
    fn emit_complete<'a>(
        &'a self,
        subagent_id: &'a str,
        is_error: bool,
        result_preview: &'a str,
        elapsed: u32,
        writeback_facts: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
    fn raw_sse_tx(
        &self,
    ) -> tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>;
}

pub struct SseEventEmitter {
    pub tx: tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
}

impl SubagentEventEmitter for SseEventEmitter {
    fn emit_started<'a>(
        &'a self,
        subagent_id: &'a str,
        task_preview: &'a str,
        mode: &'a str,
        model: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        let subagent_id = subagent_id.to_string();
        let task_preview = task_preview.to_string();
        let mode = mode.to_string();
        let model = model.to_string();
        let tx = self.tx.clone();
        Box::pin(async move {
            let ev = serde_json::json!({
                "message_type": "subagent_started",
                "subagent_id": subagent_id,
                "task": task_preview,
                "mode": mode,
                "model": model,
            });
            let _ = tx
                .send(Ok(
                    axum::response::sse::Event::default().data(ev.to_string())
                ))
                .await;
        })
    }

    fn emit_complete<'a>(
        &'a self,
        subagent_id: &'a str,
        is_error: bool,
        result_preview: &'a str,
        elapsed: u32,
        writeback_facts: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        let subagent_id = subagent_id.to_string();
        let result_preview = result_preview.to_string();
        let tx = self.tx.clone();
        Box::pin(async move {
            let ev = serde_json::json!({
                "message_type": "subagent_complete",
                "subagent_id": subagent_id,
                "status": if is_error { "error" } else { "success" },
                "result_preview": result_preview,
                "elapsed_secs": elapsed,
                "is_error": is_error,
                "writeback_facts": writeback_facts,
            });
            let _ = tx
                .send(Ok(
                    axum::response::sse::Event::default().data(ev.to_string())
                ))
                .await;
        })
    }

    fn raw_sse_tx(
        &self,
    ) -> tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>
    {
        self.tx.clone()
    }
}

pub struct SubagentExecutor<'a> {
    pub state: &'a AppState,
    pub parent_agent_id: &'a str,
    pub tool_call_id: &'a str,
    pub emitter: Box<dyn SubagentEventEmitter>,
}

impl<'a> SubagentExecutor<'a> {
    pub fn new(
        state: &'a AppState,
        parent_agent_id: &'a str,
        tool_call_id: &'a str,
        emitter: Box<dyn SubagentEventEmitter>,
    ) -> Self {
        Self {
            state,
            parent_agent_id,
            tool_call_id,
            emitter,
        }
    }

    pub async fn execute(self, args: &serde_json::Value) -> cade_agent::tools::manager::ToolResult {
        handle_run_subagent_tool_inner(
            self.state,
            self.parent_agent_id,
            self.tool_call_id,
            args,
            self.emitter,
        )
        .await
    }
}

pub(super) async fn handle_run_subagent_tool(
    state: &AppState,
    parent_agent_id: &str,
    tool_call_id: &str,
    args: &serde_json::Value,
    sse_tx: tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
) -> cade_agent::tools::manager::ToolResult {
    let executor = SubagentExecutor::new(
        state,
        parent_agent_id,
        tool_call_id,
        Box::new(SseEventEmitter { tx: sse_tx }),
    );
    executor.execute(args).await
}

pub(super) async fn handle_run_subagent_tool_inner(
    state: &AppState,
    parent_agent_id: &str,
    tool_call_id: &str,
    args: &serde_json::Value,
    emitter: Box<dyn SubagentEventEmitter>,
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

    // REC-3/G2: Backpressure — block until a semaphore slot is free instead
    // of returning an instant error that causes the parent LLM to retry-loop.
    // Wrapped in the wall-clock timeout so a full semaphore never hangs forever.
    let permit = match tokio::time::timeout(
        std::time::Duration::from_secs(subagent_timeout_secs()),
        state.subagent_semaphore.acquire(),
    )
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(_)) => {
            return ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "run_subagent".to_string(),
                output: "error: subagent semaphore closed.".to_string(),
                is_error: true,
            };
        }
        Err(_) => {
            return ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "run_subagent".to_string(),
                output: format!(
                    "error: timed out waiting for a subagent slot after {}s. \
                     All {} slots are occupied. Retry later or raise CADE_MAX_SUBAGENTS.",
                    subagent_timeout_secs(),
                    std::env::var("CADE_MAX_SUBAGENTS")
                        .ok()
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(4)
                ),
                is_error: true,
            };
        }
    };

    let subagent_id = format!("sa_{}", &uuid::Uuid::new_v4().to_string()[..8]);
    let task_preview: String = cfg.prompt.chars().take(80).collect();
    let prompt = cfg.prompt_with_test_command();

    // Resolve subagent definition + model via shared helpers
    let cwd_for_defs = std::env::current_dir().unwrap_or_default();
    let all_defs = cade_agent::subagents::discover_all_subagents(&cwd_for_defs);
    let def_opt = cade_agent::subagents::resolve_subagent_def(&cfg.mode, &all_defs);

    let parent_model = cade_store::sqlite::get_agent(&state.db, parent_agent_id)
        .ok()
        .flatten()
        .map(|a| a.model)
        .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());

    let mut model = cfg
        .resolve_model(def_opt)
        .map(|s| s.to_string())
        .unwrap_or_else(|| parent_model.clone());

    emitter
        .emit_started(&subagent_id, &task_preview, &cfg.mode, &model)
        .await;

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
        let raw_blocks =
            cade_store::sqlite::get_active_blocks(&state.db, parent_agent_id).unwrap_or_default();
        let seed: Vec<cade_agent::agent::client::MemoryBlock> = raw_blocks
            .into_iter()
            .map(|(label, value, description, tier, _last_turn)| {
                cade_agent::agent::client::MemoryBlock {
                    label,
                    value,
                    description: if description.is_empty() {
                        None
                    } else {
                        Some(description)
                    },
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
        let tools_filter = def_opt.map(|d| &d.tools).unwrap_or_else(|| {
            if cfg.mode == "plan" {
                &cade_agent::subagents::SubagentTools::Readonly
            } else {
                &cade_agent::subagents::SubagentTools::All
            }
        });
        let mut filtered = filter_subagent_tools(raw, tools_filter);

        // REC-4/G4: Inject the built-in `finish` tool so the model has an
        // explicit, canonical way to signal completion.  This replaces the
        // implicit "no tool_calls = done" heuristic which could not distinguish
        // genuine completion from a confused model emitting prose mid-task.
        filtered.push(serde_json::json!({
            "name": "finish",
            "description": "Signal task completion or a definitive block. \
                Must be called to end the subagent session. \
                Use status='done' when complete, 'blocked' when stuck, 'error' on failure.",
            "parameters": {
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "Concise summary of what was done or why the task is blocked."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["done", "blocked", "error"],
                        "description": "Final status."
                    },
                    "findings": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of key findings or artefacts."
                    }
                },
                "required": ["summary", "status"]
            }
        }));
        filtered
    };

    let mut messages = messages_init;
    let mut last_text = String::new();
    let mut llm_err: Option<String> = None;
    let next_depth = cfg.depth + 1;
    let allowed_paths = cfg.resolve_allowed_paths(def_opt);

    // G1/REC-2: Rolling fingerprint window for stagnation detection.
    // Stores hashes of (tool_name + args_json) for the last 4 dispatches.
    let mut tool_fingerprints: std::collections::VecDeque<u64> =
        std::collections::VecDeque::with_capacity(5);

    // G5: Per-call dedup cache — maps fingerprint → first iter it was seen.
    let mut tool_dedup: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();

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
            active_plan_json: None,
        },
    );

    // REC-2: Drop guard ensures write-back + row deletion even on panic.
    let mut ephemeral_guard = EphemeralEnvironment::new(
        state.db.clone(),
        subagent_id.clone(),
        parent_agent_id.to_string(),
    );

    // Setup cancellation channel
    let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::channel(1);
    {
        let mut cancellations = state.subagent_cancellations.write().await;
        cancellations.insert(subagent_id.clone(), cancel_tx);
    }

    struct CancelGuard {
        map: std::sync::Arc<
            tokio::sync::RwLock<std::collections::HashMap<String, tokio::sync::mpsc::Sender<()>>>,
        >,
        id: String,
    }
    impl Drop for CancelGuard {
        fn drop(&mut self) {
            let map = self.map.clone();
            let id = self.id.clone();
            // RC3-FIX: Guard against missing runtime context during panic
            // unwind or after runtime shutdown — tokio::task::spawn panics
            // if no runtime is available, causing a double-panic abort.
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let mut cancellations = map.write().await;
                    cancellations.remove(&id);
                });
            }
        }
    }
    let _cancel_guard = CancelGuard {
        map: state.subagent_cancellations.clone(),
        id: subagent_id.clone(),
    };

    // Wall-clock timeout guard (REC-1, pre-existing).
    let timeout_dur = std::time::Duration::from_secs(subagent_timeout_secs());
    let mut cumulative_tokens = 0u64;
    let loop_result = tokio::time::timeout(timeout_dur, async {
        for iter in 0..max_iters {
            if let Some(budget) = cfg.max_tokens_budget {
                let mut iter_input_tokens = 0;
                for m in &messages {
                    if !m.content.is_empty() {
                        iter_input_tokens += cade_ai::count_tokens(&model, &m.content) as u64;
                    }
                    if let Some(tcs) = &m.tool_calls {
                        for tc in tcs {
                            let json = tc.arguments.to_string();
                            if !json.is_empty() {
                                iter_input_tokens += cade_ai::count_tokens(&model, &json) as u64;
                            }
                        }
                    }
                }

                if cumulative_tokens + iter_input_tokens > budget {
                    llm_err = Some(format!(
                        "error: subagent token budget exceeded ({} > {})",
                        cumulative_tokens + iter_input_tokens,
                        budget
                    ));
                    break;
                }
                cumulative_tokens += iter_input_tokens;
            }

            let llm_req = cade_ai::CompletionRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: parent_tool_schemas.clone(),
                max_tokens: 8192,
                reasoning_effort: None,
            };

            let mut fallback_triggered = false;
            let mut fallback_model = String::new();
            let mut resp_opt = None;

            tokio::select! {
                res = state.llm.complete(&llm_req) => {
                    match res {
                        Ok(r) => resp_opt = Some(r),
                        Err(e) => {
                            let e_str = e.to_string();
                            if e_str.contains("404") || e_str.contains("429") {
                                fallback_triggered = true;
                                // Fallback to the parent agent's model
                                fallback_model = parent_model.clone();
                                tracing::warn!("Model {} failed ({}), falling back to {}", model, e_str, fallback_model);
                            } else {
                                llm_err = Some(e_str);
                            }
                        }
                    }
                }
                _ = cancel_rx.recv() => {
                    llm_err = Some("Task cancelled by parent".to_string());
                }
            };

            if fallback_triggered {
                let mut fallback_req = llm_req.clone();
                fallback_req.model = fallback_model.clone();
                tokio::select! {
                    res = state.llm.complete(&fallback_req) => {
                        match res {
                            Ok(r) => {
                                resp_opt = Some(r);
                                model = fallback_model;
                            },
                            Err(e) => {
                                llm_err = Some(format!("Fallback failed: {}", e));
                            }
                        }
                    }
                    _ = cancel_rx.recv() => {
                        llm_err = Some("Task cancelled by parent".to_string());
                    }
                }
            }

            let resp = match resp_opt {
                Some(r) => r,
                None => break,
            };

            if let Some(budget) = cfg.max_tokens_budget {
                if let Some(t) = &resp.content
                    && !t.is_empty() {
                        cumulative_tokens += cade_ai::count_tokens(&model, t) as u64;
                    }
                for tc in &resp.tool_calls {
                    let json = tc.arguments.to_string();
                    if !json.is_empty() {
                        cumulative_tokens += cade_ai::count_tokens(&model, &json) as u64;
                    }
                }
                if cumulative_tokens > budget {
                    llm_err = Some(format!(
                        "error: subagent token budget exceeded ({} > {})",
                        cumulative_tokens, budget
                    ));
                    break;
                }
            }

            // Accumulate the assistant's prose across iterations.
            if let Some(t) = &resp.content
                && !t.is_empty()
            {
                if !last_text.is_empty() {
                    last_text.push_str("\n\n");
                }
                last_text.push_str(t);
            }

            // REC-4/G4: `finish` tool = canonical clean exit.
            // Natural text-only response = implicit done (honour the model's
            // stopping instinct rather than punishing it).
            if resp.tool_calls.is_empty() {
                break;
            }

            // Check for `finish` tool call first — handle before dispatch.
            if let Some(finish_tc) = resp.tool_calls.iter().find(|tc| tc.name == "finish") {
                let summary = finish_tc.arguments["summary"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let status = finish_tc.arguments["status"]
                    .as_str()
                    .unwrap_or("done")
                    .to_string();
                let findings: Vec<String> = finish_tc.arguments["findings"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                if !last_text.is_empty() {
                    last_text.push_str("\n\n");
                }
                last_text.push_str(&summary);
                if !findings.is_empty() {
                    last_text.push_str("\n\nFindings:\n");
                    for f in &findings {
                        last_text.push_str(&format!("- {f}\n"));
                    }
                }

                if status == "error" {
                    llm_err = Some(format!("Subagent finished with status=error: {summary}"));
                }

                // G8/REC-5: Emit finish iter event.
                let iter_ev = serde_json::json!({
                    "message_type": "subagent_iter",
                    "subagent_id": subagent_id,
                    "iter": iter,
                    "tool": "finish",
                    "status": status,
                });
                let _ = emitter.raw_sse_tx()
                    .send(Ok(axum::response::sse::Event::default().data(iter_ev.to_string())))
                    .await;

                break;
            }

            let mut stagnation_detected = false;
            let mut stagnated_tool = String::new();
            let mut stagnated_repeat_count = 0;

            // G8/REC-5: Emit per-iteration observability event for each tool call.
            for tc in &resp.tool_calls {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                tc.name.hash(&mut h);
                tc.arguments.to_string().hash(&mut h);
                let fp = h.finish();

                let iter_ev = serde_json::json!({
                    "message_type": "subagent_iter",
                    "subagent_id": subagent_id,
                    "iter": iter,
                    "tool": tc.name,
                    "args_hash": format!("{fp:x}"),
                });
                let _ = emitter.raw_sse_tx()
                    .send(Ok(axum::response::sse::Event::default().data(iter_ev.to_string())))
                    .await;

                // G5: Per-call dedup — warn if same fingerprint seen before.
                if let Some(first_seen) = tool_dedup.get(&fp) {
                    tracing::debug!(
                        subagent_id = %subagent_id,
                        tool = %tc.name,
                        iter,
                        first_seen,
                        "duplicate tool call fingerprint detected"
                    );
                } else {
                    tool_dedup.insert(fp, iter);
                }

                // G1/REC-2: Stagnation detection — rolling window of last 4 fingerprints.
                tool_fingerprints.push_back(fp);
                if tool_fingerprints.len() > 4 {
                    tool_fingerprints.pop_front();
                }
                let repeat_count = tool_fingerprints.iter().filter(|&&x| x == fp).count();
                if repeat_count >= 3 {
                    tracing::warn!(
                        "Stagnation detected for subagent {}: tool '{}' called with identical arguments {} times. Injecting intervention.",
                        subagent_id, tc.name, repeat_count
                    );
                    stagnation_detected = true;
                    stagnated_tool = tc.name.clone();
                    stagnated_repeat_count = repeat_count;
                    break;
                }
            }

            messages.push(LlmMessage {
                role: "assistant".to_string(),
                content: resp.content.clone().unwrap_or_default(),
                tool_calls: Some(resp.tool_calls.clone()),
                tool_call_id: None,
                images: None,
            });

            if stagnation_detected {
                for tc in &resp.tool_calls {
                    messages.push(LlmMessage {
                        role: "tool".to_string(),
                        content: format!("SYSTEM INTERVENTION: Stagnation detected. You have called '{}' with identical arguments {} times in the last 4 iterations. You are stuck in a doom-loop. Do NOT repeat this call. You MUST re-evaluate your approach, try a completely different strategy, or call the `finish` tool with status='blocked' if you cannot proceed.", stagnated_tool, stagnated_repeat_count),
                        tool_calls: None,
                        tool_call_id: Some(tc.id.clone()),
                        images: None,
                    });
                }
                continue; // Skip actual tool execution, let the model process the intervention
            }

            for tc in &resp.tool_calls {
                let tool_result = if tc.name == "run_subagent" {
                    let mut nested_args = tc.arguments.clone();
                    if let Some(obj) = nested_args.as_object_mut() {
                        obj.insert(
                            "_subagent_depth".to_string(),
                            serde_json::Value::from(next_depth as u64),
                        );
                    }
                    // RC6-NOTE: Box::pin is retained because the inner future
                    // is not Send (non-Send state across await points).  The
                    // recursion depth is hard-capped at CADE_SUBAGENT_MAX_DEPTH
                    // (default 3), and the runtime thread stack is 8 MB (Fix 1),
                    // so this cannot overflow.
                    Box::pin(handle_run_subagent_tool(
                        state,
                        parent_agent_id,
                        &tc.id,
                        &nested_args,
                        emitter.raw_sse_tx(),
                    ))
                    .await
                } else {
                    let storage_backend = std::sync::Arc::new(super::storage_impl::ServerStorageBackend { state: state.clone() });
                    let mut runtime = cade_agent::tools::runtime::ToolRuntime::new(
                        storage_backend,
                        std::sync::Arc::clone(&state.mcp),
                        subagent_id.clone(),
                        std::env::current_dir().unwrap_or_default(),
                    );
                    runtime.allowed_paths = allowed_paths.clone();
                    
                    if let Some(executed) = runtime.execute(tc.id.clone(), &tc.name, &tc.arguments).await {
                        cade_agent::tools::manager::ToolResult {
                            tool_call_id: executed.tool_call_id,
                            tool_name: executed.tool_name,
                            output: executed.output,
                            is_error: executed.is_error,
                        }
                    } else {
                        cade_agent::tools::manager::ToolResult {
                            tool_call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            output: format!("Tool '{}' requires interactive TUI context and is not supported in subagent background loop.", tc.name),
                            is_error: true,
                        }
                    }
                };

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
    })
    .await;

    // REC-1: If the timeout fired, record it as an LLM error.
    if loop_result.is_err() {
        llm_err = Some(format!(
            "Subagent wall-clock timeout after {}s. The task was terminated to free resources.",
            subagent_timeout_secs()
        ));
    }

    let elapsed = start_time.elapsed().as_secs() as u32;

    // G7: Release semaphore permit explicitly before write-back so the slot
    // is freed as early as possible.  The permit's Drop impl is a no-op after
    // this — still safe on panic because OwnedSemaphorePermit::drop() handles it.
    drop(permit);

    // A15 + REC-2: Explicitly run write-back + delete via the guard.
    let writeback_count = ephemeral_guard.write_back_and_delete_async(state).await;

    let (output, is_error) = match llm_err {
        Some(e) => (format!("Subagent error: {e}"), true),
        None => (last_text, false),
    };

    let result_preview: String = output.chars().take(200).collect();
    emitter
        .emit_complete(
            &subagent_id,
            is_error,
            &result_preview,
            elapsed,
            writeback_count,
        )
        .await;

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
        format!("{}…\n[truncated: {} chars total]", head, output.len())
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

pub(super) async fn handle_run_parallel_subagents_tool(
    state: &AppState,
    parent_agent_id: &str,
    tool_call_id: &str,
    args: &serde_json::Value,
    sse_tx: tokio::sync::mpsc::Sender<Result<axum::response::sse::Event, std::convert::Infallible>>,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::tools::manager::ToolResult;

    let mut tasks_val: Vec<serde_json::Value> = Vec::new();

    if let Some(team_id) = args.get("team_id").and_then(|v| v.as_str()) {
        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    tool_name: "run_parallel_subagents".to_string(),
                    output: "error: 'prompt' is required when 'team_id' is provided".to_string(),
                    is_error: true,
                };
            }
        };

        let cwd = std::env::current_dir().unwrap_or_default();
        let all_teams = cade_agent::team::discovery::discover_all_teams(&cwd);
        let team = match cade_agent::team::discovery::resolve_team_def(team_id, &all_teams) {
            Some(t) => t,
            None => {
                return ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    tool_name: "run_parallel_subagents".to_string(),
                    output: format!("error: team not found: {}", team_id),
                    is_error: true,
                };
            }
        };

        for member in &team.members {
            let mut member_args = serde_json::Map::new();
            member_args.insert(
                "prompt".to_string(),
                serde_json::Value::String(prompt.to_string()),
            );
            member_args.insert(
                "description".to_string(),
                serde_json::Value::String(format!(
                    "Team member: {} - {}",
                    member.name, member.description
                )),
            );
            if !member.system_prompt.is_empty() {
                member_args.insert(
                    "system_prompt".to_string(),
                    serde_json::Value::String(member.system_prompt.clone()),
                );
            }
            if let Some(model) = &member.model {
                member_args.insert(
                    "model".to_string(),
                    serde_json::Value::String(model.clone()),
                );
            }
            tasks_val.push(serde_json::Value::Object(member_args));
        }
    } else if let Some(t) = args.get("tasks").and_then(|v| v.as_array()) {
        tasks_val = t.clone();
    } else {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_parallel_subagents".to_string(),
            output: "error: either 'tasks' array OR 'team_id' and 'prompt' are required"
                .to_string(),
            is_error: true,
        };
    }

    if tasks_val.is_empty() {
        return ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "run_parallel_subagents".to_string(),
            output: "error: task list cannot be empty (team may have no members)".to_string(),
            is_error: true,
        };
    }

    // Prepare futures
    let mut futures = Vec::new();
    for (idx, task_args) in tasks_val.iter().enumerate() {
        let task_call_id = format!("{}_{}", tool_call_id, idx);

        let state_c = state.clone();
        let parent_agent_id_c = parent_agent_id.to_string();
        let sse_tx_c = sse_tx.clone();
        let task_args_c = task_args.clone();

        futures.push(Box::pin(async move {
            handle_run_subagent_tool(
                &state_c,
                &parent_agent_id_c,
                &task_call_id,
                &task_args_c,
                sse_tx_c,
            )
            .await
        }));
    }

    // Join all
    let results = futures::future::join_all(futures).await;

    // Aggregate
    let mut aggregated = Vec::new();
    for (idx, tr) in results.into_iter().enumerate() {
        aggregated.push(serde_json::json!({
            "task_index": idx,
            "output": tr.output,
            "is_error": tr.is_error,
        }));
    }

    ToolResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name: "run_parallel_subagents".to_string(),
        output: serde_json::to_string_pretty(&aggregated)
            .unwrap_or_else(|e| format!("error serializing results: {e}")),
        is_error: false, // The parallel executor itself succeeded, individual tasks may have failed
    }
}
pub(super) async fn handle_cancel_subagent_tool(
    state: &AppState,
    tool_call_id: &str,
    args: &serde_json::Value,
) -> cade_agent::tools::manager::ToolResult {
    use cade_agent::tools::manager::ToolResult;

    let subagent_id = match args.get("subagent_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "cancel_subagent".to_string(),
                output: "error: 'subagent_id' is required".to_string(),
                is_error: true,
            };
        }
    };

    let tx_opt = {
        let map = state.subagent_cancellations.read().await;
        map.get(subagent_id).cloned()
    };

    if let Some(tx) = tx_opt {
        // Send cancel signal
        let _ = tx.send(()).await;
        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "cancel_subagent".to_string(),
            output: format!("Cancel signal sent to subagent {subagent_id}"),
            is_error: false,
        }
    } else {
        ToolResult {
            tool_call_id: tool_call_id.to_string(),
            tool_name: "cancel_subagent".to_string(),
            output: format!("error: no active subagent found with ID {subagent_id}"),
            is_error: true,
        }
    }
}

pub(super) async fn smart_memory_merge(
    state: AppState,
    agent_id: String,
    label: String,
    old_value: String,
    new_value: String,
    memory_type: String,
    confidence: f64,
) {
    let prompt = format!(
        "You are a memory merge sub-agent. The parent agent already has a memory block labeled `{label}`. \
         A subagent just returned new information for this exact label. Synthesize the old and new facts into a single coherent block.\n\
         If there are conflicts, resolve them by keeping the most recent/detailed information or by noting the discrepancy.\n\
         Do not include any preamble, just the final merged content.\n\n\
         OLD VALUE:\n{old_value}\n\n\
         NEW VALUE:\n{new_value}"
    );

    // Grab model (cheapest capable)
    let model = cade_store::sqlite::get_agent(&state.db, &agent_id)
        .ok()
        .flatten()
        .and_then(|a| a.compaction_model)
        .unwrap_or_else(|| "claude-3-5-haiku-20241022".to_string());

    let compaction_model = crate::server::consolidation::default_compaction_model(&model);

    let req = cade_ai::CompletionRequest {
        model: compaction_model,
        messages: vec![cade_ai::LlmMessage {
            role: "user".to_string(),
            content: prompt,
            tool_call_id: None,
            tool_calls: None,
            images: None,
        }],
        tools: vec![],
        max_tokens: 4000,
        reasoning_effort: None,
    };

    if let Ok(resp) = state.llm.complete(&req).await
        && let Some(merged) = resp.content
    {
        let desc = "Smart merged after subagent run".to_string();
        let _ = cade_store::sqlite::upsert_memory_block_typed(
            &state.db,
            &agent_id,
            &label,
            merged.trim(),
            Some(&desc),
            None,
            Some(&memory_type),
            Some(confidence),
        );
    }
}
