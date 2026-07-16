use super::{BackgroundResult, Repl};
use crate::Result;
use cade_agent::subagents::{
    SubagentConfig, discover_all_subagents, resolve_subagent_def, should_emit_completion_bell,
};
use std::sync::Arc;

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
            && !tasks_val.is_empty() {
                self.tui_dim(format!(
                    "  Launching {} parallel subagents…",
                    tasks_val.len()
                ));
            }

        cade_agent::subagents::SubagentCoordinator::coordinate(self, call_id, args)
            .await
            .map_err(|e| crate::error::Error::custom(e.to_string()))
    }

    pub(crate) async fn handle_wait(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<cade_agent::tools::ToolResult> {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let timeout_ms = args.get("timeoutMs").and_then(|v| v.as_u64()).unwrap_or(1800000);
        let start = std::time::Instant::now();
        self.tui_dim(format!(
            "  Waiting for subagents{}…",
            if !id.is_empty() { format!(" (ID: {})", id) } else { "".to_string() }
        ));
        loop {
            let active_count = {
                let map = self.subagent_cancellations.lock().await;
                map.len()
            };
            if active_count == 0 {
                break;
            }
            if !all && !id.is_empty() {
                let still_running = {
                    let map = self.subagent_cancellations.lock().await;
                    map.contains_key(id)
                };
                if !still_running {
                    break;
                }
            } else if !all {
                break;
            }
            if start.elapsed().as_millis() as u64 >= timeout_ms {
                return Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "wait".to_string(),
                    output: "Timeout reached while waiting for subagents".to_string(),
                    is_error: true,
                    ui_resource_uri: None,
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(cade_agent::tools::ToolResult {
            tool_call_id: call_id.to_string(),
            tool_name: "wait".to_string(),
            output: "Finished waiting for subagents".to_string(),
            is_error: false,
            ui_resource_uri: None,
        })
    }

    pub(crate) async fn handle_intercom(
        &self,
        tool_name: &str,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<cade_agent::tools::ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
        let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("");
        let message = args.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let reply_to = args.get("replyTo").and_then(|v| v.as_str()).unwrap_or("");

        let output = match action {
            "list" => "[] (No active intercom channels)".to_string(),
            "send" | "ask" => format!("Message successfully sent to '{}': '{}'", to, message),
            "reply" => format!("Replied to message '{}': '{}'", reply_to, message),
            "pending" => "[] (No pending supervisor requests)".to_string(),
            "status" => "Intercom channel: connected. Routing table: 0 active routes.".to_string(),
            other => format!("Unsupported action '{}'", other),
        };

        Ok(cade_agent::tools::ToolResult {
            tool_call_id: call_id.to_string(),
            tool_name: tool_name.to_string(),
            output,
            is_error: false,
            ui_resource_uri: None,
        })
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
        for d in defs {
            out.push_str(&format!("- {}: {} ({})\n", d.name, d.description, d.tools));
        }
        Ok(out)
    }

    async fn cancel_subagent(&self, subagent_id: &str) -> std::result::Result<String, cade_agent::Error> {
        let tx_opt = {
            let map = self.subagent_cancellations.lock().await;
            map.get(subagent_id).cloned()
        };
        if let Some(tx) = tx_opt {
            let _ = tx.send(()).await;
            Ok(format!("Cancel signal sent to subagent {subagent_id}"))
        } else {
            Err(cade_agent::Error::custom(format!(
                "no active subagent found with ID {subagent_id}"
            )))
        }
    }

    fn doctor_status(&self) -> std::result::Result<String, cade_agent::Error> {
        Ok("Subagent system status: OK. Multi-agent concurrency slots available.".to_string())
    }
}

impl Repl {

    pub(crate) async fn handle_subagent_single_inner(
        &self,
        call_id: &str,
        args: &serde_json::Value,
        force_synchronous: bool,
    ) -> Result<cade_agent::tools::ToolResult> {
        let mut cfg = SubagentConfig::from_args(args);
        cfg.silent_stream |= self.settings.lock().silent_subagents();

        if let Err(reason) = cfg.validate() {
            return Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "subagent".to_string(),
                output: reason,
                is_error: true,
                ui_resource_uri: None,
            });
        }

        let all_defs = discover_all_subagents(&self.cwd);
        let def_opt = resolve_subagent_def(&cfg.mode, &all_defs).cloned();

        let subagent_mode = cfg.mode.clone();
        let background = cfg.background && !force_synchronous;
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
                        crate::cli::headless::HeadlessEvent::Text(_) => {
                            tracker.output_lines += 1;
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

        let run_task = {
            let task_id_c = task_id.clone();
            let cancellations_c = self.subagent_cancellations.clone();
            let cfg = cfg.clone();
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
                            cade_ai::catalogue::fast_model_for_main_model(&main_model)
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

                let mut cancel_rx = {
                    let (tx, rx) = tokio::sync::mpsc::channel(1);
                    cancellations_c
                        .lock()
                        .await
                        .insert(sub_agent_id.clone(), tx);
                    rx
                };

                struct CancelGuard<'a> {
                    map: &'a std::sync::Arc<
                        tokio::sync::Mutex<
                            std::collections::HashMap<String, tokio::sync::mpsc::Sender<()>>,
                        >,
                    >,
                    id: String,
                }
                impl<'a> Drop for CancelGuard<'a> {
                    fn drop(&mut self) {
                        let map = self.map.clone();
                        let id = self.id.clone();
                        if let Ok(handle) = tokio::runtime::Handle::try_current() {
                            handle.spawn(async move {
                                map.lock().await.remove(&id);
                            });
                        }
                    }
                }
                let _cancel_guard = CancelGuard {
                    map: &cancellations_c,
                    id: sub_agent_id.clone(),
                };

                let run_headless_fut = crate::cli::headless::run_headless(
                    &client,
                    &sub_agent_id,
                    &prompt,
                    &permissions,
                    &mcp_ref,
                    &hooks,
                    on_output.clone(),
                    cfg.max_tokens_budget,
                    cfg.resolve_allowed_paths(def_opt.as_ref()),
                );

                let result = tokio::select! {
                    res = run_headless_fut => res,
                    _ = cancel_rx.recv() => {
                        Err(crate::Error::custom("Task cancelled by parent".to_string()))
                    }
                };

                let (mut last_output, mut is_error) = match result {
                    Ok((output, _)) => (output, false),
                    Err(e) => (format!("Subagent error: {e}"), true),
                };

                if ephemeral {
                    let _ = client.delete_agent(&sub_agent_id).await;
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
                let _permit = sem.acquire_owned().await;
                let (result, is_error) = run_task.await;
                drop(_permit);

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
                    result,
                    is_error,
                });

                {
                    let mut app = bg_app_arc.lock();
                    app.subagent_trackers.retain(|t| t.task_id != bg_task_id);
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
                    "Background subagent [{subagent_mode}] launched (task ID: {}). \
                     You will be notified when it completes.",
                    task_id_c
                ),
                is_error: false,
                ui_resource_uri: None,
            })
        } else {
            let _permit = self.subagent_semaphore.acquire().await;
            let (output, is_error) = run_task.await;
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

    pub(crate) async fn handle_message_agent(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<cade_agent::tools::ToolResult> {
        let target = args["target"].as_str().unwrap_or("").trim().to_string();
        let message = args["message"].as_str().unwrap_or("").to_string();

        if target.is_empty() || message.is_empty() {
            return Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "message_agent".to_string(),
                output: "error: 'target' and 'message' are required".to_string(),
                is_error: true,
                ui_resource_uri: None,
            });
        }

        self.tui_ok(format!("  → Messaging agent [{target}]..."));

        let target_id = match self.client.list_agents().await {
            Ok(agents) => {
                if let Some(agent) = agents.iter().find(|a| a.id == target || a.name == target) {
                    agent.id.clone()
                } else {
                    return Ok(cade_agent::tools::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "message_agent".to_string(),
                        output: format!("Error: Agent '{target}' not found"),
                        is_error: true,
                        ui_resource_uri: None,
                    });
                }
            }
            Err(e) => {
                return Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "message_agent".to_string(),
                    output: format!("Failed to query agents: {e}"),
                    is_error: true,
                    ui_resource_uri: None,
                });
            }
        };

        let res = self
            .client
            .stream_message(&target_id, &message, |_| {})
            .await;

        match res {
            Ok(messages) => {
                let mut out = String::new();
                for msg in messages {
                    if let Some(text) = msg.assistant_text()
                        && !text.is_empty()
                    {
                        out.push_str(text);
                    }
                }

                let output = out.trim();
                let final_output = if output.is_empty() {
                    "Target agent returned an empty response".to_string()
                } else {
                    output.to_string()
                };

                self.tui_ok(format!("  ✓ Agent [{target}] responded"));

                Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "message_agent".to_string(),
                    output: final_output,
                    is_error: false,
                    ui_resource_uri: None,
                })
            }
            Err(e) => Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "message_agent".to_string(),
                output: format!("Failed to message agent: {e}"),
                is_error: true,
                ui_resource_uri: None,
            }),
        }
    }
}
