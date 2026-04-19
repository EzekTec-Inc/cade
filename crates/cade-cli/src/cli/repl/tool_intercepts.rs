use super::{BackgroundResult, Repl};
use crate::Result;
use cade_agent::subagents::{discover_all_subagents, find_subagent};
use std::sync::Arc;

impl Repl {
    /// Handle the `run_subagent` tool call — spawn a subagent and return its result.
    #[allow(clippy::type_complexity)]
    pub(crate) async fn handle_run_subagent(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<cade_agent::tools::ToolResult> {
        let subagent_mode = args["mode"]
            .as_str()
            .unwrap_or("build")
            .trim()
            .to_string();
        let mut prompt = args["prompt"].as_str().unwrap_or("").trim().to_string();
        let background = args["background"].as_bool().unwrap_or(false);
        let silent_stream = args["silent_stream"].as_bool().unwrap_or(false)
            || self.settings.lock().silent_subagents();
        let test_command = args["test_command"].as_str().map(|s| s.trim().to_string());
        let agent_id_arg = args["agent_id"].as_str().map(|s| s.trim().to_string());
        let model_override = args["model"].as_str().map(|s| s.trim().to_string());
        let custom_system_prompt = args["system_prompt"].as_str().map(|s| s.trim().to_string());
        let custom_description = args["description"].as_str().map(|s| s.trim().to_string());

        if prompt.is_empty() {
            return Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "run_subagent".to_string(),
                output: "error: 'prompt' is required".to_string(),
                is_error: true,
            });
        }

        if let Some(cmd) = &test_command {
            prompt.push_str(&format!(
                "\n\nCRITICAL PROOF OF WORK REQUIRED: You MUST run the following test command to verify your fix: `{cmd}`. Do not return until this command passes. The main agent will execute this command on the host system to verify your work. If it fails, your answer will be rejected."
            ));
        }

        // Resolve subagent definition - we now always use the unified worker
        let all_defs = discover_all_subagents(&self.cwd);
        let def_opt = find_subagent("worker", &all_defs).cloned();

        // Determine if using existing stateful agent or ephemeral
        let _use_existing_agent = agent_id_arg.is_some();

        // Show progress
        self.tui_dim(format!(
            "  Launching unified subagent [mode: {}]{}…",
            subagent_mode,
            if background { " (background)" } else { "" }
        ));

        // Clone what we need for the async task
        let client = self.client.clone();
        let main_model = self.model();
        let permissions = cade_core::permissions::PermissionManager::default();
        let call_id_owned = call_id.to_string();
        let bg_results = Arc::clone(&self.background_results);
        let mcp_ref = std::sync::Arc::clone(&self.mcp);
        let parent_agent_id = self.agent_id();
        let hooks = self.hooks.clone();

        let test_command_c = test_command.clone();
        let cwd_c = self.cwd.clone();
        let task_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let task_id_c = task_id.clone();
        let prompt_preview: String = prompt.chars().take(60).collect();

