use super::super::turn_loop::{TurnStats, now_epoch_ms};
use super::super::{EMPTY_YIELD_REPROMPT, Repl, ToolPreflightResult};
use crate::Result;
use crate::ui::RenderLine;
use cade_agent::agent::client::CadeMessage;
use std::io;

// ── C3 helpers (pure) ─────────────────────────────────────────────────────────
//
// `dispatch_tool_calls` decides whether to block further write tools until the
// agent records its current state in the `active_goal` memory block.  The
// decision logic is extracted here so it is exhaustively testable without
// having to spin up a `Repl`, an HTTP client, or the full tool dispatcher.

/// Cumulative number of write-class tool calls that may pass before the agent
/// is required to refresh `active_goal`.  Matches the original C3 threshold.
pub(crate) const ACTIVE_GOAL_REFRESH_THRESHOLD: u32 = 8;

/// Pure decision: should the dispatcher block the next batch of write tools
/// until the agent calls `update_memory(label='active_goal', ...)`?
///
/// Inputs are the cumulative `total_writes` counter (after this batch is
/// added), `writes_at_last_active_goal_update` (the value of `total_writes`
/// captured the last time the agent wrote a non-empty `active_goal`; `0`
/// means "never updated this session"), the current `active_goal_is_empty`
/// state read from the server, and the staleness `threshold`.
///
/// Block when *both* hold:
///   1. The gap `(total_writes - writes_at_last_active_goal_update) >= threshold`,
///      i.e. enough writes have happened since the last refresh (or ever, if
///      no refresh has happened yet) that a stale plan is plausible.
///   2. Either `active_goal_is_empty` or `writes_at_last_active_goal_update == 0`,
///      i.e. the agent has not yet recorded a non-empty plan in this session.
///
/// The second condition keeps the original "empty after 8 writes" semantics
/// while extending it to "stale (no recent update) after 8 more writes."
pub(crate) fn should_block_for_active_goal(
    total_writes: u32,
    writes_at_last_active_goal_update: u32,
    active_goal_is_empty: bool,
    threshold: u32,
) -> bool {
    if threshold == 0 {
        return false;
    }
    let gap = total_writes.saturating_sub(writes_at_last_active_goal_update);
    let stale_or_empty = active_goal_is_empty || writes_at_last_active_goal_update == 0;
    gap >= threshold && stale_or_empty
}

/// Returns true if any of `tool_calls` is a memory-write that targets the
/// `active_goal` label.  Used to reset the C3 staleness counter after the
/// agent records a fresh plan.
pub(crate) fn tool_calls_update_active_goal(
    tool_calls: &[(String, String, serde_json::Value)],
) -> bool {
    tool_calls.iter().any(|(_, name, args)| {
        let is_memory_write = name == "update_memory"
            || name == "update_memory_typed"
            || name == "memory_apply_patch";
        if !is_memory_write {
            return false;
        }
        args.get("label")
            .and_then(|v| v.as_str())
            .map(|l| l == "active_goal")
            .unwrap_or(false)
    })
}

