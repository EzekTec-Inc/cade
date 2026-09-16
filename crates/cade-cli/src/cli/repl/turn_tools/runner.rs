use super::super::Repl;
use crate::ui::RenderLine;

impl Repl {
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

        // Capture baseline content if this is a file edit tool for live side-tray diff tracking
        let is_file_edit = cade_agent::tools::manager::is_file_edit_tool(tool_name);
        let file_path_target = if is_file_edit {
            args.get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|p| p.as_str())
                .map(std::path::PathBuf::from)
        } else {
            None
        };
        let pre_content = if let Some(ref p) = file_path_target {
            std::fs::read_to_string(p).unwrap_or_default()
        } else {
            String::new()
        };

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

        if !result.is_error && is_file_edit {
            if let Some(ref p) = file_path_target {
                let post_content = std::fs::read_to_string(p).unwrap_or_default();
                let mut a = app.lock();
                a.modified_files_tracker
                    .record_mutation(p, &pre_content, &post_content);
                a.draw_dirty = true;
            }

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