        // Seed memory: fetch parent agent's pinned + short-term memory blocks
        // so the sub-agent starts with relevant context from the parent.
        let seed_blocks: Vec<cade_agent::agent::client::MemoryBlock> = {
            let parent_blocks = self
                .client
                .get_memory(&parent_agent_id)
                .await
                .unwrap_or_default();
            parent_blocks
                .into_iter()
                .filter(|b| {
                    // Include pinned and short-tier blocks; skip internal bookkeeping.
                    let dominated = b.label.starts_with("__");
                    let tier_ok = b
                        .tier
                        .as_deref()
                        .is_none_or(|t| t == "pinned" || t == "short");
                    !dominated && tier_ok && !b.value.trim().is_empty()
                })
                .map(|b| cade_agent::agent::client::MemoryBlock {
                    label: b.label,
                    value: {
                        // Cap each block to keep the seed compact.
                        let max = 1500;
                        if b.value.chars().count() > max {
                            let end = b
                                .value
                                .char_indices()
                                .nth(max)
                                .map(|(i, _)| i)
                                .unwrap_or(b.value.len());
                            format!("{}…", &b.value[..end])
                        } else {
                            b.value
                        }
                    },
                    description: b.description,
                    tier: None, // server defaults to short
                })
                .collect()
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

        let on_output: Option<std::sync::Arc<dyn Fn(&str) + Send + Sync>> =
            if let Some(idx) = live_idx {
                let app_arc = app_arc.clone();
                let buffer = buffer.clone();
                Some(std::sync::Arc::new(move |chunk: &str| {
                    let mut buf = buffer.lock();
                    buf.push_str(chunk);
                    while let Some(pos) = buf.find('\n') {
                        let line = buf[..pos].to_string();
                        buf.replace_range(..=pos, "");
                        let _ = app_arc
                            .lock()
                            .append_live_output_line(idx, line);
                    }
                }))
            } else {
                // For background subagents, we just buffer silently or ignore
                Some(std::sync::Arc::new(|_| {}))
            };

        let run_task = {
            let subagent_mode_c = subagent_mode.clone();
            let task_id_c = task_id.clone();
            let _prompt_preview_c = prompt_preview.clone();
            async move {
                // Determine agent to use
                let (sub_agent_id, ephemeral) = if let Some(existing_id) = agent_id_arg {
                    (existing_id, false)
                } else {
                    // Create ephemeral agent
                    let final_system_prompt = custom_system_prompt
                        .or_else(|| def_opt.as_ref().map(|d| d.system_prompt.clone()))
                        .unwrap_or_else(|| {
                            "You are a helpful coding assistant. Complete the task and report back."
                                .to_string()
                        });

                    let final_description = custom_description
                        .unwrap_or_else(|| format!("Ephemeral subagent: {subagent_mode_c}"));

                    let model = model_override
                        .clone()
                        .or_else(|| def_opt.as_ref().and_then(|d| d.model.clone()))
                        .unwrap_or_else(|| cade_ai::catalogue::fast_model_for_main_model(&main_model));

                    let req = cade_agent::agent::client::CreateAgentRequest {
                        name: Some(format!("subagent-{}-{}", subagent_mode_c, task_id_c)),
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

                // Run headless
                let result = crate::cli::headless::run_headless(
                    &client,
                    &sub_agent_id,
                    &prompt,
                    &permissions,
                    &mcp_ref,
                    &hooks,
                    on_output.clone(),
                )
                .await;

                let (mut last_output, mut is_error) = match result {
                    Ok((output, _)) => (output, false),
                    Err(e) => (format!("Subagent error: {e}"), true),
                };

                // Delete ephemeral agent
                if ephemeral {
                    let _ = client.delete_agent(&sub_agent_id).await;
                }
                
                // Verify Proof of Work
                if !is_error
                    && let Some(cmd) = test_command_c {
                        match std::process::Command::new("bash")
                            .arg("-c")
                            .arg(&cmd)
                            .current_dir(&cwd_c)
                            .output()
                        {
                            Ok(output) => {
                                if !output.status.success() {
                                    is_error = true;
                                    let stdout = String::from_utf8_lossy(&output.stdout);
                                    let stderr = String::from_utf8_lossy(&output.stderr);
                                    last_output = format!("PROOF OF WORK FAILED: Subagent claimed success, but the test command `{cmd}` failed on the host.\n\nSubagent output:\n{last_output}\n\nTest stdout:\n{stdout}\nTest stderr:\n{stderr}\n\nYou must re-run the subagent or fix the remaining issues yourself.");
                                } else {
                                    last_output.push_str(&format!("\n\n[PROOF OF WORK VERIFIED: `{cmd}` exited with code 0]"));
                                }
                            }
                            Err(e) => {
                                is_error = true;
                                last_output = format!("PROOF OF WORK FAILED: Failed to execute test command `{cmd}`: {e}\n\nSubagent output:\n{last_output}");
                            }
                        }
                    }

                (last_output, is_error)
            }
        };

        if background {
            // Acquire a permit — blocks if cap is reached, queues the task
            let sem = std::sync::Arc::clone(&self.subagent_semaphore);
            let bg = bg_results;
            let st = subagent_mode.clone();
            let bg_client = self.client.clone();
            let bg_parent_id = parent_agent_id.clone();
            let bg_st_label = subagent_mode.clone();
            let bg_task_id = task_id.clone();
            tokio::spawn(async move {
                // Permit held for the lifetime of the spawned task
                let _permit = sem.acquire_owned().await;
                let (result, is_error) = run_task.await;
                drop(_permit);

                // Write sub-agent result summary into parent agent's short-term memory.
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
            });

            Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id_owned,
                tool_name: "run_subagent".to_string(),
                output: format!(
                    "Background subagent [{subagent_mode}] launched (task ID: {}). \
                     You will be notified when it completes.",
                    task_id_c
                ),
                is_error: false,
            })
        } else {
            // Run synchronously — acquire permit, wait for result, release
            let _permit = self.subagent_semaphore.acquire().await;
            let (output, is_error) = run_task.await;
            drop(_permit);

            // Finish live output and push any remaining buffer
            if let Some(idx) = live_idx {
                let mut buf = buffer.lock();
                if !buf.is_empty() {
                    let _ = app_arc
                        .lock()
                        .append_live_output_line(idx, buf.clone());
                    buf.clear();
                }
                let _ = app_arc
                    .lock()
                    .finish_live_output(idx);
            }

            // SubagentStop hook — can block (exit 2 continues the agent)
            let hook_outcome = self
                .hooks
                .subagent_stop(&subagent_mode, &output, is_error)
                .await;

            if !is_error {
                self.tui_ok(format!("  ✓ Subagent [{}] complete", subagent_mode));
            }

            // Clean up any stale subagent memory blocks from the parent agent
            if let Ok(blocks) = self.client.get_memory(&parent_agent_id).await {
                for block in blocks {
                    if block.label.starts_with("subagent:") {
                        let _ = self.client.delete_memory(&parent_agent_id, &block.label).await;
                    }
                }
            }

            // Store full output in Archival Memory if it's large, but DO NOT pollute active memory.
            if output.chars().count() > 1500 {
                let _ = self.client.insert_archival_memory(
                    &parent_agent_id,
                    &output,
                    &["subagent".to_string(), task_id_c.clone()],
                ).await;
            }

            // If hook blocked, append its reason to the output so the agent sees it
            let final_output = match hook_outcome {
                cade_core::hooks::HookOutcome::Block { reason } => {
                    format!("{output}\n\n[SubagentStop hook: {reason}]")
                }
                cade_core::hooks::HookOutcome::Allow => output,
            };

            Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id_owned,
                tool_name: "run_subagent".to_string(),
                output: final_output,
                is_error,
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
                    });
                }
            }
            Err(e) => {
                return Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "message_agent".to_string(),
                    output: format!("Failed to query agents: {e}"),
                    is_error: true,
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
                })
            }
            Err(e) => Ok(cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "message_agent".to_string(),
                output: format!("Failed to message agent: {e}"),
                is_error: true,
            }),
        }
    }
}
