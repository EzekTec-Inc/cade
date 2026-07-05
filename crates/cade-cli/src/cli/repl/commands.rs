//! Slash command dispatch for the REPL.
//!
//! Contains the `handle_slash_command` method extracted from the main
//! `run()` event loop.  Returns `Ok(true)` when the REPL should exit,
//! `Ok(false)` to continue the loop.

use std::io;
use std::sync::Arc;

use super::slash::SlashCmd;
use super::{Repl, SubagentPickerResult};
use crate::Result;
use crate::ui::{RenderLine, ToastLevel};
use cade_agent::subagents::discover_all_subagents;
use cade_core::permissions::PermissionMode;

// ── /new helpers (pure) ───────────────────────────────────────────────────────
//
// `/new` clears the agent's `active_goal` memory block so the agent forgets
// the previous task and starts fresh.  Without an archive step the previous
// plan is silently lost — across an accidental `/new` press the agent has no
// way to recover it.  These helpers package the "what to archive" decision
// so it is unit-testable and shared between the slash-command path and the
// `--new` bootstrap path.

/// Build the archival-memory snapshot text for a non-empty `active_goal`
/// block being cleared by `/new`.  Returns `None` when the value is empty
/// or whitespace-only — there is nothing worth archiving in that case.
///
/// The snapshot is plain text with a small header so a future
/// `archival_memory_search("active_goal")` query immediately surfaces it.
pub(crate) fn build_active_goal_archive_snapshot(
    active_goal_value: &str,
    conversation_id: Option<&str>,
) -> Option<String> {
    let trimmed = active_goal_value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let conv_part = conversation_id
        .map(|c| format!(" (conversation {})", &c[..c.len().min(20)]))
        .unwrap_or_default();
    Some(format!(
        "[active_goal snapshot — archived on /new{conv_part}]\n\n{trimmed}"
    ))
}

/// Tags applied to the archival entry created when `/new` clears a non-empty
/// `active_goal`.  Stable strings so users / tests can locate snapshots.
pub(crate) fn active_goal_archive_tags() -> Vec<String> {
    vec![
        "active_goal".to_string(),
        "snapshot".to_string(),
        "slash_new".to_string(),
    ]
}

// ── P4: Structured session handoff ───────────────────────────────────────────

/// Build a structured session handoff note from the current memory state.
///
/// Collects `active_goal`, `recent_edits`, and `session_summary` into a
/// single archival entry with clear sections. Returns `None` if all inputs
/// are empty — nothing worth archiving.
pub(crate) fn build_session_handoff(
    active_goal: &str,
    recent_edits: &str,
    session_summary: &str,
    conversation_id: Option<&str>,
) -> Option<String> {
    let goal = active_goal.trim();
    let edits = recent_edits.trim();
    let summary = session_summary.trim();

    if goal.is_empty() && edits.is_empty() && summary.is_empty() {
        return None;
    }

    let ts = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");
    let conv_part = conversation_id
        .map(|c| format!(" (conversation {})", &c[..c.len().min(20)]))
        .unwrap_or_default();

    let mut sections = vec![format!("## Session Handoff — {ts}{conv_part}")];

    if !goal.is_empty() {
        sections.push(format!("### Task & Status\n{goal}"));
    }
    if !edits.is_empty() {
        // Take only the first 20 lines of recent_edits to keep it compact
        let compact_edits: String = edits.lines().take(20).collect::<Vec<_>>().join("\n");
        sections.push(format!("### Files Modified\n{compact_edits}"));
    }
    if !summary.is_empty() {
        // Take only the last 2000 chars of summary (most recent context)
        let tail: String = if summary.chars().count() > 2_000 {
            let skip = summary.chars().count() - 2_000;
            summary.chars().skip(skip).collect()
        } else {
            summary.to_string()
        };
        sections.push(format!("### Session Context\n{tail}"));
    }

    Some(sections.join("\n\n"))
}

/// Tags for session handoff archival entries.
pub(crate) fn session_handoff_tags() -> Vec<String> {
    vec!["session_handoff".to_string(), "slash_new".to_string()]
}

