use super::{BackgroundResult, Repl};
use crate::Result;
use cade_agent::subagents::{
    SubagentConfig, discover_all_subagents, should_emit_completion_bell, visible_subagents,
};
use std::sync::Arc;

type CancellationMap = Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<String, cade_agent::subagents::SubagentCancellation>,
    >,
>;

async fn cancel_registered_subagent(
    map: &CancellationMap,
    id: &str,
) -> std::result::Result<String, cade_agent::Error> {
    let handle = map.lock().await.get(id).cloned().ok_or_else(|| {
        cade_agent::Error::custom(format!("no active subagent found with ID {id}"))
    })?;
    handle.cancel().map_err(|_| {
        cade_agent::Error::custom(format!("subagent {id} is no longer accepting cancellation"))
    })?;
    Ok(format!("Cancel signal sent to subagent {id}"))
}

impl Repl {
    /// Handle the `run_subagent` tool call — spawn a subagent and return its result.
    #[allow(clippy::type_complexity)]
    pub(crate) async fn handle_subagent(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<cade_agent::tools::ToolResult> {
        let mut cfg = SubagentConfig::from_args(args);
        cfg.silent_stream |= self.settings.lock().silent_subagents();

        if let Some(chain_val) = &cfg.chain {
            if !chain_val.is_empty() {
                self.tui_dim(format!(
                    "  Launching sequential subagent chain ({} steps)…",
                    chain_val.len()
                ));
            }
        } else if let Some(tasks_val) = &cfg.tasks
            && !tasks_val.is_empty()
        {
            self.tui_dim(format!(
                "  Launching {} parallel subagents…",
                tasks_val.len()
            ));
        }

        cade_agent::subagents::SubagentCoordinator::coordinate(self, call_id, args)
            .await
            .map_err(|e| crate::error::Error::custom(e.to_string()))
    }
}

#[async_trait::async_trait]
impl cade_agent::subagents::SubagentSingleRunner for Repl {
    async fn run_single(
        &self,
        call_id: &str,
        args: &serde_json::Value,
        force_sync: bool,
    ) -> std::result::Result<cade_agent::tools::ToolResult, cade_agent::Error> {
        self.handle_subagent_single_inner(call_id, args, force_sync)
            .await
            .map_err(|e| cade_agent::Error::custom(e.to_string()))
    }

    fn list_subagents(&self) -> std::result::Result<String, cade_agent::Error> {
        let defs = discover_all_subagents(&self.cwd);
        let mut out = String::from("Available subagents:\n");
        for d in visible_subagents(&defs) {
            out.push_str(&format!("- {}: {} ({})\n", d.name, d.description, d.tools));
        }
        Ok(out)
    }

    async fn cancel_subagent(
        &self,
        subagent_id: &str,
    ) -> std::result::Result<String, cade_agent::Error> {
        cancel_registered_subagent(&self.subagent_cancellations, subagent_id).await
    }