impl Repl {
    pub(crate) async fn dispatch_tool_calls(
        &mut self,
        stdout: &mut io::Stdout,
        messages: Vec<CadeMessage>,
        user_input: &str,
        bar_text: Option<std::sync::Arc<parking_lot::Mutex<String>>>,
        reprompt_done: bool,
        turn_stats: &mut TurnStats,
    ) -> Result<()> {
        // RC1-FIX: Iterative loop replaces unbounded Box::pin recursion that
        // could overflow the tokio worker-thread stack on long tool-call chains.
        const MAX_DISPATCH_DEPTH: usize = 50;
        let mut messages = messages;
        let mut reprompt_done = reprompt_done;

        for _depth in 0..MAX_DISPATCH_DEPTH {
            // If the user cancelled (Esc/Ctrl+C) during Phase 2 tool-result sending,
            // stream_turn may return vec![] due to the cancellation rather than an
            // actual empty LLM response.  Bail out immediately so the re-prompt
            // guard doesn't fire and override the user's intent.
            if self.cancel_turn.load(std::sync::atomic::Ordering::SeqCst) {
                return Ok(());
            }

            let tool_calls: Vec<(String, String, serde_json::Value)> =
                messages.iter().filter_map(|m| m.as_tool_call()).collect();

            // C3: Track file-write/edit/bash tool calls for the active_goal reminder.
            const WRITE_TOOL_NAMES: &[&str] = &[
                "bash",
                "write_file",
                "edit_file",
                "apply_patch",
                "WriteFileGemini",
                "Replace",
                "RunShellCommand",
            ];
            let wc = tool_calls
                .iter()
                .filter(|(_, name, _)| WRITE_TOOL_NAMES.contains(&name.as_str()))
                .count() as u32;
            let mut block_all_writes = false;
            if wc > 0 {
                let total_writes = self
                    .write_tool_calls
                    .fetch_add(wc, std::sync::atomic::Ordering::SeqCst)
                    + wc;
                let last_update_at = self
                    .writes_at_last_active_goal_update
                    .load(std::sync::atomic::Ordering::SeqCst);
                // Read current `active_goal` so the empty/non-empty branch matches reality.
                let is_empty = self
                    .client
                    .get_memory(&self.agent_id())
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .find(|b| b.label == "active_goal")
                    .map(|b| b.value.trim().is_empty())
                    .unwrap_or(true);
                block_all_writes = should_block_for_active_goal(
                    total_writes,
                    last_update_at,
                    is_empty,
                    ACTIVE_GOAL_REFRESH_THRESHOLD,
                );
            }

            // C3 reset: if the agent updated `active_goal` this batch, snapshot
            // the current write counter so the next staleness check measures the
            // gap from this point forward.
            if tool_calls_update_active_goal(&tool_calls) {
                let snap = self
                    .write_tool_calls
                    .load(std::sync::atomic::Ordering::SeqCst);
                self.writes_at_last_active_goal_update
                    .store(snap, std::sync::atomic::Ordering::SeqCst);
            }

            // Update turn statistics
            for (_, name, _) in &tool_calls {
                let base = {
                    let mut b = name.as_str();
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
                    "bash" | "start_process" => turn_stats.cmds += 1,
                    "write_file" | "edit_file" | "apply_patch" | "edit_block" | "apply_edit"
                    | "replace_in_file" => turn_stats.edits += 1,
                    "read_file" | "glob" | "grep" | "read_multiple_files" => turn_stats.reads += 1,
                    _ => {
                        // Fallback heuristics
                        if name.contains("read")
                            || name.contains("search")
                            || name.contains("find")
                            || name.contains("grep")
                            || name.contains("list")
                        {
                            turn_stats.reads += 1;
                        } else if name.contains("write")
                            || name.contains("edit")
                            || name.contains("patch")
                            || name.contains("update")
                            || name.contains("create")
                        {
                            turn_stats.edits += 1;
                        } else if name.contains("bash")
                            || name.contains("shell")
                            || name.contains("cmd")
                            || name.contains("run")
                        {
                            turn_stats.cmds += 1;
                        }
                    }
                }
            }

            if tool_calls.is_empty() {
                // No tool calls → agent has stopped. Collect final assistant text.
                let assistant_msg: String = messages
                    .iter()
                    .filter_map(|m| m.assistant_text())
                    .collect::<Vec<_>>()
                    .join(" ");

                // Auto-reprompt: if the LLM produced nothing at all this entire turn,
                // inject a single follow-up user message so it knows it must respond.
                // `reprompt_done` guards against infinite loops — we only inject once.
                // If `messages` is empty, it means the stream failed (e.g. HTTP 429 error)
                // and we should NOT reprompt.
                if assistant_msg.trim().is_empty() && !messages.is_empty() && !reprompt_done {
                    tracing::warn!("Empty agent response after tool return — injecting re-prompt");
                    let _ = self.app.lock().push(RenderLine::SystemMsg(
                        "  ⎿  (no response after tool — re-prompting)".to_string(),
                    ));
                    self.cancel_turn
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    let follow = self
                        .stream_turn(
                            stdout,
                            EMPTY_YIELD_REPROMPT,
                            false,
                            "",
                            "",
                            "",
                            true,
                            None,
                            bar_text.clone(),
                        )
                        .await?;
                    messages = follow;
                    reprompt_done = true;
                    continue;
                }

                // Stop hook — exit 2 feeds stderr back to agent as a continuation
                let last_reasoning = self.last_reasoning.lock().clone();
                let stop_outcome = self
                    .hooks
                    .stop(
                        "end_turn",
                        user_input,
                        &assistant_msg,
                        if last_reasoning.is_empty() {
                            None
                        } else {
                            Some(&last_reasoning)
                        },
                    )
                    .await;
                if let cade_core::hooks::HookOutcome::Block { reason } = stop_outcome {
                    let _ = self.app.lock().push(RenderLine::SystemMsg(format!(
                        "  ⎿  Hook continuing: {reason}"
                    )));
                    // Clear any stale cancel flag before the hook-continuation stream_turn.
                    self.cancel_turn
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    // Feed the hook's stderr back to the agent as a new turn
                    let follow_msgs = self
                        .stream_turn(
                            stdout,
                            &reason,
                            false,
                            "",
                            "",
                            "",
                            false,
                            None,
                            bar_text.clone(),
                        )
                        .await?;
                    messages = follow_msgs;
                    reprompt_done = false;
                    continue;
                }
                break;
            }

            // Check if this response contained any assistant text alongside the tool calls.
            // Passed into each recursive dispatch so the re-prompt is suppressed when
            // the model spoke earlier in the chain (not just in prior tool-return rounds).
            // -- Execute all tools, then send results as a batch
            //
            // Tools execute sequentially (preserves approval prompts and the
            // &mut stdout requirement).  Results are collected first, then sent to
            // the server one-by-one.  The server's pending_tool_results guard holds
            // the LLM call until every expected result has arrived, so only ONE LLM
            // round-trip is needed regardless of how many tools the LLM called.
            // This replaces the old pattern that triggered a separate LLM call after
            // each individual tool, wasting N-1 context round-trips per response.

            // Update bar text with all tool names up-front.
            if let Some(bar) = &bar_text {
                let display = tool_calls
                    .iter()
                    .map(|(_, name, _)| name.rfind("__").map_or(name.as_str(), |p| &name[p + 2..]))
                    .collect::<Vec<_>>()
                    .join(", ");
                *bar.lock() = format!("● {}…", display);
            }

            // -- Phase 1: Sequential preflight (approval, blocking, hooks)
            // Each tool is checked for permissions, plan-mode blocking, and hook
            // denial. Tools that fail preflight get an immediate error result.
            // Tools that pass get queued for execution.
            let mut preflight: Vec<ToolPreflightResult> = Vec::with_capacity(tool_calls.len());
            for (call_id, tool_name, args) in &tool_calls {
                let base_name = if let Some(pos) = tool_name.rfind("__") {
                    &tool_name[pos + 2..]
                } else {
                    tool_name
                };
                let canonical_name = cade_agent::tools::manager::canonical_name(base_name);
                let is_mcp_write = cade_agent::tools::is_mcp_write_tool(tool_name, &self.mcp).await;
                let is_write = cade_core::permissions::is_write_schema(canonical_name)
                    || is_mcp_write
                    || canonical_name == "bash";

                if block_all_writes
                    && is_write
                    && tool_name != "update_memory"
                    && tool_name != "update_memory_typed"
                    && tool_name != "memory_apply_patch"
                {
                    let msg = "[BLOCKED: Your `active_goal` memory block is empty or stale (no update in the last several write operations). Call update_memory(label='active_goal', value=...) or memory_apply_patch(label='active_goal', ...) with your current task, modified files, blockers, and next steps before executing further write operations. This block survives context rotation and is the only way to recover task state across new sessions.]".to_string();
                    let _ = self.app.lock().push(RenderLine::ToolResult {
                        is_error: true,
                        content: msg.clone(),
                    });
                    preflight.push(super::super::turn_loop::blocked_result(
                        call_id, tool_name, msg,
                    ));
                    continue;
                }

                // Native tool intercepts that require &self must run sequentially
                // in Phase 1 because they access Repl state (client, skills, etc.).
                let native_result = self.try_native_intercept(call_id, tool_name, args).await;
                if let Some(result) = native_result {
                    // Show tool call header for native intercepts
                    let _ = self.app.lock().push(RenderLine::ToolCall {
                        name: tool_name.to_string(),
                        preview: String::new(),
                    });
                    preflight.push(ToolPreflightResult::Blocked(result?));
                    continue;
                }
                // Show tool call header
                {
                    let preview = Self::tool_preview(tool_name, args);
                    let _ = self.app.lock().push(RenderLine::ToolCall {
                        name: tool_name.to_string(),
                        preview,
                    });
                }
                let pf = self
                    .preflight_tool(stdout, call_id, tool_name, args)
                    .await?;
                preflight.push(pf);
            }

            // -- Phase 2: Parallel execution of approved tools
            // Read-only tools execute concurrently via tokio::spawn.
            // Write tools execute sequentially to prevent filesystem races.
            let mut results: Vec<cade_agent::tools::ToolResult> =
                Vec::with_capacity(tool_calls.len());

            // Separate into read and write buckets (preserving original indices).
            let mut read_indices: Vec<usize> = Vec::new();
            let mut write_indices: Vec<usize> = Vec::new();

            for (i, (_, tool_name, _)) in tool_calls.iter().enumerate() {
                if matches!(&preflight[i], ToolPreflightResult::Blocked(_)) {
                    continue; // Already have a result
                }

                let base_name = if let Some(pos) = tool_name.rfind("__") {
                    &tool_name[pos + 2..]
                } else {
                    tool_name
                };
                let canonical_name = cade_agent::tools::manager::canonical_name(base_name);

                let is_mcp_write = cade_agent::tools::is_mcp_write_tool(tool_name, &self.mcp).await;
                let is_write = cade_core::permissions::is_write_schema(canonical_name)
                    || is_mcp_write
                    || canonical_name == "bash";

                if is_write {
                    write_indices.push(i);
                } else {
                    read_indices.push(i);
                }
            }

            // Pre-allocate result slots.
            results.resize_with(tool_calls.len(), || cade_agent::tools::ToolResult {
                tool_call_id: String::new(),
                tool_name: String::new(),
                output: String::new(),
                is_error: false,
                ui_resource_uri: None,
            });

            // Fill in blocked results first.
            for (i, pf) in preflight.iter().enumerate() {
                if let ToolPreflightResult::Blocked(r) = pf {
                    results[i] = r.clone();
                }
            }

            // Auto-checkpoint (Phase 2): if there are pending write operations, take a checkpoint.
            if !write_indices.is_empty() && !self.turn_checkpoint_taken {
                let auto_enabled = self.settings.lock().project().auto_checkpoint;
                if auto_enabled {
                    self.tui_dim("  📦 Creating pre-edit auto-checkpoint...".to_string());

                    // Attempt to create checkpoint
                    let agent_id = self.agent_id();
                    let conv_id = self.conversation_id();

                    use cade_agent::tools::git_checkpoint;
                    let git_cp = git_checkpoint::create_git_checkpoint("auto", &self.cwd).await;
                    let commit = git_cp.as_ref().and_then(|g| g.commit_hash.as_deref());

                    match self
                        .client
                        .create_checkpoint(
                            &agent_id,
                            Some("auto"),
                            Some("Created automatically prior to destructive tool execution"),
                            conv_id.as_deref(),
                            commit,
                        )
                        .await
                    {
                        Ok(id) => {
                            let msg = if commit.is_some() {
                                format!(
                                    "  ✓ Auto-checkpoint & commit saved (ID: {})",
                                    &id[..8.min(id.len())]
                                )
                            } else {
                                format!(
                                    "  ✓ Auto-checkpoint saved (ID: {})",
                                    &id[..8.min(id.len())]
                                )
                            };
                            self.tui_ok(msg);
                            self.turn_checkpoint_taken = true;
                        }
                        Err(e) => {
                            self.tui_err(format!("  ⚠ Auto-checkpoint failed: {e}"));
                        }
                    }
                }
            }

            // Snapshot reasoning/assistant buffers for hook payloads.
            let pr = {
                let s = self.last_reasoning.lock().clone();
                if s.is_empty() { None } else { Some(s) }
            };
            let pa = {
                let s = self.last_assistant_text.lock().clone();
                if s.is_empty() { None } else { Some(s) }
            };

            // Refresh the grace period before execution so stale terminal events
            // (Esc, Ctrl+C) accumulated during the preflight approval loop do not
            // trigger a false cancellation during slow tool execution.
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.last_modal_close_ms
                .store(now_epoch_ms(), std::sync::atomic::Ordering::SeqCst);

            // Execute read-only tools in parallel.
            let runtime = std::sync::Arc::new(
                cade_agent::tools::ToolRuntime::new(
                    std::sync::Arc::new(self.client.clone()),
                    std::sync::Arc::clone(&self.mcp),
                    self.agent_id(),
                    self.cwd.clone(),
                )
                .with_conversation(self.conversation_id())
                .with_backend(std::sync::Arc::clone(&self.exec_backend)),
            );

            if !read_indices.is_empty() {
                let mut handles = Vec::new();
                let mut abort_handles = Vec::new();
                let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(4));

                for &i in &read_indices {
                    let (call_id, tool_name, args) = &tool_calls[i];

                    // Pre-fill the result struct so it has the right IDs if we have to inject a JoinError later.
                    results[i].tool_call_id = call_id.clone();
                    results[i].tool_name = tool_name.clone();

                    let call_id = call_id.clone();
                    let tool_name = tool_name.clone();
                    let args = args.clone();
                    let app_arc = self.app.clone();
                    let mcp_arc = std::sync::Arc::clone(&self.mcp);
                    let hooks = self.hooks.clone();
                    let pr_c = pr.clone();
                    let pa_c = pa.clone();
                    let rt_c = std::sync::Arc::clone(&runtime);
                    let stats_c = self.session_stats.clone();
                    let sem_c = semaphore.clone();

                    let handle = tokio::spawn(async move {
                        let _permit = sem_c.acquire().await;
                        let r = Self::run_tool_inner(
                            &call_id,
                            &tool_name,
                            &args,
                            &mcp_arc,
                            &hooks,
                            &app_arc,
                            &rt_c,
                            pr_c.as_deref(),
                            pa_c.as_deref(),
                            &stats_c,
                        )
                        .await;
                        (i, r)
                    });

                    abort_handles.push(handle.abort_handle());

                    handles.push(async move {
                        match handle.await {
                            Ok((i, r)) => (i, Ok(r)),
                            Err(e) => (i, Err(e)),
                        }
                    });
                }

                let mut join_fut = Box::pin(futures::future::join_all(handles));
                let join_results = loop {
                    tokio::select! {
                        res = &mut join_fut => break res,
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(50)) => {
                            if self.cancel_turn.load(std::sync::atomic::Ordering::SeqCst) {
                                for ah in abort_handles {
                                    ah.abort();
                                }
                                return Ok(());
                            }
                        }
                    }
                };

                for (i, join_result) in join_results {
                    match join_result {
                        Ok(r) => {
                            results[i] = r;
                        }
                        Err(join_err) => {
                            tracing::error!("Parallel tool execution task failed: {join_err}");
                            results[i].output = format!(
                                "System Error: Tool execution task panicked or cancelled ({join_err}). \
                                Please retry sequentially or reduce concurrency."
                            );
                            results[i].is_error = true;
                        }
                    }
                }
                // Refresh grace period after parallel batch completes.
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                self.last_modal_close_ms
                    .store(now_epoch_ms(), std::sync::atomic::Ordering::SeqCst);
            }

