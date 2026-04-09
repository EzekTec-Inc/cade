//! Slash command dispatch for the REPL.
//!
//! Contains the `handle_slash_command` method extracted from the main
//! `run()` event loop.  Returns `Ok(true)` when the REPL should exit,
//! `Ok(false)` to continue the loop.

use std::io;
use std::sync::Arc;

use crate::Result;
use crate::ui::{RenderLine, ToastLevel};
use super::{
    mode_display,
    SubagentPickerResult, Repl,
};
use super::slash::SlashCmd;
use cade_agent::subagents::discover_all_subagents;
use cade_core::permissions::PermissionMode;
use cade_core::toolsets::Toolset;

impl Repl {
    /// Dispatch a parsed slash command.
    ///
    /// Returns `Ok(true)` if the REPL should exit (i.e. `/exit`),
    /// `Ok(false)` to continue the loop.
    pub(crate) async fn handle_slash_command(
        &mut self,
        cmd: SlashCmd,
        input: &str,
        mut stdout: &mut io::Stdout,
        pending_input: &mut Option<String>,
    ) -> Result<bool> {
        match cmd {
            SlashCmd::Exit => {
                use std::sync::atomic::Ordering;
                let in_tok = self.session_input_tokens.load(Ordering::SeqCst);
                let out_tok = self.session_output_tokens.load(Ordering::SeqCst);
                if in_tok > 0 || out_tok > 0 {
                    let _ = self.app.lock().push(
                        RenderLine::SystemMsg(format!(
                            "  Session tokens — in: {in_tok}  out: {out_tok}  total: {}",
                            in_tok + out_tok
                        )),
                    );
                }
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::SystemMsg("Bye!".to_string()));
                return Ok(true);
            }
            // SlashCmd::Clear is handled below (with context clearing)
            SlashCmd::RunSkill(skill_id) => {
                // Find the skill, build a prompt that injects its content,
                // and send it as an agent turn so the agent follows the skill.
                let skill_body = self
                    .skills
                    .lock()
                    .iter()
                    .find(|s| s.id == skill_id)
                    .map(|s| s.to_context_block());
                if let Some(body) = skill_body {
                    let prompt = format!(
                        "[Skill invoked: /{skill_id}]\n\nFollow this skill:\n\n{body}"
                    );
                    self.tui_sys(format!("  Running skill: /{skill_id}"));
                    self.agent_turn(&mut stdout, &prompt).await?;
                } else {
                    self.tui_err(format!(
                        "  Skill '{skill_id}' not found. Try /skills reload"
                    ));
                }
                return Ok(false);
            }
            SlashCmd::Help => {
                // Open full-screen command browser (filtered by capabilities)
                let chosen = {
                    let mut app = self.app.lock();
                    let colors = app.colors.clone();
                    crate::ui::menu::show_command_menu_with_caps(
                        &mut app.terminal,
                        &colors,
                        Some(&self.capabilities),
                    )?
                };
                let _ = self.app.lock().draw();
                if let Some(cmd) = chosen {
                    // If it's a tool hint (no slash) or a command that needs arguments,
                    // insert it into the editor instead of executing immediately.
                    let needs_args = !cmd.starts_with('/')
                        || (cmd.contains(' ')
                            && !["/stats model", "/skills reload"].contains(&cmd.as_str()))
                        || [
                            "/delete",
                            "/checkpoint",
                            "/fork",
                            "/approve-always",
                            "/deny-always",
                            "/remember",
                            "/disconnect",
                            "/search",
                            "/export",
                            "/rename",
                            "/connect",
                        ]
                        .contains(&cmd.as_str());

                    if needs_args {
                        let mut app = self.app.lock();
                        app.editor.insert_str(&format!("{cmd} "));
                    } else {
                        *pending_input = Some(cmd);
                    }
                }
                return Ok(false);
            }
            SlashCmd::Agent => {
                let msg = format!("  Agent: {} ({})", self.agent_name(), self.agent_id());
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::SystemMsg(msg));
            }
            SlashCmd::Info => {
                let msg = format!(
                    "  Agent   : {} ({})\n  Conv    : {}\n  Model   : {}\n  Mode    : {}\n  CWD     : {}\n  Version : {}",
                    self.agent_name(),
                    self.agent_id(),
                    self.conversation_id().as_deref().unwrap_or("default"),
                    self.model(),
                    self.permissions.mode(),
                    self.cwd.display(),
                    env!("CARGO_PKG_VERSION")
                );
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::SystemMsg(msg));
            }
            SlashCmd::Yolo => {
                self.permissions.set_mode(PermissionMode::BypassPermissions);
                self.app
                    .lock()
                    .update_mode(PermissionMode::BypassPermissions);
                let _ =
                    self.app
                        .lock()
                        .push(RenderLine::SystemMsg(
                        "⚡ Permission mode: bypassPermissions — all tools auto-approved"
                            .to_string(),
                    ));
                self.sync_plan_tools(false).await;
            }
            SlashCmd::Mcp => {
                return self.cmd_mcp(input, pending_input).await;
            }
            SlashCmd::McpSave(payload) => {
                let parsed: std::result::Result<
                    std::collections::HashMap<String, cade_core::settings::McpServerConfig>,
                    _,
                > = serde_json::from_str(&payload);
                match parsed {
                    Ok(servers) => {
                        let mut s = self.settings.lock();
                        for (k, v) in servers {
                            s.global_settings_mut().mcp_servers.insert(k.clone(), v);
                            self.tui_ok(format!("  ✓ Saved MCP server: {}", k));
                        }
                        let _ = s.save_global();
                        drop(s);
                        self.do_settings_reload().await;
                    }
                    Err(e) => {
                        self.tui_err(format!("  ✗ Failed to parse JSON: {}", e));
                    }
                }
            }
            SlashCmd::Link => {
                self.tui_dim("  Linking tools…");
                let client2 = self.client.clone();
                let mcp2 = std::sync::Arc::clone(&self.mcp);
                let toolset2 = *self.current_toolset.lock();
                let agent_id = self.agent_id();
                use cade_agent::agent::tools::{register_cade_tools, register_mcp_tools};
                let allow_agent_mode = self
                    .settings
                    .lock()
                    .permission_settings()
                    .allow_agent_mode_changes;
                let native_ids: Vec<String> =
                    register_cade_tools(&client2, toolset2, allow_agent_mode)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .map(|t| t.id)
                        .collect();
                let n_native = native_ids.len();
                if !native_ids.is_empty() {
                    let _ = client2.attach_agent_tools(&agent_id, &native_ids).await;
                }
                let mcp_ids: Vec<String> =
                    register_mcp_tools(&client2, mcp2.all_tool_schemas().await)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .map(|t| t.id)
                        .collect();
                let n_mcp = mcp_ids.len();
                if !mcp_ids.is_empty() {
                    let _ = client2.attach_agent_tools(&agent_id, &mcp_ids).await;
                }
                self.tui_ok(format!(
                    "  ✓ Linked {n_native} native + {n_mcp} MCP tool(s)"
                ));
            }
            SlashCmd::Unlink => {
                let agent_id = self.agent_id();
                match self.client.detach_agent_tools(&agent_id).await {
                    Ok(n) => self.tui_ok(format!("  ✓ Detached {n} tool(s) from agent")),
                    Err(e) => self.tui_err(e.to_string()),
                }
            }
            SlashCmd::Stream => {
                use std::sync::atomic::Ordering;
                let current = self.streaming_enabled.load(Ordering::SeqCst);
                self.streaming_enabled.store(!current, Ordering::SeqCst);
                let label = if !current { "on" } else { "off" };
                self.tui_hdr(format!("  Streaming: {label}"));
                self.app
                    .lock()
                    .show_toast(format!("Streaming {label}"), ToastLevel::Info);
            }
            SlashCmd::Usage => {
                use std::sync::atomic::Ordering;
                let in_tok = self.session_input_tokens.load(Ordering::SeqCst);
                let out_tok = self.session_output_tokens.load(Ordering::SeqCst);
                let total = in_tok + out_tok;
                self.tui_blank();
                self.tui_hdr("  Token usage this session:");
                self.tui_dim(format!("    Input  : {:>8}", in_tok));
                self.tui_dim(format!("    Output : {:>8}", out_tok));
                self.tui_dim(format!("    Total  : {:>8}", total));
                if total == 0 {
                    self.tui_dim("    (no usage recorded yet — requires Anthropic/OpenAI)");
                }
            }
            SlashCmd::Context => {
                return self.cmd_context(stdout).await;
            }
            SlashCmd::DebugLast => {
                let conv = self.conversation_id();
                match self
                    .client
                    .last_assistant_message(&self.agent_id(), conv.as_deref())
                    .await
                {
                    Ok(Some(msg)) => {
                        self.tui_hdr("  Raw last assistant message");
                        if let Ok(raw) = serde_json::to_string_pretty(&msg) {
                            for line in raw.lines() {
                                self.tui_dim(format!("    {line}"));
                            }
                        } else {
                            self.tui_dim(format!("    {msg}"));
                        }
                        self.tui_blank();
                    }
                    Ok(None) => self.tui_dim("  ⎿  No assistant replies stored yet."),
                    Err(e) => {
                        self.tui_err(format!("Failed to load last assistant message: {e}"))
                    }
                }
            }
            SlashCmd::Stats(arg) => {
                let sub = arg.as_deref().unwrap_or("").trim();
                let lines = match sub {
                    "model" | "models" => self
                        .session_stats
                        .lock()
                        .render_model_detail(),
                    _ => {
                        // full session card (default)
                        let auth_method = if self.settings.lock().api_key().is_some() {
                            "API Key".to_string()
                        } else {
                            "OAuth / Browser".to_string()
                        };
                        let session_id = self.conversation_id().unwrap_or_default();
                        self.session_stats
                            .lock()
                            .render_card(&auth_method, &session_id)
                    }
                };
                self.tui_blank();
                for line in lines {
                    let _ = self.app.lock().push(line);
                }
                self.tui_blank();
            }
            SlashCmd::Logout => {
                { let mut s = self.settings.lock();
                    s.clear_api_key();
                }
                self.tui_ok("  ✓ API key cleared. Restart CADE to re-authenticate.");
                return Ok(true);
            }
            SlashCmd::Plan => {
                self.permissions.set_mode(PermissionMode::Plan);
                self.app
                    .lock()
                    .show_toast("Permission mode: plan (read-only)", ToastLevel::Info);
                self.tui_hdr("📖 Permission mode: plan (read-only) — write/exec tools blocked. Use /default to resume.");
                self.sync_plan_tools(true).await;
            }
            SlashCmd::Todos => {
                { let mut app = self.app.lock();
                    let mut has_plan = false;
                    let mut now_visible = false;
                    if let Some(plan) = &mut app.active_plan {
                        plan.is_visible = !plan.is_visible;
                        now_visible = plan.is_visible;
                        has_plan = true;
                    }
                    if !has_plan {
                        let _ = app.push(crate::ui::RenderLine::SystemMsg(
                            "No active plan. Ask the agent to use the set_plan tool."
                                .to_string(),
                        ));
                    } else {
                        app.show_toast(
                            if now_visible {
                                "Plan panel shown"
                            } else {
                                "Plan panel hidden"
                            },
                            ToastLevel::Info,
                        );
                    }
                    app.draw_dirty = true;
                    let _ = app.draw();
                }
            }
            SlashCmd::Todo => {
                let content = crate::ui::TuiApp::read_todo_file();
                let _ = self
                    .app
                    .lock()
                    .push(crate::ui::RenderLine::SystemMsg(content));
            }
            SlashCmd::Default => {
                self.permissions.set_mode(PermissionMode::Default);
                self.app
                    .lock()
                    .show_toast("Permission mode: default", ToastLevel::Success);
                self.tui_ok("✅ Permission mode: default — tools require approval");
                self.sync_plan_tools(false).await;
            }
            SlashCmd::Mode(arg) => {
                return self.cmd_mode(arg).await;
            }
            SlashCmd::Model(m) => {
                return self.cmd_model(m, stdout).await;
            }
            SlashCmd::Reasoning(r) => {
                let r = if r.is_empty() {
                    match self
                        .interactive_reasoning_picker(Arc::clone(&self.app))
                        .await?
                    {
                        Some(picked) => picked,
                        None => {
                            let _ = self.app.lock().draw();
                            return Ok(false);
                        }
                    }
                } else {
                    r
                };
                let valid = ["none", "low", "medium", "high", "xhigh"];
                if !valid.contains(&r.as_str()) {
                    self.tui_err(format!("Invalid reasoning tier '{r}'. Valid: none, low, medium, high, xhigh"));
                } else {
                    let effort = if r == "none" { None } else { Some(r.clone()) };
                    *self.reasoning_effort.lock() = effort.clone();
                    {
                        let mut app = self.app.lock();
                        app.reasoning_effort = effort;
                        app.show_toast(format!("Reasoning → {r}"), ToastLevel::Success);
                    }
                    self.tui_ok(format!("  ✓ Reasoning effort: {r}"));
                }
            }

            // -- New commands
            SlashCmd::Clear => {
                let _ = self.app.lock().clear_content();
                match self.client.clear_messages(&self.agent_id()).await {
                    Ok(n) => self
                        .tui_ok(format!("✓ Context window cleared ({n} messages deleted)")),
                    Err(e) => self
                        .tui_sys(format!("⚠ Screen cleared (context clear failed: {e})")),
                }
            }

            SlashCmd::Pricing(arg) => {
                return self.cmd_pricing(arg).await;
            }
            SlashCmd::Cost => {
                return self.cmd_cost().await;
            }
            SlashCmd::Copy => {
                let mut app = self.app.lock();
                app.toggle_copy_mode();
                if app.copy_mode {
                    let _ = app.push(RenderLine::SystemMsg(
                        "Copy mode ON — mouse scroll disabled. Click and drag to select text. /copy to restore.".into()
                    ));
                } else {
                    let _ = app.push(RenderLine::SuccessMsg(
                        "Copy mode OFF — mouse scroll restored.".into(),
                    ));
                }
            }

            SlashCmd::Export(out_arg) => {
                let agent_id = self.agent_id();
                let agent_name = self.agent_name();
                let out_path = out_arg.unwrap_or_else(|| {
                    crate::cli::export_import::default_export_path(&agent_name)
                });
                self.tui_dim(format!("  Exporting agent '{agent_name}' → {out_path} …"));
                match crate::cli::export_import::export_agent_to_file(
                    &self.client,
                    &agent_id,
                    &out_path,
                )
                .await
                {
                    Ok(_) => {
                        self.app.lock().show_toast(
                            format!("Exported → {out_path}"),
                            ToastLevel::Success,
                        );
                        self.tui_ok(format!("  ✓ Exported → {out_path}"))
                    }
                    Err(e) => self.tui_err(format!("  ✗ Export failed: {e}")),
                }
            }

            // -- Checkpoints
            SlashCmd::Checkpoint(label_arg) => {
                return self.cmd_checkpoint(label_arg).await;
            }
            SlashCmd::Undo => {
                let agent_id = self.agent_id();
                match self.client.list_checkpoints(&agent_id).await {
                    Err(e) => self.tui_err(format!("  ✗ list_checkpoints: {e}")),
                    Ok(checkpoints) if checkpoints.is_empty() => {
                        self.tui_dim("  No checkpoints available to undo.".to_string());
                    }
                    Ok(checkpoints) => {
                        if let Some(last_cp) = checkpoints.last() {
                            let checkpoint_id =
                                last_cp["id"].as_str().unwrap_or("").to_string();
                            let stash_ref =
                                last_cp["git_stash_ref"].as_str().map(String::from);

                            self.tui_dim(format!(
                                "  Restoring checkpoint {checkpoint_id}…"
                            ));

                            if let Some(s) = stash_ref {
                                use cade_agent::tools::git_checkpoint;
                                match git_checkpoint::restore_git_checkpoint(&s, &self.cwd)
                                    .await
                                {
                                    Ok(()) => {
                                        self.tui_ok(format!("  ✓ Git stash applied: {s}"))
                                    }
                                    Err(e) => self.tui_err(format!("  ✗ Git restore: {e}")),
                                }
                            }
                            let _ = self
                                .client
                                .restore_checkpoint(&agent_id, &checkpoint_id)
                                .await;
                            self.tui_ok(format!(
                                "  ✓ Restored to checkpoint {checkpoint_id}"
                            ));
                        }
                    }
                }
            }

            SlashCmd::Tree => {
                return self.cmd_tree().await;
            }
            SlashCmd::Fork(label_arg) => {
                return self.cmd_fork(label_arg).await;
            }
            SlashCmd::Backend(backend_arg) => {
                return self.cmd_backend(backend_arg).await;
            }
            SlashCmd::Reflect(focus_arg) => {
                if self.require_capability(
                    cade_core::capabilities::Capability::Agentic,
                    "/reflect",
                ) {
                    return Ok(false);
                }
                let agent_id = self.agent_id();
                let focus = focus_arg.as_deref();
                let focus_msg = focus.map(|f| format!(" (focus: {f})")).unwrap_or_default();
                self.tui_dim(format!("  Reflecting on conversation history{focus_msg}…"));
                match self.client.trigger_reflect(&agent_id, focus).await {
                    Ok(summary) => self.tui_ok(format!("  ✓ {summary}")),
                    Err(e) => self.tui_err(format!("  ✗ Reflect failed: {e}")),
                }
            }

            SlashCmd::Artifacts => {
                if self.require_capability(
                    cade_core::capabilities::Capability::Agentic,
                    "/artifacts",
                ) {
                    return Ok(false);
                }
                let agent_id = self.agent_id();
                match self.client.list_artifacts(&agent_id).await {
                    Err(e) => self.tui_err(format!("  ✗ list_artifacts: {e}")),
                    Ok(arts) if arts.is_empty() => {
                        self.tui_dim("  No artifacts stored yet.".to_string());
                    }
                    Ok(arts) => {
                        self.tui_hdr(format!("  Artifacts ({}):", arts.len()));
                        for a in arts.iter().take(20) {
                            let id = a["id"].as_str().unwrap_or("?");
                            let kind = a["kind"].as_str().unwrap_or("?");
                            let size = a["size_bytes"].as_i64().unwrap_or(0);
                            let ts = a["created_at"].as_i64().unwrap_or(0);
                            let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
                                .map(|d| d.format("%m-%d %H:%M").to_string())
                                .unwrap_or_default();
                            self.tui_dim(format!(
                                "    {kind:<12}  {size:>6}B  {dt}  {}",
                                &id[..12.min(id.len())]
                            ));
                        }
                    }
                }
            }

            SlashCmd::New => {
                let agent_id = self.agent_id();
                match self.client.create_conversation(&agent_id, "").await {
                    Ok(conv) => {
                        let cid = conv["id"].as_str().unwrap_or("").to_string();
                        *self.conversation_id.lock() =
                            Some(cid.clone());
                        { let mut s = self.session.lock();
                            let _ = s.set_conversation(Some(cid.clone()));
                        }
                        self.first_turn
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        self.tui_ok(format!(
                            "  ✓ New conversation started  ({})",
                            &cid[..cid.len().min(20)]
                        ));
                    }
                    Err(e) => self.tui_err(e.to_string()),
                }
            }

            SlashCmd::NewAgent => {
                return self.cmd_newagent(pending_input).await;
            }
            SlashCmd::Resume => {
                return self.cmd_resume().await;
            }
            SlashCmd::Pin => {
                let id = self.agent_id();
                let name = self.agent_name();
                { let mut s = self.settings.lock();
                    match s.pin_agent(&id, &name) {
                        Ok(_) => {
                            self.app.lock().show_toast(
                                format!("Pinned agent: {name}"),
                                ToastLevel::Success,
                            );
                            self.tui_ok(format!("  ✓ Pinned: {name} ({id})"))
                        }
                        Err(e) => self.tui_err(format!("Pin failed: {e}")),
                    }
                }
            }

            SlashCmd::Agents => {
                return self.cmd_agents().await;
            }
            SlashCmd::Delete(target) => {
                return self.cmd_delete(target, stdout, pending_input).await;
            }
            SlashCmd::Init => {
                return self.cmd_init(stdout).await;
            }
            SlashCmd::Remember(text) => {
                // Route through the agent — it decides what to store and where.
                // This matches CADE's /remember behaviour exactly.
                let msg = if text.is_empty() {
                    "[/remember] Please review our recent conversation and update your \
                     memory blocks with anything important you've learned about me, \
                     my preferences, or this project."
                        .to_string()
                } else {
                    format!("[/remember] {text}")
                };
                self.agent_turn(&mut stdout, &msg).await?;
                let _ = self.app.lock().commit_streaming();
            }


            SlashCmd::Memory => {
                return self.cmd_memory(input, stdout, pending_input).await;
            }
            SlashCmd::Search(query) => {
                return self.cmd_search(query).await;
            }
            SlashCmd::Skills(arg) => {
                return self.cmd_skills(arg, stdout, pending_input).await;
            }
            SlashCmd::Subagents => {
                if self.require_capability(
                    cade_core::capabilities::Capability::Agentic,
                    "/subagents",
                ) {
                    return Ok(false);
                }
                let all = discover_all_subagents(&self.cwd);
                match self
                    .subagent_picker(std::sync::Arc::clone(&self.app), &all)
                    .await?
                {
                    Some(SubagentPickerResult::Run(name)) => {
                        *pending_input = Some(format!(
                            "run_subagent(subagent_type=\"{name}\", prompt=\"\")"
                        ));
                    }
                    Some(SubagentPickerResult::Edit(path)) => {
                        // Drop the TUI temporarily, open $EDITOR, then return
                        let editor =
                            std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                        self.tui_sys(format!(
                            "  Opening {} in {}...",
                            path.display(),
                            editor
                        ));

                        let _ = self.app.lock().suspend_for(|| {
                            let mut cmd = std::process::Command::new(&editor);
                            cmd.arg(&path);
                            let _ = cmd.status();
                        });
                        self.tui_ok(format!("  ✓ Finished editing {}", path.display()));
                    }
                    None => {
                        self.tui_dim("  /subagents cancelled".to_string());
                    }
                }
            }

            SlashCmd::Providers => {
                return self.cmd_providers().await;
            }
            SlashCmd::Connect(preset) => {
                return self.cmd_connect(preset, stdout).await;
            }
            SlashCmd::Disconnect(name) => {
                return self.cmd_disconnect(name).await;
            }
            SlashCmd::Permissions => {
                return self.cmd_permissions().await;
            }
            SlashCmd::ApproveAlways(pattern) => {
                return self.cmd_approve_always(pattern).await;
            }
            SlashCmd::DenyAlways(pattern) => {
                return self.cmd_deny_always(pattern).await;
            }
            SlashCmd::Hooks => {
                return self.cmd_hooks().await;
            }
            SlashCmd::Theme(theme_arg) => {
                return self.cmd_theme(theme_arg).await;
            }
            SlashCmd::Rename(new_name) => {
                let id = self.agent_id();
                let new_name = new_name.trim().to_string();
                let name = if new_name.is_empty() {
                    // Prompt for name via QuestionWidget
                    use crate::ui::question::{Question, QuestionOption};
                    let opts = vec![QuestionOption {
                        label: "Cancel".to_string(),
                        description: String::new(),
                    }];
                    let q = Question {
                        header: "Rename agent".to_string(),
                        text: "Enter new agent name:".to_string(),
                        options: opts.clone(),
                        multi_select: false,
                        allow_other: true,
                        progress: None,
                    };
                    let ans = {
                        let mut app = self.app.lock();
                        app.ask_question(&q)?
                    };
                    match &ans {
                        Some(a) if a.as_str() != "Cancel" && !a.as_str().is_empty() => {
                            a.as_str().to_string()
                        }
                        _ => String::new(),
                    }
                } else {
                    new_name
                };
                if name.is_empty() {
                    self.tui_dim("  (cancelled)");
                } else {
                    match self.client.rename_agent(&id, &name).await {
                        Ok(_) => {
                            *self.agent_name.lock() = name.clone();
                            self.tui_ok(format!("  ✓ Renamed to: {name}"));
                        }
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }
            }

            SlashCmd::Toolset(arg) => {
                let old_toolset = *self.current_toolset.lock();
                let new_toolset = if let Some(name) = arg.as_deref() {
                    match cade_core::toolsets::Toolset::from_name(name) {
                        Some(t) => t,
                        None => {
                            self.tui_dim("  Toolsets: default | codex | gemini");
                            return Ok(false);
                        }
                    }
                } else {
                    self.tui_hdr(format!("  Current toolset: {old_toolset:?}"));
                    self.tui_dim("  /toolset default | codex | gemini");
                    return Ok(false);
                };
                if new_toolset != old_toolset {
                    *self.current_toolset.lock() = new_toolset;
                    self.spawn_tool_reregister();
                    self.tui_ok(format!("  ✓ Toolset → {}", new_toolset.display_name()));
                } else {
                    self.tui_dim(format!("  Toolset already: {new_toolset:?}"));
                }
            }

            SlashCmd::Feedback => {
                self.tui_hdr("  Report issues or give feedback:");
                self.tui_sys("  https://github.com/EzekTec-Inc/CADE/issues");
            }
        }
        Ok(false)
    }
}