    fn doctor_status(&self) -> std::result::Result<String, cade_agent::Error> {
        let report = cade_core::doctor::check_multiplexer_and_keys();
        let mut out =
            "Subagent system status: OK. Multi-agent concurrency slots available.\n".to_string();
        out.push_str(&report.to_formatted_summary());
        Ok(out)
    }
}

impl Repl {
    pub(crate) async fn dispatch_subagent_tray_action(
        &self,
        action: cade_tui::app::subagent_tray::SubagentTrayAction,
    ) {
        use cade_agent::subagents::SubagentSingleRunner;
        use cade_tui::app::subagent_tray::SubagentTrayAction;
        match action {
            SubagentTrayAction::None => {}
            SubagentTrayAction::Kill { subagent_id } => {
                let cancel_res = self.cancel_subagent(&subagent_id).await;
                if cancel_res.is_ok() {
                    let mut app = self.app.lock();
                    app.show_toast(
                        format!("Cancellation requested for {subagent_id}"),
                        cade_tui::ToastLevel::Info,
                    );
                    app.draw_dirty = true;
                    let _ = app.draw();
                }
                if let Err(e) = cancel_res {
                    tracing::debug!("Local subagent cancel fallback for {subagent_id}: {e}");
                    let remote = self
                        .client
                        .raw_post(
                            &format!("/subagents/{subagent_id}/cancel"),
                            &serde_json::json!({ "action": "cancel", "id": subagent_id }),
                        )
                        .await;
                    let mut app = self.app.lock();
                    app.show_toast(
                        match remote {
                            Ok(_) => format!("Cancellation requested for {subagent_id}"),
                            Err(e) => format!("Could not cancel {subagent_id}: {e}"),
                        },
                        cade_tui::ToastLevel::Info,
                    );
                    app.draw_dirty = true;
                }
            }
            SubagentTrayAction::Steer {
                subagent_id,
                message,
            } => {
                {
                    let mut app = self.app.lock();
                    if let Some(t) = app
                        .subagent_trackers
                        .iter_mut()
                        .find(|t| t.task_id == subagent_id)
                    {
                        t.push_output(format!("[STEERING GUIDANCE]: {message}"));
                    }
                    app.show_toast(
                        format!("Steering guidance sent to {subagent_id}"),
                        cade_tui::ToastLevel::Success,
                    );
                    app.draw_dirty = true;
                    let _ = app.draw();
                }
                let body = serde_json::json!({
                    "action": "steer",
                    "id": subagent_id,
                    "message": message,
                });
                let _ = self
                    .client
                    .raw_post(&format!("/subagents/{subagent_id}/steer"), &body)
                    .await;
            }
            SubagentTrayAction::HotSwapModel { subagent_id, model } => {
                {
                    let mut app = self.app.lock();
                    if let Some(t) = app
                        .subagent_trackers
                        .iter_mut()
                        .find(|t| t.task_id == subagent_id)
                    {
                        t.push_output(format!("[MODEL HOT-SWAP]: {model}"));
                    }
                    app.show_toast(
                        format!("Model hot-swap to {model} for {subagent_id}"),
                        cade_tui::ToastLevel::Info,
                    );
                    app.draw_dirty = true;
                    let _ = app.draw();
                }
                let body = serde_json::json!({
                    "action": "hot_swap",
                    "id": subagent_id,
                    "model": model,
                });
                let _ = self
                    .client
                    .raw_post(&format!("/subagents/{subagent_id}/model"), &body)
                    .await;
            }
            SubagentTrayAction::PauseResume { subagent_id } => {
                {
                    let mut app = self.app.lock();
                    app.show_toast(
                        format!("Pause/Resume signal sent to {subagent_id}"),
                        cade_tui::ToastLevel::Info,
                    );
                    app.draw_dirty = true;
                    let _ = app.draw();
                }
                let body = serde_json::json!({
                    "action": "pause_resume",
                    "id": subagent_id,
                });
                let _ = self
                    .client
                    .raw_post(&format!("/subagents/{subagent_id}/pause"), &body)
                    .await;
            }
        }
    }

    pub(crate) async fn handle_subagent_single_inner(
        &self,
        call_id: &str,
        args: &serde_json::Value,
        force_synchronous: bool,
    ) -> Result<cade_agent::tools::ToolResult> {
        let mut cfg = SubagentConfig::from_args(args);
        cfg.silent_stream |= self.settings.lock().silent_subagents();

        let all_defs = discover_all_subagents(&self.cwd);
        let def_opt = match cfg.resolve_definition(&all_defs) {
            Ok(def) => def.cloned(),
            Err(reason) => {
                return Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "subagent".to_string(),
                    output: reason,
                    is_error: true,
                    ui_resource_uri: None,
                });
            }
        };