impl Repl {
    /// Dispatch a parsed slash command.
    ///
    /// Returns `Ok(true)` if the REPL should exit (i.e. `/exit`),
    /// `Ok(false)` to continue the loop.
    pub(crate) async fn handle_slash_command(
        &mut self,
        cmd: SlashCmd,
        input: &str,
        stdout: &mut io::Stdout,
        pending_input: &mut Option<String>,
    ) -> Result<bool> {
        match cmd {
            SlashCmd::Gui => {
                let server_url = self.client.base_url();
                let agent_id = self.agent_id();
                let conv_id = self.conversation_id().unwrap_or_default();
                let api_key = self.client.api_key().to_string();

                let url = format!(
                    "{}/dashboard?agent_id={}&conversation_id={}&api_key={}",
                    server_url, agent_id, conv_id, api_key
                );

                let mut app = self.app.lock();
                let _ = app.push(RenderLine::SystemMsg(
                    "  Opening CADE Web GUI Dashboard in default browser…".to_string(),
                ));

                #[cfg(target_os = "linux")]
                {
                    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
                }
                #[cfg(target_os = "macos")]
                {
                    let _ = std::process::Command::new("open").arg(&url).spawn();
                }
                #[cfg(target_os = "windows")]
                {
                    let _ = std::process::Command::new("cmd").args(["/C", "start", &url]).spawn();
                }

                return Ok(false);
            }
            SlashCmd::Exit => {
                use std::sync::atomic::Ordering;
                let in_tok = self.session_input_tokens.load(Ordering::SeqCst);
                let out_tok = self.session_output_tokens.load(Ordering::SeqCst);
                if in_tok > 0 || out_tok > 0 {
                    let _ = self.app.lock().push(RenderLine::SystemMsg(format!(
                        "  Session tokens — in: {in_tok}  out: {out_tok}  total: {}",
                        in_tok + out_tok
                    )));
                }
                let _ = self
                    .app
                    .lock()
                    .push(RenderLine::SystemMsg("Bye!".to_string()));
                return Ok(true);
            }
            // SlashCmd::Clear is handled below (with context clearing)
            SlashCmd::RunSkill(skill_id, user_prompt) => {
                // Find the skill, build a prompt that injects its content,
                // and send it as an agent turn so the agent follows the skill.
                let skill_body = self
                    .skills
                    .lock()
                    .iter()
                    .find(|s| s.id == skill_id)
                    .map(|s| s.to_context_block());
                if let Some(body) = skill_body {
                    let mut prompt =
                        format!("[Skill invoked: /{skill_id}]\n\nFollow this skill:\n\n{body}");
                    if let Some(prompt_str) = &user_prompt {
                        prompt.push_str("\n\nUser request:\n\n");
                        prompt.push_str(prompt_str);
                    }
                    if let Some(prompt_str) = &user_prompt {
                        self.tui_sys(format!(
                            "  Running skill: /{skill_id} with prompt: {prompt_str}"
                        ));
                    } else {
                        self.tui_sys(format!("  Running skill: /{skill_id}"));
                    }
                    self.agent_turn(stdout, &prompt).await?;
                } else {
                    self.tui_err(format!(
                        "  Skill '{skill_id}' not found. Try /skills reload"
                    ));
                }
                return Ok(false);
            }
            SlashCmd::Help => {
                return self.cmd_help(pending_input).await;
            }
            SlashCmd::Summarize => {
                let memory_blocks = match self.client.get_memory(&self.agent_id()).await {
                    Ok(blocks) => blocks,
                    Err(e) => {
                        self.tui_err(format!("Failed to retrieve memory blocks: {e}"));
                        return Ok(false);
                    }
                };

                let summary = memory_blocks
                    .into_iter()
                    .find(|b| b.label == "session_summary")
                    .map(|b| b.value);

                match summary {
                    Some(text) if !text.is_empty() => {
                        self.app
                            .lock()
                            .overlays
                            .push(Box::new(crate::ui::app::SummaryState { text, scroll_y: 0 }));
                        let _ = self.app.lock().draw();
                    }
                    _ => {
                        self.app.lock().show_toast(
                            "Conversation is too short for a background summary.",
                            ToastLevel::Warning,
                        );
                        let _ = self.app.lock().draw();
                    }
                }
                return Ok(false);
            }
            SlashCmd::Agent => {
                let msg = format!("  Agent: {} ({})", self.agent_name(), self.agent_id());
                let _ = self.app.lock().push(RenderLine::SystemMsg(msg));
            }
            SlashCmd::Info => {
                return self.cmd_info().await;
            }
            SlashCmd::Yolo => {
                return self.cmd_yolo().await;
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
            SlashCmd::Link(arg) => {
                return self.cmd_link(arg, pending_input).await;
            }
            SlashCmd::Unlink(arg) => {
                return self.cmd_unlink(arg).await;
            }
            SlashCmd::Stream => {
                return self.cmd_stream().await;
            }
            SlashCmd::Usage => {
                return self.cmd_usage(pending_input).await;
            }
            SlashCmd::Context => {
                return self.cmd_context(stdout).await;
            }
            SlashCmd::DebugLast => {
                return self.cmd_debug_last(pending_input).await;
            }
            SlashCmd::Stats(arg) => {
                return self.cmd_stats(arg).await;
            }
            SlashCmd::Logout => {
                {
                    let mut s = self.settings.lock();
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
                let mut app = self.app.lock();
                let mut has_plan = false;
                let mut now_visible = false;
                if let Some(plan) = &mut app.active_plan {
                    plan.is_visible = !plan.is_visible;
                    now_visible = plan.is_visible;
                    has_plan = true;
                }
                if !has_plan {
                    let _ = app.push(crate::ui::RenderLine::SystemMsg(
                        "No active plan. Ask the agent to use the set_plan tool.".to_string(),
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
            SlashCmd::CompactionModel(m) => {
                let m_opt = if m.trim().is_empty() {
                    None
                } else {
                    Some(m.trim())
                };
                match self
                    .client
                    .patch_agent_compaction_model(&self.agent_id(), m_opt)
                    .await
                {
                    Ok(_) => {
                        let msg = if let Some(model) = m_opt {
                            format!("✅ Compaction model set to {model}")
                        } else {
                            "✅ Compaction model cleared (using main model)".to_string()
                        };
                        self.app.lock().show_toast(&msg, ToastLevel::Success);
                        self.tui_ok(msg);
                    }
                    Err(e) => self.tui_err(format!("Failed to set compaction model: {e}")),
                }
                return Ok(false);
            }
            SlashCmd::Compact => {
                self.app.lock().show_toast(
                    "Compacting context — consolidating dropped turns…",
                    ToastLevel::Info,
                );
                let _ = self.app.lock().draw();
                let agent_id = self.agent_id();
                match self.client.compact(&agent_id, None).await {
                    Ok(chars) => {
                        let msg = if chars > 0 {
                            format!("✓ Context compacted (session_summary: {chars} chars)")
                        } else {
                            "✓ Compact triggered (nothing to consolidate yet)".to_string()
                        };
                        self.app.lock().show_toast(&msg, ToastLevel::Success);
                        self.tui_ok(msg);
                    }
                    Err(e) => {
                        let msg = format!("Compact failed: {e}");
                        self.app.lock().show_toast(&msg, ToastLevel::Error);
                        self.tui_err(msg);
                    }
                }
                return Ok(false);
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
                    self.tui_err(format!(
                        "Invalid reasoning tier '{r}'. Valid: none, low, medium, high, xhigh"
                    ));
                } else {
                    let effort = if r == "none" { None } else { Some(r.clone()) };
                    *self.reasoning_effort.lock() = effort.clone();

                    if let Err(e) = self.settings.lock().set_reasoning_effort(effort.clone()) {
                        self.tui_err(format!("Failed to save reasoning effort to settings: {e}"));
                    }

                    {
                        let mut app = self.app.lock();
                        app.reasoning_effort = effort;
                        app.show_toast(format!("Reasoning → {r}"), ToastLevel::Success);
                    }
                    self.tui_ok(format!("  ✓ Reasoning effort: {r}"));
                }
            }

            // -- New commands
            SlashCmd::Reload => {
                return self.cmd_reload().await;
            }
            SlashCmd::Trust => {
                let cwd = self.cwd.clone();
                let mut settings = self.settings.lock();
                match settings.trust_directory(&cwd) {
                    Ok(_) => {
                        self.tui_ok(format!(
                            "  ✓ Directory trusted: {}. Project-local settings and MCP servers are now active.",
                            cwd.display()
                        ));
                        let _ = settings.reload();
                        self.app
                            .lock()
                            .show_toast("Directory trusted successfully", ToastLevel::Success);
                    }
                    Err(e) => {
                        self.tui_err(format!("  ✗ Failed to trust directory: {}", e));
                    }
                }
                return Ok(false);
            }
            SlashCmd::Mouse => {
                self.app.lock().toggle_mouse_capture();
                return Ok(false);
            }
            SlashCmd::Update => {
                let _ = self.app.lock().suspend();
                let res = crate::cli::update::run_update(false).await;
                let _ = self.app.lock().resume();

                res.map_err(|e| crate::error::Error::custom(e.to_string()))?;

                self.tui_ok("Update complete! Please restart CADE.".to_string());
                return Ok(false);
            }
            SlashCmd::Marketplace => {
                return self.cmd_marketplace().await;
            }
            SlashCmd::Clear => {
                return self.cmd_clear().await;
            }
            SlashCmd::Pricing(arg) => {
                return self.cmd_pricing(arg).await;
            }
            SlashCmd::Cost => {
                return self.cmd_cost().await;
            }

            SlashCmd::Export(out_arg) => {
                return self.cmd_export(out_arg).await;
            }
            SlashCmd::Checkpoint(label_arg) => {
                return self.cmd_checkpoint(label_arg).await;
            }
            SlashCmd::Undo => {
                return self.cmd_undo().await;
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
                return self.cmd_reflect(focus_arg).await;
            }
            SlashCmd::Artifacts => {
                return self.cmd_artifacts().await;
            }
            SlashCmd::New => {
                let agent_id = self.agent_id();
                match self.client.create_conversation(&agent_id, "").await {
                    Ok(conv) => {
                        let cid = conv["id"].as_str().unwrap_or("").to_string();

                        // P4: Build a structured session handoff before clearing state.
                        // Collects active_goal + recent_edits + session_summary into a
                        // rich snapshot that survives in archival memory.
                        let blocks = self.client.get_memory(&agent_id).await.unwrap_or_default();
                        let active_goal = blocks
                            .iter()
                            .find(|b| b.label == "active_goal")
                            .map(|b| b.value.as_str())
                            .unwrap_or("");
                        let recent_edits = blocks
                            .iter()
                            .find(|b| b.label == "recent_edits")
                            .map(|b| b.value.as_str())
                            .unwrap_or("");
                        let session_summary = blocks
                            .iter()
                            .find(|b| b.label == "session_summary")
                            .map(|b| b.value.as_str())
                            .unwrap_or("");

                        // Build the structured handoff note
                        if let Some(handoff) = build_session_handoff(
                            active_goal,
                            recent_edits,
                            session_summary,
                            Some(&cid),
                        ) {
                            let _ = self
                                .client
                                .insert_archival_memory(
                                    &agent_id,
                                    &handoff,
                                    &session_handoff_tags(),
                                )
                                .await;
                        }

                        // Also archive raw active_goal for backward compat
                        if let Some(snapshot) =
                            build_active_goal_archive_snapshot(active_goal, Some(&cid))
                        {
                            let _ = self
                                .client
                                .insert_archival_memory(
                                    &agent_id,
                                    &snapshot,
                                    &active_goal_archive_tags(),
                                )
                                .await;
                        }

                        // Clear the active_goal memory block so the agent forgets the previous task
                        let _ = self.client.delete_memory(&agent_id, "active_goal").await;
                        // Reset the C3 staleness counter so the next session starts clean
                        // and the recurring `update_memory(active_goal)` reminder fires
                        // on schedule.
                        self.write_tool_calls
                            .store(0, std::sync::atomic::Ordering::SeqCst);
                        self.writes_at_last_active_goal_update
                            .store(0, std::sync::atomic::Ordering::SeqCst);
                        *self.conversation_id.lock() = Some(cid.clone());
                        {
                            let mut s = self.session.lock();
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
                return self.cmd_pin().await;
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
                return self.cmd_remember(text).await;
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
                if self
                    .require_capability(cade_core::capabilities::Capability::Agentic, "/subagents")
                {
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
                        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                        self.tui_sys(format!("  Opening {} in {}...", path.display(), editor));

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

            SlashCmd::Teams => {
                if self.require_capability(cade_core::capabilities::Capability::Agentic, "/teams") {
                    return Ok(false);
                }
                let all_teams = cade_agent::team::discovery::discover_all_teams(&self.cwd);
                if all_teams.is_empty() {
                    self.tui_dim("  No teams discovered.".to_string());
                } else {
                    self.tui_blank();
                    self.tui_hdr("  Available teams:");
                    for team in &all_teams {
                        self.tui_sys(team.summary());
                        for member in &team.members {
                            self.tui_dim(format!(
                                "    └─ {} ({}) — {}",
                                member.id, member.tools, member.description
                            ));
                        }
                    }
                    self.tui_blank();
                    self.tui_dim(
                        "  Tip: Use run_team(task=\"...\", team=\"<id>\") to delegate work."
                            .to_string(),
                    );
                }
            }

            SlashCmd::Approvals => {
                return self.cmd_approvals().await;
            }
            SlashCmd::Approve(id) => {
                return self.cmd_approve(id).await;
            }
            SlashCmd::Deny(id) => {
                return self.cmd_deny(id).await;
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
                return self.cmd_rename(new_name).await;
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

    pub(crate) async fn cmd_approvals(&self) -> Result<bool> {
        match self.client.raw_get("/approvals").await {
            Ok(v) => {
                let approvals = v["approvals"].as_array();
                if approvals.is_none() || approvals.unwrap().is_empty() {
                    self.tui_dim("  No pending approvals found.".to_string());
                    return Ok(false);
                }
                self.tui_blank();
                self.tui_hdr("  Pending approvals queue:");
                for app in approvals.unwrap() {
                    let id = app["id"].as_str().unwrap_or("?");
                    let subagent = app["subagent_id"].as_str().unwrap_or("?");
                    let tool = app["tool_name"].as_str().unwrap_or("?");
                    let args = app["arguments"].as_str().unwrap_or("{}");
                    self.tui_sys(format!("  [{id}] Subagent: {} -> tool: {}", subagent, tool));
                    self.tui_dim(format!("    arguments: {}", args));
                }
                self.tui_blank();
                self.tui_dim("  Tip: Use /approve <id> or /deny <id> to take action.".to_string());
            }
            Err(e) => self.tui_err(format!("Failed to list approvals: {e}")),
        }
        Ok(false)
    }

    pub(crate) async fn cmd_approve(&self, id: String) -> Result<bool> {
        let trimmed_id = id.trim();
        if trimmed_id.is_empty() {
            self.tui_err("Usage: /approve <id>".to_string());
            return Ok(false);
        }
        let body = serde_json::json!({ "action": "approve" });
        match self
            .client
            .raw_post(&format!("/approvals/{trimmed_id}/action"), &body)
            .await
        {
            Ok(_) => {
                self.tui_ok(format!(
                    "  ✓ Request '{trimmed_id}' APPROVED successfully. Subagent resumed."
                ));
            }
            Err(e) => self.tui_err(format!("Failed to approve request: {e}")),
        }
        Ok(false)
    }

    pub(crate) async fn cmd_deny(&self, id: String) -> Result<bool> {
        let trimmed_id = id.trim();
        if trimmed_id.is_empty() {
            self.tui_err("Usage: /deny <id>".to_string());
            return Ok(false);
        }
        let body = serde_json::json!({ "action": "deny" });
        match self
            .client
            .raw_post(&format!("/approvals/{trimmed_id}/action"), &body)
            .await
        {
            Ok(_) => {
                self.tui_ok(format!(
                    "  ✗ Request '{trimmed_id}' DENIED successfully. Subagent notified."
                ));
            }
            Err(e) => self.tui_err(e.to_string()),
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_active_goal_archive_snapshot ──────────────────────────────────

    #[test]
    fn snapshot_none_when_value_is_empty() {
        assert!(build_active_goal_archive_snapshot("", None).is_none());
    }

    #[test]
    fn snapshot_none_when_value_is_whitespace() {
        assert!(build_active_goal_archive_snapshot("   \n\t  ", None).is_none());
    }

    #[test]
    fn snapshot_includes_value_and_header_no_conv() {
        let s = build_active_goal_archive_snapshot("Working on M1.", None)
            .expect("non-empty value must produce a snapshot");
        assert!(s.contains("active_goal snapshot"));
        assert!(s.contains("/new"));
        assert!(s.contains("Working on M1."));
        // No conversation suffix when conversation_id is None.
        assert!(!s.contains("conversation "));
    }

    #[test]
    fn snapshot_includes_conversation_id_when_provided() {
        let s = build_active_goal_archive_snapshot("Some plan", Some("conv-abc-123"))
            .expect("non-empty value must produce a snapshot");
        assert!(s.contains("conversation conv-abc-123"));
        assert!(s.contains("Some plan"));
    }

    #[test]
    fn snapshot_truncates_long_conversation_id_in_header() {
        // Defensive: header should not embed multi-hundred-char conv ids.
        let long_id = "a".repeat(100);
        let s =
            build_active_goal_archive_snapshot("plan", Some(&long_id)).expect("snapshot present");
        // Header takes only first 20 chars of the id.
        assert!(s.contains(&"a".repeat(20)));
        assert!(!s.contains(&"a".repeat(21)));
    }

    #[test]
    fn snapshot_trims_value_whitespace() {
        let s = build_active_goal_archive_snapshot("  spaced plan  \n", None)
            .expect("snapshot present");
        // Trimmed plan should appear; leading/trailing whitespace gone.
        assert!(s.ends_with("spaced plan"));
    }

    // ── active_goal_archive_tags ────────────────────────────────────────────

    #[test]
    fn archive_tags_are_stable() {
        let tags = active_goal_archive_tags();
        assert!(tags.contains(&"active_goal".to_string()));
        assert!(tags.contains(&"snapshot".to_string()));
        assert!(tags.contains(&"slash_new".to_string()));
    }
}
