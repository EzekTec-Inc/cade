use super::super::{EMPTY_YIELD_REPROMPT, Repl, ToolPreflightResult};
use crate::Result;
use crate::ui::RenderLine;
use cade_agent::agent::client::CadeMessage;
use std::io;
use super::super::turn_loop::{now_epoch_ms, TurnStats};

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
        // If the user cancelled (Esc/Ctrl+C) during Phase 2 tool-result sending,
        // stream_turn may return vec![] due to the cancellation rather than an
        // actual empty LLM response.  Bail out immediately so the re-prompt
        // guard doesn't fire and override the user's intent.
        if self.cancel_turn.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }

        let tool_calls: Vec<(String, String, serde_json::Value)> =
            messages.iter().filter_map(|m| m.as_tool_call()).collect();

        // C3: Track file-write/edit/bash tool calls for the working_set reminder.
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
        if wc > 0 {
            self.write_tool_calls
                .fetch_add(wc, std::sync::atomic::Ordering::SeqCst);
        }

        // Update turn statistics
        for (_, name, _) in &tool_calls {
            match name.as_str() {
                "bash" | "RunShellCommand" | "desktop-commander__start_process" => {
                    turn_stats.cmds += 1
                }
                "write_file"
                | "edit_file"
                | "apply_patch"
                | "WriteFileGemini"
                | "Replace"
                | "desktop-commander__write_file"
                | "desktop-commander__edit_block" => turn_stats.edits += 1,
                "read_file"
                | "ReadFileGemini"
                | "glob"
                | "GlobGemini"
                | "grep"
                | "SearchFileContent"
                | "desktop-commander__read_file"
                | "desktop-commander__read_multiple_files" => turn_stats.reads += 1,
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
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::SystemMsg(
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
                        true,
                        None,
                        bar_text.clone(),
                    )
                    .await?;
                Box::pin(
                    self.dispatch_tool_calls(
                        stdout, follow, user_input, bar_text, true, turn_stats,
                    ),
                )
                .await?;
                return Ok(());
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
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::SystemMsg(format!(
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
                        false,
                        None,
                        bar_text.clone(),
                    )
                    .await?;
                Box::pin(self.dispatch_tool_calls(
                    stdout,
                    follow_msgs,
                    user_input,
                    bar_text,
                    false,
                    turn_stats,
                ))
                .await?;
            }
            return Ok(());
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
            // Native tool intercepts that require &self must run sequentially
            // in Phase 1 because they access Repl state (client, skills, etc.).
            let native_result = self.try_native_intercept(call_id, tool_name, args).await;
            if let Some(result) = native_result {
                // Show tool call header for native intercepts
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::ToolCall {
                        name: tool_name.to_string(),
                        preview: String::new(),
                    });
                preflight.push(ToolPreflightResult::Blocked(result?));
                continue;
            }
            // Show tool call header
            {
                let preview = Self::tool_preview(tool_name, args);
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::ToolCall {
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
        let mut results: Vec<cade_agent::tools::ToolResult> = Vec::with_capacity(tool_calls.len());

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

            let is_mcp_write = cade_agent::tools::is_mcp_write_tool(canonical_name, &self.mcp).await;
            let is_write = cade_core::permissions::is_write_schema(canonical_name) || is_mcp_write || canonical_name == "bash";

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
        });

        // Fill in blocked results first.
        for (i, pf) in preflight.iter().enumerate() {
            if let ToolPreflightResult::Blocked(r) = pf {
                results[i] = r.clone();
            }
        }

        // Auto-checkpoint (Phase 2): if there are pending write operations, take a checkpoint.
        if !write_indices.is_empty() && !self.turn_checkpoint_taken {
            let auto_enabled = self
                .settings
                .lock()
                .project()
                .auto_checkpoint;
            if auto_enabled {
                self.tui_dim("  📦 Creating pre-edit auto-checkpoint...".to_string());

                // Attempt to create checkpoint
                let agent_id = self.agent_id();
                let conv_id = self.conversation_id();

                use cade_agent::tools::git_checkpoint;
                let git_cp = git_checkpoint::create_git_checkpoint("auto", &self.cwd).await;
                let stash = git_cp.as_ref().and_then(|g| g.stash_ref.as_deref());
                let commit = git_cp.as_ref().and_then(|g| g.commit_hash.as_deref());

                match self
                    .client
                    .create_checkpoint(
                        &agent_id,
                        Some("auto"),
                        Some("Created automatically prior to destructive tool execution"),
                        conv_id.as_deref(),
                        stash,
                        commit,
                    )
                    .await
                {
                    Ok(id) => {
                        let msg = if stash.is_some() {
                            format!(
                                "  ✓ Auto-checkpoint & stash saved (ID: {})",
                                &id[..8.min(id.len())]
                            )
                        } else {
                            format!("  ✓ Auto-checkpoint saved (ID: {})", &id[..8.min(id.len())])
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
            let s = self
                .last_assistant_text
                .lock()
                .clone();
            if s.is_empty() { None } else { Some(s) }
        };

        // Refresh the grace period before execution so stale terminal events
        // (Esc, Ctrl+C) accumulated during the preflight approval loop do not
        // trigger a false cancellation during slow tool execution.
        self.cancel_turn
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.last_modal_close_ms.store(
            now_epoch_ms(),
            std::sync::atomic::Ordering::SeqCst,
        );

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
            for &i in &read_indices {
                let (call_id, tool_name, args) = &tool_calls[i];
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

                handles.push(tokio::spawn(async move {
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
                }));
            }
            let join_results = futures::future::join_all(handles).await;
            for (i, r) in join_results.into_iter().flatten() {
                results[i] = r;
            }
            // Refresh grace period after parallel batch completes.
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.last_modal_close_ms.store(
                now_epoch_ms(),
                std::sync::atomic::Ordering::SeqCst,
            );
        }

        // Execute write tools sequentially.
        for &i in &write_indices {
            let (call_id, tool_name, args) = &tool_calls[i];
            let r = Self::run_tool_inner(
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
            )
            .await;
            results[i] = r;
            // Refresh grace period after each write tool so the next tool (or
            // Phase 3 streaming) is protected from stale terminal events.
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.last_modal_close_ms.store(
                now_epoch_ms(),
                std::sync::atomic::Ordering::SeqCst,
            );
        }

        // Update stats.
        for r in &results {
            { let mut stats = self.session_stats.lock();
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
        self.last_modal_close_ms.store(
            now_epoch_ms(),
            std::sync::atomic::Ordering::SeqCst,
        );

        // Phase 2: deposit all results to the server.  The first N-1 sends
        // return [] (server is still buffering); the Nth triggers the LLM and
        // streams back the assistant response with full context of all results.
        let mut follow = Vec::new();
        for result in &results {
            follow = self
                .stream_turn(
                    stdout,
                    "",
                    true,
                    &result.tool_call_id,
                    &result.output,
                    false,
                    None,
                    bar_text.clone(),
                )
                .await?;
        }

        Box::pin(self.dispatch_tool_calls(stdout, follow, user_input, bar_text, false, turn_stats))
            .await?;

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

        // Bash tools — live-streaming path (buffered per-tool)
        if matches!(tool_name, "bash" | "run_command" | "execute_command") {
            let live_idx = app.lock().begin_live_output(8);
            let app_arc = app.clone();
            let run_result = cade_agent::tools::bash::BashTool::run_streaming(args, move |line| {
                let _ = app_arc
                    .lock()
                    .append_live_output_line(live_idx, line);
            })
            .await;
            let _ = app
                .lock()
                .finish_live_output(live_idx);

            let (output, is_error) = match run_result {
                Ok(out) => (out, false),
                Err(e) => (format!("Error: {e}"), true),
            };

            let mut result = cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output,
                is_error,
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
        const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
        let mut result = match tokio::time::timeout(
            TOOL_TIMEOUT,
            runtime.execute(call_id.to_string(), tool_name, args),
        )
        .await
        {
            Ok(Some(rt)) => cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: rt.output,
                is_error: rt.is_error,
            },
            Ok(None) => {
                // ToolRuntime returned None — interactive-only tool not handled there;
                // fall through to native dispatch / MCP.
                match tokio::time::timeout(
                    TOOL_TIMEOUT,
                    dispatch(call_id.to_string(), tool_name, args, mcp),
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
                            TOOL_TIMEOUT.as_secs()
                        ),
                        is_error: true,
                    },
                }
            }
            Err(_) => cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: format!(
                    "Tool '{}' timed out after {}s",
                    tool_name,
                    TOOL_TIMEOUT.as_secs()
                ),
                is_error: true,
            },
        };

        if !result.is_error {
            match tool_name {
                "write_file" | "edit_file" | "apply_patch" | "Replace" | "WriteFileGemini" => {
                    let path = args["file_path"]
                        .as_str()
                        .or(args["path"].as_str())
                        .unwrap_or("unknown");
                    let msg = format!("Recently edited: {path}\n");
                    let c = runtime.client.clone();
                    let a = runtime.agent_id.clone();
                    tokio::spawn(async move {
                        let _ = c
                            .append_memory_with_limit(&a, "working_set", &msg, None, Some(3000))
                            .await;
                    });
                }
                _ => {}
            }
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
        let _ = app
            .lock()
            .push(RenderLine::ToolResult {
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
                }))
            }
            "run_subagent" => Some(self.handle_run_subagent(call_id, args).await),
            "ask_user_question" => Some(self.handle_ask_user_question(call_id, args).await),
            "message_agent" => Some(self.handle_message_agent(call_id, args).await),
            // Plan panel — require TuiApp access, intercepted before generic dispatch.
            "set_plan" => {
                let steps: Vec<String> = args["steps"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let n = steps.len();
                self.app.lock().set_plan(steps);
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: format!("Plan set with {n} step(s)."),
                    is_error: false,
                }))
            }
            "UpdatePlan" => {
                let step_id = args["step_id"].as_u64().unwrap_or(0) as usize;
                let done = args["done"].as_bool().unwrap_or(true);
                let found = self
                    .app
                    .lock()
                    .update_plan_step(step_id, done);
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: if found {
                        format!(
                            "Step {step_id} marked {}.",
                            if done { "done" } else { "not done" }
                        )
                    } else {
                        format!("Step {step_id} not found in active plan.")
                    },
                    is_error: !found,
                }))
            }
            _ => None,
        }
    }

    /// Check if a tool is a native intercept (requires &self). If so, execute
    /// it immediately and return the result. Returns None for generic tools.
    pub(crate) async fn sync_plan_tools(&self, enter_plan: bool) {
        let agent_id = self.agent_id.lock().clone();
        
        if enter_plan {
            // Strip write tools
            if let Ok(attached) = self.client.get_agent_tools(&agent_id).await {
                let mut new_ids = Vec::new();
                for (id, name) in attached {
                    let canonical_name = cade_agent::tools::manager::canonical_name(&name);
                    let is_mcp = cade_agent::tools::is_mcp_write_tool(canonical_name, &self.mcp).await;
                    let is_write = cade_core::permissions::is_write_schema(canonical_name) || is_mcp;
                    if !is_write && canonical_name != "exitplanmode" {
                        new_ids.push(id);
                    }
                }
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
                    // For now, let's just add everything back that isn't a known tool from a disabled capability.
                    // This might be slightly loose but works for re-attaching.
                    // To be safe, we only add back the write tools that exist on the server.
                    // Wait, what if the write tool belongs to a capability that is disabled?
                    // `write_file`, `edit_file`, `apply_patch`, `bash` are CORE tools, so they are always enabled.
                    // `desktop_control`, `desktop_screenshot` are DESKTOP capability.
                    // We can just add them back if their capability is enabled.
                    
                    let canonical_name = cade_agent::tools::manager::canonical_name(&t.name);
                    let is_mcp = cade_agent::tools::is_mcp_write_tool(canonical_name, &self.mcp).await;
                    let is_write_tool = cade_core::permissions::is_write_schema(canonical_name) || is_mcp;
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
                            "desktop_control" | "desktop_screenshot" => caps.is_enabled(cade_core::capabilities::Capability::Desktop),
                            _ => true, // core write tools
                        };
                        if allowed {
                            new_ids.push(t.id);
                        }
                    }
                }
                
                // Now we also need to get MCP tools and ensure they are attached. 
                // MCP tools are fetched via list_tools() too since they are registered on the server.
                let _ = self.client.attach_agent_tools(&agent_id, &new_ids).await;
            }
        }
    }

    /// C3: Inject a one-time ephemeral reminder prompting the agent to fill its
    /// `working_set` memory block after significant file-write activity.
    ///
    /// Only fires when the block is actually empty so the model is not nagged
    /// when it has already been diligently updating its own memory.
    pub(crate) async fn inject_working_set_reminder(
        &mut self,
        stdout: &mut io::Stdout,
    ) -> Result<()> {
        let agent_id = self.agent_id();

        // Fetch the current working_set value — one async call, performed once
        // per session at most.
        let is_empty = self
            .client
            .get_memory(&agent_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|b| b.label == "working_set")
            .map(|b| b.value.trim().is_empty())
            .unwrap_or(true);

        if !is_empty {
            // Already populated — no reminder needed.
            return Ok(());
        }

        let reminder = "[System: You have made several file changes this session. \
            Your `working_set` memory block is currently empty. \
            Please call update_memory now with label='working_set' and a value that records: \
            (1) the current task / goal, \
            (2) files you have modified, \
            (3) your immediate next steps. \
            Keep it under 200 words. This block persists when older context is dropped.]";

        tracing::debug!(
            "Injecting working_set reminder (write_tool_calls={})",
            self.write_tool_calls
                .load(std::sync::atomic::Ordering::SeqCst)
        );

        // Send as an ephemeral user message so it is not stored in the
        // conversation history but the agent still sees it and can respond
        // with an update_memory call.
        let msgs = self
            .stream_turn(stdout, reminder, false, "", "", true, None, None)
            .await?;

        // Dispatch any tool calls the model makes in response (usually update_memory).
        // reprompt_done=true prevents re-entry loops.
        let mut turn_stats = TurnStats::default();
        Box::pin(self.dispatch_tool_calls(stdout, msgs, "", None, true, &mut turn_stats)).await
    }

}