        if let Err(reason) = cfg.validate() {
            return Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "subagent".to_string(),
                output: reason,
                is_error: true,
                ui_resource_uri: None,
            });
        }

        // Headless runs execute on the server; a local clone cannot redirect
        // the server's tool calls. Refuse required isolation before start_run.
        if cfg.enforce_isolation || std::env::var("CADE_ISOLATION").is_ok_and(|v| v == "true") {
            return Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "subagent".to_string(),
                output: "error: required subagent isolation cannot be established for a CLI headless run; refusing to run in the live workspace".to_string(),
                is_error: true,
                ui_resource_uri: None,
            });
        }

        let subagent_mode = cfg.mode.clone();
        let background = cfg.background && !force_synchronous;
        let queued = background && self.subagent_semaphore.available_permits() == 0;
        let silent_stream = cfg.silent_stream;
        let human_review = cfg.human_review;
        let prompt = cfg.prompt_with_test_command();

        self.tui_dim(format!(
            "  Launching subagent [mode: {}]{}…",
            subagent_mode,
            if background { " (background)" } else { "" }
        ));

        let client = self.client.clone();
        let main_model = self.model();
        let permissions = cade_core::permissions::PermissionManager::new(if cfg.mode == "plan" {
            cade_core::permissions::PermissionMode::Plan
        } else {
            self.permissions.mode()
        });
        let call_id_owned = call_id.to_string();
        let bg_results = Arc::clone(&self.background_results);
        let mcp_ref = std::sync::Arc::clone(&self.mcp);
        let parent_agent_id = self.agent_id();
        let hooks = self.hooks.clone();

        let cwd_c = self.cwd.clone();
        let task_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let task_id_c = task_id.clone();
        let prompt_preview: String = prompt.chars().take(60).collect();

        let seed_blocks = {
            let parent_blocks = self
                .client
                .get_memory(&parent_agent_id)
                .await
                .unwrap_or_default();
            SubagentConfig::build_seed_memory(parent_blocks)
        };

        let app_arc = self.app.clone();
        let live_idx = if !background && !silent_stream {
            let mut app = app_arc.lock();
            app.push_silent(crate::ui::RenderLine::SystemMsg(format!(
                "  [Subagent: {}]",
                subagent_mode
            )));
            Some(app.begin_live_output(12))
        } else {
            None
        };

        let buffer = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));

        #[allow(clippy::type_complexity)]
        let on_output: Option<
            std::sync::Arc<dyn for<'a> Fn(crate::cli::headless::HeadlessEvent<'a>) + Send + Sync>,
        > = if let Some(idx) = live_idx {
            let app_arc = app_arc.clone();
            let buffer = buffer.clone();
            Some(std::sync::Arc::new(move |evt| match evt {
                crate::cli::headless::HeadlessEvent::Text(chunk) => {
                    let mut buf = buffer.lock();
                    buf.push_str(chunk);
                    while let Some(pos) = buf.find('\n') {
                        let line = buf[..pos].to_string();
                        buf.replace_range(..=pos, "");
                        let _ = app_arc.lock().append_live_output_line(idx, line);
                    }
                }
                crate::cli::headless::HeadlessEvent::ToolCall(tname) => {
                    let mut buf = buffer.lock();
                    let msg = format!("  [Calling {}...]\n", tname);
                    buf.push_str(&msg);
                    while let Some(pos) = buf.find('\n') {
                        let line = buf[..pos].to_string();
                        buf.replace_range(..=pos, "");
                        let _ = app_arc.lock().append_live_output_line(idx, line);
                    }
                }
            }))
        } else if background {
            let app_arc = app_arc.clone();
            let tid = task_id.clone();
            let smode = subagent_mode.clone();

            {
                let mut app = app_arc.lock();
                app.subagent_trackers
                    .push(cade_tui::subagent_tracker::SubagentTracker::new(
                        tid.clone(),
                        smode,
                    ));
                app.draw_dirty = true;
            }

            Some(std::sync::Arc::new(move |evt| {
                let mut app = app_arc.lock();
                if let Some(tracker) = app.subagent_trackers.iter_mut().find(|t| t.task_id == tid) {
                    match evt {
                        crate::cli::headless::HeadlessEvent::Text(text) => {
                            for line in text.split('\n') {
                                if !line.is_empty() {
                                    tracker.push_output(line.to_string());
                                }
                            }
                            tracker.current_tool = None;
                        }
                        crate::cli::headless::HeadlessEvent::ToolCall(tname) => {
                            tracker.tool_calls += 1;
                            tracker.current_tool = Some(tname.to_string());
                        }
                    }
                    app.draw_dirty = true;
                }
            }))
        } else {
            Some(std::sync::Arc::new(|_| {}))
        };

        let (cancel_tx, mut cancel_rx) = tokio::sync::mpsc::channel(1);
        let cancellation = cade_agent::subagents::SubagentCancellation::new(cancel_tx);
        self.subagent_cancellations
            .lock()
            .await
            .insert(task_id.clone(), cancellation.clone());
        struct CancelGuard {
            map: Arc<
                tokio::sync::Mutex<
                    std::collections::HashMap<String, cade_agent::subagents::SubagentCancellation>,
                >,
            >,
            id: String,
            cancellation: cade_agent::subagents::SubagentCancellation,
        }
        impl Drop for CancelGuard {
            fn drop(&mut self) {
                self.cancellation.close();
                let map = self.map.clone();
                let id = self.id.clone();
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        map.lock().await.remove(&id);
                    });
                }
            }
        }
        let cancel_guard = CancelGuard {
            map: self.subagent_cancellations.clone(),
            id: task_id.clone(),
            cancellation,
        };
        let created_agent = Arc::new(tokio::sync::Mutex::new(None::<String>));
        let server_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let run_task = {
            let task_id_c = task_id.clone();
            let cfg = cfg.clone();
            let created_agent = created_agent.clone();
            let server_cancel = server_cancel.clone();
            async move {
                let (sub_agent_id, ephemeral) = if let Some(existing_id) = cfg.agent_id.clone() {
                    (existing_id, false)
                } else {
                    let system_prompt_base = cfg.resolve_system_prompt(def_opt.as_ref());
                    let final_system_prompt = format!("{system_prompt_base}\n\nTask: {prompt}");
                    let final_description = cfg.ephemeral_description();
                    let model = cfg
                        .resolve_model(def_opt.as_ref())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            cade_ai::catalogue::select_fast_subagent_model(&main_model, None)
                        });

                    let req = cade_agent::agent::client::CreateAgentRequest {
                        name: Some(cfg.ephemeral_agent_name(&task_id_c)),
                        model,
                        description: Some(final_description),
                        system_prompt: Some(final_system_prompt),
                        memory_blocks: seed_blocks,
                        tool_ids: vec![],
                    };
                    match client.create_agent(req).await {
                        Ok(a) => (a.id, true),
                        Err(e) => return (format!("Failed to create subagent: {e}"), true),
                    }
                };

                if ephemeral {
                    *created_agent.lock().await = Some(sub_agent_id.clone());
                }

                let run_headless_fut = crate::cli::headless::run_headless_with_cancel(
                    &client,
                    &sub_agent_id,
                    &prompt,
                    &permissions,
                    &mcp_ref,
                    &hooks,
                    on_output.clone(),
                    cfg.max_tokens_budget,
                    cfg.resolve_allowed_paths(def_opt.as_ref()),
                    Some(&server_cancel),
                );

                let result = run_headless_fut.await.and_then(|out| {
                    if server_cancel.load(std::sync::atomic::Ordering::SeqCst) {
                        Err(crate::Error::custom("Task cancelled by parent"))
                    } else {
                        Ok(out)
                    }
                });

                let (mut last_output, mut is_error) = match result {
                    Ok((output, _)) => (output, false),
                    Err(e) => (format!("Subagent error: {e}"), true),
                };

                if ephemeral {
                    let _ = client.delete_agent(&sub_agent_id).await;
                    created_agent.lock().await.take();
                }

                if !is_error && let Some(cmd) = cfg.test_command.as_deref() {
                    match std::process::Command::new("bash")
                        .arg("-c")
                        .arg(cmd)
                        .current_dir(&cwd_c)
                        .output()
                    {
                        Ok(output) => {
                            if !output.status.success() {
                                is_error = true;
                                let stdout = String::from_utf8_lossy(&output.stdout);
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                last_output = format!(
                                    "PROOF OF WORK FAILED: Subagent claimed success, but the test command `{cmd}` failed on the host.\n\nSubagent output:\n{last_output}\n\nTest stdout:\n{stdout}\nTest stderr:\n{stderr}\n\nYou must re-run the subagent or fix the remaining issues yourself."
                                );
                            } else {
                                last_output.push_str(&format!(
                                    "\n\n[PROOF OF WORK VERIFIED: `{cmd}` exited with code 0]"
                                ));
                            }
                        }
                        Err(e) => {
                            is_error = true;
                            last_output = format!(
                                "PROOF OF WORK FAILED: Failed to execute test command `{cmd}`: {e}\n\nSubagent output:\n{last_output}"
                            );
                        }
                    }
                }

                (last_output, is_error)
            }
        };

        if background {
            let sem = std::sync::Arc::clone(&self.subagent_semaphore);
            let bg = bg_results;
            let st = subagent_mode.clone();
            let bg_client = self.client.clone();
            let bg_parent_id = parent_agent_id.clone();
            let bg_st_label = subagent_mode.clone();
            let bg_task_id = task_id.clone();
            let bg_silent = silent_stream;
            let bg_app_arc = app_arc.clone();
            tokio::spawn(async move {
                let _cancel_guard = cancel_guard;
                let outcome = tokio::select! {
                    biased;
                    Some(()) = cancel_rx.recv() => None,
                    permit = sem.acquire_owned() => {
                        match permit {
                            Ok(_permit) => {
                                let mut run_task = std::pin::pin!(run_task);
                                tokio::select! {
                                    biased;
                                    Some(()) = cancel_rx.recv() => {
                                        server_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                                        Some(run_task.await)
                                    },
                                    result = &mut run_task => Some(result),
                                }
                            }
                            Err(_) => Some(("Subagent semaphore closed".to_string(), true)),
                        }
                    }
                };
                let (result, is_error) =
                    outcome.unwrap_or_else(|| ("Subagent cancelled by parent".to_string(), true));
                _cancel_guard.cancellation.close();
                if let Some(id) = created_agent.lock().await.take() {
                    let _ = bg_client.delete_agent(&id).await;
                }

                {
                    let label = format!("subagent:{}:{}", bg_st_label, bg_task_id);
                    let summary_value = if result.chars().count() > 1500 {
                        let _ = bg_client
                            .insert_archival_memory(
                                &bg_parent_id,
                                &result,
                                &["subagent".to_string(), bg_task_id.clone()],
                            )
                            .await;

                        let end = result
                            .char_indices()
                            .nth(500)
                            .map(|(i, _)| i)
                            .unwrap_or(result.len());
                        format!(
                            "Subagent completed. Full output is stored in Archival Memory. To view it, use archival_memory_search with query 'subagent {}'. Summary preview: {}…",
                            bg_task_id,
                            &result[..end]
                        )
                    } else {
                        result.clone()
                    };
                    let desc = format!("Result from background subagent [{}]", bg_st_label);
                    let _ = bg_client
                        .upsert_memory(&bg_parent_id, &label, &summary_value, Some(&desc))
                        .await;
                }

                bg.lock().push(BackgroundResult {
                    task_id: task_id.clone(),
                    subagent: st,
                    prompt_preview,
                    result: result.clone(),
                    is_error,
                });

                {
                    let mut app = bg_app_arc.lock();
                    if let Some(tracker) = app
                        .subagent_trackers
                        .iter_mut()
                        .find(|t| t.task_id == bg_task_id)
                    {
                        tracker.status = if is_error {
                            cade_tui::subagent_tracker::SubagentStatus::Failed {
                                finished_at: std::time::Instant::now(),
                                error: result.chars().take(120).collect(),
                            }
                        } else {
                            cade_tui::subagent_tracker::SubagentStatus::Completed {
                                finished_at: std::time::Instant::now(),
                            }
                        };
                    } else {
                        app.subagent_trackers.retain(|t| t.task_id != bg_task_id);
                    }
                    app.draw_dirty = true;
                }

                if should_emit_completion_bell(
                    bg_silent,
                    std::io::IsTerminal::is_terminal(&std::io::stdout()),
                ) {
                    use std::io::Write;
                    let mut out = std::io::stdout().lock();
                    let _ = out.write_all(b"\x07");
                    let _ = out.flush();
                }
            });

            Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id_owned,
                tool_name: "subagent".to_string(),
                output: format!(
                    "Background subagent [{subagent_mode}] {} (task ID: {}). \
                     You will be notified when it completes.",
                    if queued { "queued" } else { "launched" },
                    task_id_c
                ),
                is_error: false,
                ui_resource_uri: None,
            })
        } else {
            let _cancel_guard = cancel_guard;
            let _permit = self.subagent_semaphore.acquire().await;
            let mut run_task = std::pin::pin!(run_task);
            let (output, is_error) = tokio::select! {
                biased;
                Some(()) = cancel_rx.recv() => {
                    server_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                    run_task.await
                },
                result = &mut run_task => result,
            };
            _cancel_guard.cancellation.close();
            if let Some(id) = created_agent.lock().await.take() {
                let _ = self.client.delete_agent(&id).await;
            }
            drop(_permit);

            if let Some(idx) = live_idx {
                let mut buf = buffer.lock();
                if !buf.is_empty() {
                    let _ = app_arc.lock().append_live_output_line(idx, buf.clone());
                    buf.clear();
                }
                let _ = app_arc.lock().finish_live_output(idx);
            }

            let hook_outcome = self
                .hooks
                .subagent_stop(&subagent_mode, &output, is_error)
                .await;

            if !is_error {
                self.tui_ok(format!("  ✓ Subagent [{}] complete", subagent_mode));
            }

            {
                let own_label = format!("subagent:{}:{}", subagent_mode, task_id_c);
                let _ = self
                    .client
                    .delete_memory(&parent_agent_id, &own_label)
                    .await;
            }

            if output.chars().count() > 1500 {
                let _ = self
                    .client
                    .insert_archival_memory(
                        &parent_agent_id,
                        &output,
                        &["subagent".to_string(), task_id_c.clone()],
                    )
                    .await;
            }

            let mut final_output = match hook_outcome {
                cade_core::hooks::HookOutcome::Block { reason } => {
                    format!("{output}\n\n[SubagentStop hook: {reason}]")
                }
                cade_core::hooks::HookOutcome::Allow => output,
            };

            let mut final_is_error = is_error;

            if !final_is_error && human_review {
                use crate::ui::question::{Question, QuestionOption};
                let q = Question {
                    header: format!("Subagent [{subagent_mode}] Completed"),
                    text: "Review the subagent's work. Select Approve, or type feedback to Reject and re-task:".to_string(),
                    options: vec![
                        QuestionOption { label: "Approve".to_string(), description: String::new() },
                    ],
                    multi_select: false,
                    allow_other: true,
                    progress: None,
                };

                let ans_opt = self.app.lock().ask_question(&q).unwrap_or(None);

                if let Some(ans) = ans_opt {
                    let val = ans.as_str();
                    if val != "Approve" {
                        final_is_error = true;
                        final_output = format!(
                            "HUMAN REVIEW REJECTED: The user reviewed the subagent's work and rejected it with the following feedback:\n\n\"{val}\"\n\nYou MUST re-invoke the subagent with these additional instructions to fix the issue.\n\nPrevious subagent output:\n{final_output}"
                        );
                    }
                }
            }

            Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id_owned,
                tool_name: "subagent".to_string(),
                output: final_output,
                is_error: final_is_error,
                ui_resource_uri: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::cancel_registered_subagent;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{Mutex, mpsc};

    #[tokio::test]
    async fn cli_cancellation_is_truthful_for_active_repeated_missing_and_closed_handles() {
        let cancellations: Arc<
            Mutex<HashMap<String, cade_agent::subagents::SubagentCancellation>>,
        > = Arc::new(Mutex::new(HashMap::new()));
        let (tx, mut rx) = mpsc::channel(1);

        let sub_agent_id = "subagent-agent-123".to_string();
        let task_id = "task-abc456".to_string();

        {
            let mut map = cancellations.lock().await;
            let handle = cade_agent::subagents::SubagentCancellation::new(tx);
            map.insert(sub_agent_id.clone(), handle.clone());
            map.insert(task_id.clone(), handle);
        }

        // Cancel via task_id
        assert!(
            cancel_registered_subagent(&cancellations, &task_id)
                .await
                .is_ok()
        );
        assert!(
            cancel_registered_subagent(&cancellations, &sub_agent_id)
                .await
                .is_err()
        );
        assert!(
            cancel_registered_subagent(&cancellations, "missing")
                .await
                .is_err()
        );

        // Verify receiver catches cancellation
        assert_eq!(rx.recv().await, Some(()));
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        cancellations.lock().await.insert(
            "closed".into(),
            cade_agent::subagents::SubagentCancellation::new(tx),
        );
        assert!(
            cancel_registered_subagent(&cancellations, "closed")
                .await
                .is_err()
        );

        // Clean up
        {
            let mut map = cancellations.lock().await;
            map.remove(&sub_agent_id);
            map.remove(&task_id);
            map.remove("closed");
        }
        assert!(cancellations.lock().await.is_empty());
    }
}