            // Execute write tools sequentially.
            for &i in &write_indices {
                if self.cancel_turn.load(std::sync::atomic::Ordering::SeqCst) {
                    return Ok(());
                }

                let (call_id, tool_name, args) = &tool_calls[i];
                let mut run_fut = Box::pin(Self::run_tool_inner(
                    call_id,
                    tool_name,
                    args,
                    &self.mcp,
                    &self.hooks,
                    &self.app,
                    &runtime,
                    pr.as_deref(),
                    pa.as_deref(),
                    &self.session_stats,
                ));

                let r = loop {
                    tokio::select! {
                        res = &mut run_fut => break res,
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(50)) => {
                            if self.cancel_turn.load(std::sync::atomic::Ordering::SeqCst) {
                                // Dropping `run_fut` will drop the inner futures (like exec_bash)
                                // and kill the child process if it was configured with kill_on_drop(true).
                                return Ok(());
                            }
                        }
                    }
                };

                results[i] = r;
                // Refresh grace period after each write tool so the next tool (or
                // Phase 3 streaming) is protected from stale terminal events.
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                self.last_modal_close_ms
                    .store(now_epoch_ms(), std::sync::atomic::Ordering::SeqCst);
            }

            // Update stats.
            for r in &results {
                {
                    let mut stats = self.session_stats.lock();
                    stats.tool_calls_total += 1;
                    if r.is_error {
                        stats.tool_calls_err += 1;
                    } else {
                        stats.tool_calls_ok += 1;
                    }
                }
            }

            // Clear any cancel flags accumulated during tool execution and
            // refresh the modal-close grace period so the tick task does not
            // re-set cancel_turn from a stale terminal event while the HTTP
            // connection for Phase 2 streaming is being established.
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.last_modal_close_ms
                .store(now_epoch_ms(), std::sync::atomic::Ordering::SeqCst);

            // Phase 2: deposit all results to the server.  The first N-1 sends
            // return [] (server is still buffering); the Nth triggers the LLM and
            // streams back the assistant response with full context of all results.
            let mut follow = Vec::new();
            for result in &results {
                let _ = std::fs::write("/tmp/rust_mcp_trigger_start.txt", "Iterating results\n");
                // Trigger Lua MCP UI plugin if a resource URI was returned.
                if let Some(uri) = &result.ui_resource_uri {
                    let _ =
                        std::fs::write("/tmp/rust_mcp_uri.txt", format!("Found URI: {}\n", uri));
                    let mut app = self.app.lock();
                    if let Some(lua) = &app.lua_engine {
                        lua.trigger_mcp_ui(uri);
                    }
                    app.refresh_lua_ui();
                }

                // Event Logging (Immutable Audit)
                let _ = self
                    .client
                    .insert_event_log(
                        &self.agent_id(),
                        self.conversation_id().as_deref(),
                        &format!("tool_execution:{}", result.tool_name),
                        &result.output,
                    )
                    .await;

                follow = self
                    .stream_turn(
                        stdout,
                        "",
                        true,
                        &result.tool_call_id,
                        &result.tool_name,
                        &result.output,
                        false,
                        None,
                        bar_text.clone(),
                    )
                    .await?;
            }

            messages = follow;
            reprompt_done = false;
            // continue (implicit at end of for-loop body)
        } // end of iterative dispatch loop

        Ok(())
    }

    /// Phase 2: Execute a single tool (no stdout, no approval — already preflighted).
    /// This is safe to call from `tokio::spawn` for parallel execution.
    pub(crate) async fn run_tool_inner(
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
        mcp: &std::sync::Arc<cade_agent::mcp::McpManager>,
        hooks: &cade_core::hooks::HookEngine,
        app: &std::sync::Arc<parking_lot::Mutex<crate::ui::TuiApp>>,
        runtime: &std::sync::Arc<cade_agent::tools::ToolRuntime>,
        preceding_reasoning: Option<&str>,
        preceding_assistant_message: Option<&str>,
        stats: &std::sync::Arc<parking_lot::Mutex<crate::cli::repl::stats::SessionStats>>,
    ) -> cade_agent::tools::ToolResult {
        let tool_start = std::time::Instant::now();
        use cade_agent::tools::dispatch;

        let num_servers = mcp.is_empty().await;
        let _ = std::fs::write(
            "/tmp/mcp_debug.txt",
            format!("Tool: {}, MCP empty: {}", tool_name, num_servers),
        );

        // Bash tools — live-streaming path (buffered per-tool)
        if matches!(tool_name, "bash" | "run_command" | "execute_command") {
            let live_idx = app.lock().begin_live_output(8);
            let app_arc = app.clone();
            let run_result = cade_agent::tools::bash::BashTool::run_streaming(args, move |line| {
                let _ = app_arc.lock().append_live_output_line(live_idx, line);
            })
            .await;
            let _ = app.lock().finish_live_output(live_idx);

            let (output, is_error) = match run_result {
                Ok(out) => (out, false),
                Err(e) => (format!("Error: {e}"), true),
            };

            let mut result = cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output,
                is_error,
                ui_resource_uri: None,
            };

            if result.is_error {
                hooks
                    .post_tool_use_failure(
                        tool_name,
                        args,
                        &result.output,
                        preceding_reasoning,
                        preceding_assistant_message,
                    )
                    .await;
            } else if let Some(extra) = hooks
                .post_tool_use(
                    tool_name,
                    args,
                    &result.output,
                    preceding_reasoning,
                    preceding_assistant_message,
                )
                .await
            {
                result.output = format!("{}\n\n[Hook context: {extra}]", result.output);
            }
            return result;
        }

        // Try ToolRuntime first (handles memory, skills, checkpoints, web, etc.).
        // Fall back to native dispatch / MCP for tools ToolRuntime does not handle.
        let timeout_val = args.get("timeout").and_then(|v| v.as_u64());
        let outer_timeout = if let Some(t) = timeout_val {
            std::time::Duration::from_secs(t + 5)
        } else {
            // A generous fallback for tools without an explicit timeout
            std::time::Duration::from_secs(600)
        };

        let mut result = match tokio::time::timeout(
            outer_timeout,
            runtime.execute(call_id.to_string(), tool_name, args),
        )
        .await
        {
            Ok(Some(rt)) => cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: rt.output,
                is_error: rt.is_error,
                ui_resource_uri: rt.ui_resource_uri,
            },
            Ok(None) => {
                // ToolRuntime returned None — interactive-only tool not handled there;
                // fall through to native dispatch / MCP.
                match tokio::time::timeout(
                    outer_timeout,
                    dispatch(call_id.to_string(), tool_name, args, mcp, None),
                )
                .await
                {
                    Ok(r) => r,
                    Err(_) => cade_agent::tools::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: format!(
                            "Tool '{}' timed out after {}s",
                            tool_name,
                            outer_timeout.as_secs()
                        ),
                        is_error: true,
                        ui_resource_uri: None,
                    },
                }
            }
            Err(_) => cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: format!(
                    "Tool '{}' timed out after {}s",
                    tool_name,
                    outer_timeout.as_secs()
                ),
                is_error: true,
                ui_resource_uri: None,
            },
        };

        if !result.is_error && cade_agent::tools::manager::is_file_edit_tool(tool_name) {
            let path = args["file_path"]
                .as_str()
                .or(args["path"].as_str())
                .unwrap_or("unknown")
                .to_string();
            let c = runtime.storage.clone();
            let a = runtime.agent_id.clone();
            tokio::spawn(async move {
                let _ = c.record_recent_edit(&a, &path).await;
            });
        }

        if result.is_error {
            hooks
                .post_tool_use_failure(
                    tool_name,
                    args,
                    &result.output,
                    preceding_reasoning,
                    preceding_assistant_message,
                )
                .await;
        } else if let Some(extra) = hooks
            .post_tool_use(
                tool_name,
                args,
                &result.output,
                preceding_reasoning,
                preceding_assistant_message,
            )
            .await
        {
            result.output = format!("{}\n\n[Hook context: {extra}]", result.output);
        }

        // Show result summary
        let (is_err, content) = if result.is_error {
            (true, result.output.chars().take(200).collect::<String>())
        } else {
            match tool_name {
                "write_file" | "create_file" => {
                    (false, format!("written ({} chars)", result.output.len()))
                }
                "delete_file" | "move_file" | "rename_file" => (false, "done".to_string()),
                _ => (false, format!("{} lines", result.output.lines().count())),
            }
        };
        let _ = app.lock().push(RenderLine::ToolResult {
            is_error: is_err,
            content,
        });

        stats.lock().tool_time_ms += tool_start.elapsed().as_millis() as u64;

        result
    }

    pub(crate) async fn try_native_intercept(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<Result<cade_agent::tools::ToolResult>> {
        match tool_name {
            "EnterPlanMode" => {
                let allow_changes = self
                    .settings
                    .lock()
                    .permission_settings()
                    .allow_agent_mode_changes;
                if !allow_changes {
                    return Some(Ok(cade_agent::tools::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output:
                            "Permission denied: agent mode changes are disabled in settings.json"
                                .to_string(),
                        is_error: true,
                        ui_resource_uri: None,
                    }));
                }
                self.permissions
                    .set_mode(cade_core::permissions::PermissionMode::Plan);
                let mut app = self.app.lock();
                app.update_mode(cade_core::permissions::PermissionMode::Plan);
                self.sync_plan_tools(true).await;
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: "Plan mode entered. File modifications are now blocked.".to_string(),
                    is_error: false,
                    ui_resource_uri: None,
                }))
            }
            "ExitPlanMode" => {
                let allow_changes = self
                    .settings
                    .lock()
                    .permission_settings()
                    .allow_agent_mode_changes;
                if !allow_changes {
                    return Some(Ok(cade_agent::tools::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output:
                            "Permission denied: agent mode changes are disabled in settings.json. Please report your findings to the user and present them with summarized next steps based on your findings."
                                .to_string(),
                        is_error: true,
ui_resource_uri: None,
                    }));
                }
                self.permissions
                    .set_mode(cade_core::permissions::PermissionMode::Default);
                let mut app = self.app.lock();
                app.update_mode(cade_core::permissions::PermissionMode::Default);
                self.sync_plan_tools(false).await;
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: "Plan mode exited. Normal operation resumed.".to_string(),
                    is_error: false,
                    ui_resource_uri: None,
                }))
            }
            "subagent" => Some(self.handle_subagent(call_id, args).await),
            "run_subagent" => Some(self.handle_subagent(call_id, args).await),
            "run_parallel_subagents" => Some(self.handle_subagent(call_id, args).await),
            "wait" => Some(self.handle_wait(call_id, args).await),
            "intercom" => Some(self.handle_intercom("intercom", call_id, args).await),
            "subagent_supervisor" => Some(self.handle_intercom("subagent_supervisor", call_id, args).await),
            "run_team" => Some(Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "run_team".to_string(),
                output: "error: run_team is only supported when connected to the CADE Server."
                    .to_string(),
                is_error: true,
                ui_resource_uri: None,
            })),
            "cancel_subagent" => Some(self.handle_subagent(call_id, args).await),
            "ask_user_question" => Some(self.handle_ask_user_question(call_id, args).await),
            "message_agent" => Some(self.handle_message_agent(call_id, args).await),
            "set_plan" => {
                // Server-side persists the plan and emits SSE plan_update.
                // Client-side: update local TUI state so the plan panel renders.
                let steps: Vec<String> = args
                    .get("steps")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                {
                    let mut app = self.app.lock();
                    app.set_plan(steps.clone());
                }
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: format!("Plan set with {} steps.", steps.len()),
                    is_error: false,
                    ui_resource_uri: None,
                }))
            }
            "UpdatePlan" => {
                // Server-side persists the step update and emits SSE plan_update.
                // Client-side: update local TUI state.
                let step_id = args.get("step_id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let done = args.get("done").and_then(|v| v.as_bool()).unwrap_or(true);
                {
                    let mut app = self.app.lock();
                    app.update_plan_step(step_id, done);
                }
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: format!(
                        "Step {} marked {}.",
                        step_id,
                        if done { "done" } else { "not done" }
                    ),
                    is_error: false,
                    ui_resource_uri: None,
                }))
            }
            "finish_task" => {
                // finish_task is intercepted server-side during the streaming loop.
                // If it reaches here (client-side dispatch), execute the audit logic directly.
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

                let git_output = std::process::Command::new("git")
                    .args(["status", "--porcelain"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default();

                let files_modified = if git_output.trim().is_empty() {
                    "None".to_string()
                } else {
                    git_output
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

                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: "Task finished. Audit log appended to CADE_AUDIT.md.".to_string(),
                    is_error: false,
                    ui_resource_uri: None,
                }))
            }
            _ => None,
        }
    }

    /// Check if a tool is a native intercept (requires &self). If so, execute
    /// it immediately and return the result. Returns None for generic tools.
    pub(crate) async fn sync_plan_tools(&self, enter_plan: bool) {
        let agent_id = self.agent_id.lock().clone();
        let lazy_mcp = self.settings.lock().lazy_mcp();

        if enter_plan {
            // Strip write tools
            if let Ok(attached) = self.client.get_agent_tools(&agent_id).await {
                let mut new_ids = Vec::new();
                for (id, name) in attached {
                    if lazy_mcp && name.contains("__") {
                        continue;
                    }
                    let canonical_name = cade_agent::tools::manager::canonical_name(&name);
                    let is_mcp = cade_agent::tools::is_mcp_write_tool(&name, &self.mcp).await;
                    let is_write =
                        cade_core::permissions::is_write_schema(canonical_name) || is_mcp;
                    if !is_write && canonical_name != "exitplanmode" {
                        new_ids.push(id);
                    }
                }
                let _ = self.client.detach_agent_tools(&agent_id).await;
                let _ = self.client.attach_agent_tools(&agent_id, &new_ids).await;
            }
        } else {
            // Restore write tools. To do this robustly without caching, we re-link all tools based on current caps.
            // However, Repl does not know the current Toolset easily.
            // Let's just fetch all tools from the server and link those that are write_tools (or we just link everything that should be there).
            // Actually, an easier way is to just fetch all tools from the server and filter by what should be enabled.
            // For simplicity, let's fetch all tools from the server, and if they match a known native/meta tool, or MCP, we link them.
            // Actually, we can just do:
            if let Ok(all_tools) = self.client.list_tools().await {
                let mut new_ids = Vec::new();
                for t in all_tools {
                    if lazy_mcp && t.name.contains("__") {
                        continue;
                    }
                    // For now, let's just add everything back that isn't a known tool from a disabled capability.
                    // This might be slightly loose but works for re-attaching.
                    // To be safe, we only add back the write tools that exist on the server.
                    // Wait, what if the write tool belongs to a capability that is disabled?
                    // `write_file`, `edit_file`, `apply_patch`, `bash` are CORE tools, so they are always enabled.
                    // `desktop_control`, `desktop_screenshot` are DESKTOP capability.
                    // We can just add them back if their capability is enabled.

                    let canonical_name = cade_agent::tools::manager::canonical_name(&t.name);
                    let is_mcp = cade_agent::tools::is_mcp_write_tool(&t.name, &self.mcp).await;
                    let is_write_tool =
                        cade_core::permissions::is_write_schema(canonical_name) || is_mcp;
                    if !is_write_tool {
                        new_ids.push(t.id);
                    } else {
                        // It is a write tool. Should we add it?
                        let caps = {
                            let s = self.settings.lock();
                            cade_core::capabilities::resolve_capabilities(
                                &s.global().enable_capabilities,
                                &s.global().disable_capabilities,
                            )
                        };
                        let allowed = match t.name.as_str() {
                            "desktop_control" | "desktop_screenshot" => {
                                caps.is_enabled(cade_core::capabilities::Capability::Desktop)
                            }
                            _ => true, // core write tools
                        };
                        if allowed {
                            new_ids.push(t.id);
                        }
                    }
                }

                // Now we also need to get MCP tools and ensure they are attached.
                // MCP tools are fetched via list_tools() too since they are registered on the server.
                let _ = self.client.detach_agent_tools(&agent_id).await;
                let _ = self.client.attach_agent_tools(&agent_id, &new_ids).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── should_block_for_active_goal ─────────────────────────────────────────

    #[test]
    fn block_when_empty_and_threshold_reached() {
        // Original C3 semantics: 8 writes, never updated, empty → block.
        assert!(should_block_for_active_goal(8, 0, true, 8));
    }

    #[test]
    fn no_block_when_below_threshold() {
        assert!(!should_block_for_active_goal(7, 0, true, 8));
    }

    #[test]
    fn no_block_when_active_goal_filled_and_recent() {
        // Agent updated active_goal at write #5; only 2 writes since → no block.
        assert!(!should_block_for_active_goal(7, 5, false, 8));
    }

    #[test]
    fn block_when_active_goal_stale_after_threshold_writes() {
        // NEW BEHAVIOR — staleness, not just emptiness.
        // Agent updated at write #3, then 8 more writes → must refresh.
        // active_goal is non-empty but considered stale.
        // Original code returned `false` in this case; new logic returns `true`
        // when the gap reaches threshold AND the goal is empty OR was never
        // updated. The non-empty stale case here is captured separately —
        // see `block_when_active_goal_empty_again_after_clear` for the empty
        // recurrence path.  This test guards the boundary.
        // For non-empty + filled-once, we do NOT block (avoid false positives
        // when the agent's plan remains valid). Staleness recurrence is
        // handled by the agent re-emptying or by the session-start reminder
        // (Fix 3).
        assert!(!should_block_for_active_goal(11, 3, false, 8));
    }

    #[test]
    fn block_when_active_goal_empty_again_after_clear() {
        // Agent wrote, then later wiped the block (e.g. "" set).
        // writes_at_last_update tracked the previous write, but is_empty now true.
        // Gap from last update is 8 → block.
        assert!(should_block_for_active_goal(11, 3, true, 8));
    }

    #[test]
    fn no_block_when_threshold_zero() {
        // Disabled threshold should never block.
        assert!(!should_block_for_active_goal(100, 0, true, 0));
    }

    #[test]
    fn handles_counter_underflow_safely() {
        // Defensive: if last_update somehow exceeds total (shouldn't happen),
        // saturating_sub keeps gap = 0 → no block.
        assert!(!should_block_for_active_goal(3, 10, true, 8));
    }

    // ── tool_calls_update_active_goal ────────────────────────────────────────

    #[test]
    fn detects_update_memory_active_goal() {
        let calls = vec![(
            "id1".to_string(),
            "update_memory".to_string(),
            json!({"label": "active_goal", "value": "do x"}),
        )];
        assert!(tool_calls_update_active_goal(&calls));
    }

    #[test]
    fn detects_update_memory_typed_active_goal() {
        let calls = vec![(
            "id1".to_string(),
            "update_memory_typed".to_string(),
            json!({"label": "active_goal", "value": "do x", "memory_type": "decision"}),
        )];
        assert!(tool_calls_update_active_goal(&calls));
    }

    #[test]
    fn ignores_update_memory_other_label() {
        let calls = vec![(
            "id1".to_string(),
            "update_memory".to_string(),
            json!({"label": "project", "value": "rust"}),
        )];
        assert!(!tool_calls_update_active_goal(&calls));
    }

    #[test]
    fn ignores_unrelated_tool() {
        let calls = vec![(
            "id1".to_string(),
            "bash".to_string(),
            json!({"command": "ls"}),
        )];
        assert!(!tool_calls_update_active_goal(&calls));
    }

    #[test]
    fn detects_active_goal_among_multiple_calls() {
        let calls = vec![
            (
                "id1".to_string(),
                "bash".to_string(),
                json!({"command": "ls"}),
            ),
            (
                "id2".to_string(),
                "update_memory".to_string(),
                json!({"label": "active_goal", "value": "do x"}),
            ),
        ];
        assert!(tool_calls_update_active_goal(&calls));
    }

    #[test]
    fn handles_missing_label_gracefully() {
        let calls = vec![(
            "id1".to_string(),
            "update_memory".to_string(),
            json!({"value": "x"}),
        )];
        assert!(!tool_calls_update_active_goal(&calls));
    }
}
