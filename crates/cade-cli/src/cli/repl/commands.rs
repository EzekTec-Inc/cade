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
                if self.require_capability(cade_core::capabilities::Capability::Mcp, "/mcp")
                {
                    return Ok(false);
                }
                // Support "/mcp reload" subcommand
                let sub = input.trim().strip_prefix("/mcp").unwrap_or("").trim();
                if sub == "reload" {
                    self.do_settings_reload().await;
                    return Ok(false);
                }

                match self
                    .interactive_mcp_picker(std::sync::Arc::clone(&self.app))
                    .await?
                {
                    Some(cade_tui::mcp_picker::McpAction::Toggle(key)) => {
                        let mut s = self.settings.lock();
                        if let Some(server) =
                            s.global_settings_mut().mcp_servers.get_mut(&key)
                        {
                            server.disabled = !server.disabled;
                        }
                        let _ = s.save_global();
                        drop(s);
                        self.do_settings_reload().await;
                    }
                    Some(cade_tui::mcp_picker::McpAction::Delete(key)) => {
                        let mut s = self.settings.lock();
                        s.global_settings_mut().mcp_servers.remove(&key);
                        let _ = s.save_global();
                        drop(s);
                        self.do_settings_reload().await;
                    }
                    Some(cade_tui::mcp_picker::McpAction::New) => {
                        let tmpl = serde_json::json!({
                            "new_server": {
                                "command": "npx",
                                "args": ["-y", "@modelcontextprotocol/server-everything"],
                                "env": {},
                                "disabled": false
                            }
                        });
                        let mut app = self.app.lock();
                        let text = format!(
                            "/mcp-save\n{}",
                            serde_json::to_string_pretty(&tmpl).unwrap()
                        );
                        app.editor.set_text(text.clone());
                        app.editor.set_cursor_pos(text.len());
                        app.push_silent(crate::ui::RenderLine::SystemMsg(
                            "  Edit the JSON below and hit Enter to create/save."
                                .to_string(),
                        ));
                    }
                    Some(cade_tui::mcp_picker::McpAction::Edit(key)) => {
                        let config = self
                            .settings
                            .lock()
                            .global_settings_mut()
                            .mcp_servers
                            .get(&key)
                            .cloned()
                            .unwrap_or_default();
                        let tmpl = serde_json::json!({ key: config });
                        let mut app = self.app.lock();
                        let text = format!(
                            "/mcp-save\n{}",
                            serde_json::to_string_pretty(&tmpl).unwrap()
                        );
                        app.editor.set_text(text.clone());
                        app.editor.set_cursor_pos(text.len());
                        app.push_silent(crate::ui::RenderLine::SystemMsg(
                            "  Edit the JSON below and hit Enter to save.".to_string(),
                        ));
                    }
                    None => {
                        self.tui_dim("  /mcp closed");
                    }
                }
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
                use crate::cli::repl::format::parse_mode_label;
                match arg.as_deref() {
                    None | Some("") => {
                        let (icon, label, hint) = mode_display(self.permissions.mode());
                        self.tui_sys(format!("{icon} Current mode: {label}  {hint}"));
                    }
                    Some(name) => {
                        let resolved = parse_mode_label(name);
                        match resolved {
                            Some("default") => {
                                self.permissions.set_mode(PermissionMode::Default);
                                let (icon, label, _) =
                                    mode_display(PermissionMode::Default);
                                self.app.lock().show_toast(
                                    format!("{icon} {label}"),
                                    ToastLevel::Success,
                                );
                                self.tui_ok(format!("{icon} Permission mode: {label}"));
                                self.sync_plan_tools(false).await;
                            }
                            Some("plan") => {
                                self.permissions.set_mode(PermissionMode::Plan);
                                let (icon, label, hint) =
                                    mode_display(PermissionMode::Plan);
                                self.app.lock().show_toast(
                                    format!("{icon} {label}"),
                                    ToastLevel::Info,
                                );
                                self.tui_hdr(format!(
                                    "{icon} Permission mode: {label} {hint}"
                                ));
                                self.sync_plan_tools(true).await;
                            }
                            Some("yolo") => {
                                self.permissions
                                    .set_mode(PermissionMode::BypassPermissions);
                                let (icon, label, _) =
                                    mode_display(PermissionMode::BypassPermissions);
                                self.app.lock().show_toast(
                                    format!("{icon} {label}"),
                                    ToastLevel::Warning,
                                );
                                self.tui_sys(format!("{icon} Permission mode: {label}"));
                                self.sync_plan_tools(false).await;
                            }
                            Some("acceptEdits") => {
                                self.permissions.set_mode(PermissionMode::AcceptEdits);
                                let (icon, label, _) =
                                    mode_display(PermissionMode::AcceptEdits);
                                self.app.lock().show_toast(
                                    format!("{icon} {label}"),
                                    ToastLevel::Success,
                                );
                                self.tui_ok(format!("{icon} Permission mode: {label}"));
                                self.sync_plan_tools(false).await;
                            }
                            _ => {
                                self.tui_err(format!(
                                    "Unknown mode '{name}'. Valid: safe | edit-freely | plan | full-access (or: default | acceptEdits | yolo)"
                                ));
                            }
                        }
                    }
                }
            }
            // SlashCmd::New is handled below (hot-swap)
            SlashCmd::Model(m) => {
                // Empty arg → open interactive picker
                let m = if m.is_empty() {
                    match self.interactive_model_picker(Arc::clone(&self.app)).await? {
                        Some(picked) => picked,
                        None => {
                            let _ = self.app.lock().draw();
                            return Ok(false);
                        }
                    }
                } else {
                    m
                };
                let new_toolset = Toolset::for_model(&m);
                let old_toolset = *self.current_toolset.lock();
                self.tui_dim(format!("  Switching model → {m}…"));
                match self.client.patch_agent_model(&self.agent_id(), &m).await {
                    Ok(new_model) => {
                        *self.current_model.lock() =
                            new_model.clone();
                        if new_toolset != old_toolset {
                            *self.current_toolset.lock() =
                                new_toolset;
                            self.spawn_tool_reregister();
                            self.tui_hdr(format!(
                                "  Toolset → {}",
                                new_toolset.display_name()
                            ));
                        }
                        self.tui_ok(format!("  ✓ Model: {new_model}"));
                        {
                            let mut app = self.app.lock();
                            app.show_toast(
                                format!("Model → {new_model}"),
                                ToastLevel::Success,
                            );
                            let _ = app.draw();
                        }
                    }
                    Err(e) => self.tui_err(e.to_string()),
                }
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

            SlashCmd::Pricing(arg) => match arg.as_deref() {
                Some("sync") => {
                    self.tui_dim("  Fetching latest pricing rules from cloud...");
                    let url = "https://raw.githubusercontent.com/EzekTec-Inc/CADE/main/crates/cade-ai/src/default_pricing.json";
                    match reqwest::get(url).await {
                        Ok(res) if res.status().is_success() => {
                            if let Ok(text) = res.text().await
                                && let Some(p) = dirs::home_dir()
                                    .map(|h| h.join(".cade").join("pricing.json"))
                            {
                                if let Err(e) = std::fs::write(&p, text) {
                                    self.tui_err(format!(
                                        "  Failed to write pricing.json: {}",
                                        e
                                    ));
                                } else {
                                    let mut stats =
                                        self.session_stats.lock();
                                    stats.registry = std::sync::Arc::new(
                                        cade_ai::ModelRegistry::load_or_default(Some(&p)),
                                    );
                                    self.tui_ok("  Pricing synced successfully!");
                                }
                            }
                        }
                        _ => self.tui_err("  Failed to fetch pricing from cloud."),
                    }
                }
                Some(cmd) if cmd.starts_with("set ") => {
                    self.tui_err("  /pricing set is not fully implemented yet. Please edit ~/.cade/pricing.json manually.");
                }
                _ => {
                    let model = self.model();
                    let stats = self.session_stats.lock();
                    let pricing = stats.registry.pricing_for_model(&model);
                    self.tui_hdr(format!("  Pricing for model: {}", model));
                    self.tui_dim(format!("  Input: ${}/1M", pricing.input));
                    self.tui_dim(format!("  Output: ${}/1M", pricing.output));
                    self.tui_dim(format!("  Cache Read: ${}/1M", pricing.cache_read));
                    self.tui_dim(format!("  Cache Write: ${}/1M", pricing.cache_write));
                    self.tui_dim("  Use /pricing sync to update from the cloud, or edit ~/.cade/pricing.json to add local overrides.");
                }
            },

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
                let agent_id = self.agent_id();
                let label = label_arg.as_deref().unwrap_or("manual");
                self.tui_dim(format!("  Creating checkpoint '{label}'…"));

                // Git stash if dirty
                use cade_agent::tools::git_checkpoint;
                let git_cp = git_checkpoint::create_git_checkpoint(label, &self.cwd).await;
                let stash = git_cp
                    .as_ref()
                    .and_then(|g| g.stash_ref.as_deref())
                    .map(String::from);
                let commit = git_cp
                    .as_ref()
                    .and_then(|g| g.commit_hash.as_deref())
                    .map(String::from);
                let conv_id = self.conversation_id();

                match self
                    .client
                    .create_checkpoint(
                        &agent_id,
                        Some(label),
                        None,
                        conv_id.as_deref(),
                        stash.as_deref(),
                        commit.as_deref(),
                    )
                    .await
                {
                    Ok(cp_id) => {
                        let mut msg = format!("  ✓ Checkpoint '{label}' — ID: {cp_id}");
                        if stash.is_some() {
                            msg.push_str("  (git stashed)");
                        }
                        self.app.lock().show_toast(
                            format!("Checkpoint '{label}' created"),
                            ToastLevel::Success,
                        );
                        self.tui_ok(msg);
                    }
                    Err(e) => self.tui_err(format!("  ✗ Checkpoint failed: {e}")),
                }
            }

            // -- Undo
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
                let agent_id = self.agent_id();
                let label = label_arg.as_deref().unwrap_or("fork");
                self.tui_dim(format!("  Creating fork point '{label}'…"));
                use cade_agent::tools::git_checkpoint;
                let git_cp = git_checkpoint::create_git_checkpoint(label, &self.cwd).await;
                let stash = git_cp
                    .as_ref()
                    .and_then(|g| g.stash_ref.as_deref())
                    .map(String::from);
                let commit = git_cp
                    .as_ref()
                    .and_then(|g| g.commit_hash.as_deref())
                    .map(String::from);

                // Create a checkpoint as the fork anchor
                match self
                    .client
                    .create_checkpoint(
                        &agent_id,
                        Some(label),
                        Some("fork anchor"),
                        self.conversation_id().as_deref(),
                        stash.as_deref(),
                        commit.as_deref(),
                    )
                    .await
                {
                    Ok(cp_id) => {
                        // Start a new conversation from this point
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
                                    "  ✓ Forked from checkpoint {cp_id}  →  new conversation {}",
                                    &cid[..cid.len().min(16)]
                                ));
                            }
                            Err(e) => self.tui_err(format!("  ✗ Create conversation: {e}")),
                        }
                    }
                    Err(e) => self.tui_err(format!("  ✗ Fork failed: {e}")),
                }
            }

            SlashCmd::Backend(backend_arg) => {
                let current = self.exec_backend.name();
                match backend_arg {
                    None => {
                        self.tui_hdr(format!("  Execution backend: {current}"));
                        self.tui_dim(
                            "  Available: local, docker, ssh, readonly".to_string(),
                        );
                        self.tui_dim(
                            "  Change: /backend local|docker|ssh|readonly".to_string(),
                        );
                        self.tui_dim("  Or set in ~/.cade/settings.json: { \"execution\": { \"backend\": \"docker\" } }".to_string());
                    }
                    Some(new_backend) => {
                        use cade_core::settings::ExecutionBackendKind;

                        match new_backend.parse::<ExecutionBackendKind>() {
                            Err(e) => self.tui_err(format!("  ✗ {e}")),
                            Ok(kind) => {
                                // Build a new backend from the current settings profile
                                // with the backend kind overridden
                                let profile = {
                                    let s = self.settings.lock();
                                    let mut p = s.execution_profile().clone();
                                    p.backend = kind;
                                    p
                                };
                                let new_b =
                                    cade_agent::backends::backend_from_profile(&profile);
                                let name = new_b.name();
                                self.exec_backend = std::sync::Arc::from(new_b);
                                self.tui_ok(format!("  ✓ Switched to {name} backend"));
                                if name == "docker" {
                                    let docker_image = profile
                                        .docker_image
                                        .as_deref()
                                        .unwrap_or("ubuntu:22.04");
                                    self.tui_dim(format!("  Image: {docker_image}  (set execution.docker_image in settings to change)"));
                                } else if name == "ssh" {
                                    let host = profile
                                        .ssh_host
                                        .as_deref()
                                        .unwrap_or("(not configured)");
                                    self.tui_dim(format!("  Host: {host}  (set execution.ssh_host in settings)"));
                                }
                            }
                        }
                    }
                }
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
                self.tui_dim("  Fetching conversations…");
                let agent_id = self.agent_id();
                match self.client.list_conversations(&agent_id).await {
                    Ok(convs) => {
                        if convs.is_empty() {
                            let _ =
                                self.app
                                    .lock()
                                    .push(RenderLine::DimMsg(
                                    "  No saved conversations yet. Use /new to start one."
                                        .to_string(),
                                ));
                        } else if let Some(picked) = self
                            .conversation_picker(Arc::clone(&self.app), &convs, &agent_id)
                            .await?
                        {
                            let cid = picked["id"].as_str().unwrap_or("").to_string();
                            *self.conversation_id.lock() =
                                Some(cid.clone());
                            { let mut s = self.session.lock();
                                let _ = s.set_conversation(Some(cid));
                            }
                            self.first_turn
                                .store(false, std::sync::atomic::Ordering::SeqCst);
                            let _ = self.app.lock().push(
                                RenderLine::SuccessMsg(format!(
                                    "  ✓ Switched to: {}",
                                    picked["title"].as_str().unwrap_or("(untitled)")
                                )),
                            );
                        }
                        let _ = self.app.lock().draw();
                    }
                    Err(e) => {
                        let _ = self
                            .app
                            .lock()
                            .push(RenderLine::ErrorMsg(e.to_string()));
                    }
                }
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
                // /delete [name-or-id] — delete a specific agent by name/id prefix
                let agents = match self.client.list_agents().await {
                    Ok(a) => a,
                    Err(e) => {
                        self.print_error(&mut stdout, &e.to_string())?;
                        vec![]
                    }
                };
                if agents.is_empty() {
                    self.tui_dim("  (no agents)");
                } else if let Some(query) = target {
                    let q = query.to_lowercase();
                    let matched: Vec<_> = agents
                        .iter()
                        .filter(|a| {
                            a.name.to_lowercase().contains(&q) || a.id.starts_with(&q)
                        })
                        .collect();
                    match matched.len() {
                        0 => self.tui_err(format!("No agent matching '{query}'")),
                        1 => {
                            let a = matched[0];
                            use crate::ui::question::{Question, QuestionOption};
                            let opts = vec![
                                QuestionOption {
                                    label: "Yes — delete".to_string(),
                                    description: String::new(),
                                },
                                QuestionOption {
                                    label: "No — cancel".to_string(),
                                    description: String::new(),
                                },
                            ];
                            let q_widget = Question {
                                header: "Confirm delete".to_string(),
                                text: format!("Delete '{}'?", a.name),
                                options: opts.clone(),
                                multi_select: false,
                                allow_other: false,
                                progress: None,
                            };
                            let confirmed = {
                                let mut app = self.app.lock();
                                let r = app.ask_question(&q_widget)?;
                                app.scroll = 0;
                                let _ = app.draw();
                                matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
                            };
                            if confirmed {
                                match self.client.delete_agent(&a.id).await {
                                    Ok(_) => {
                                        self.tui_ok(format!("  ✓ Deleted: {}", a.name));
                                        if a.id == self.agent_id() {
                                            self.tui_dim("  Active agent deleted — use /new or /agents to continue");
                                        }
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            } else {
                                self.tui_dim("  (cancelled)");
                            }
                        }
                        n => self.tui_err(format!(
                            "{n} agents match '{query}' — be more specific"
                        )),
                    }
                } else {
                    self.tui_dim("  Usage: /delete <name-or-id>  or  /agents then press d");
                }
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
                if query.is_empty() {
                    self.tui_dim("  Usage: /search <query>");
                    return Ok(false);
                }
                // Run both searches concurrently
                let agent_id = self.agent_id();
                let (msg_res, mem_res) = tokio::join!(
                    self.client.search_messages(&agent_id, &query),
                    self.client.search_memory(&agent_id, &query),
                );

                let msgs_empty = msg_res.as_ref().map(|v| v.is_empty()).unwrap_or(true);
                let mem_empty = mem_res.as_ref().map(|v| v.is_empty()).unwrap_or(true);

                if msgs_empty && mem_empty && msg_res.is_ok() && mem_res.is_ok() {
                    self.tui_dim(format!("  No results for '{query}'"));
                } else {
                    self.tui_blank();
                    self.tui_hdr(format!("  Search results for '{query}'"));
                    self.tui_blank();

                    // Message results (FTS5 BM25-ranked)
                    match &msg_res {
                        Ok(msgs) if !msgs.is_empty() => {
                            self.tui_dim(format!(
                                "  ── Messages ({} match(es)) ──",
                                msgs.len()
                            ));
                            for m in msgs.iter().take(8) {
                                let role = m["role"].as_str().unwrap_or("?");
                                let snippet = m["snippet"].as_str().unwrap_or("").trim();
                                let display = if snippet.is_empty() {
                                    m["content"]["content"]
                                        .as_str()
                                        .or_else(|| m["content"].as_str())
                                        .unwrap_or("")
                                        .chars()
                                        .take(100)
                                        .collect::<String>()
                                } else {
                                    snippet.chars().take(120).collect::<String>()
                                };
                                let score = m["score"].as_f64().unwrap_or(0.0);
                                self.tui_dim(format!(
                                    "  [{role}] (bm25 {score:.2})  {display}"
                                ));
                            }
                            self.tui_blank();
                        }
                        Err(e) => self.tui_err(format!("  Message search error: {e}")),
                        _ => {}
                    }

                    // Memory results (LIKE search)
                    match &mem_res {
                        Ok(blocks) if !blocks.is_empty() => {
                            self.tui_dim(format!(
                                "  ── Memory ({} match(es)) ──",
                                blocks.len()
                            ));
                            for b in blocks.iter().take(5) {
                                let label = b["label"].as_str().unwrap_or("?");
                                let snippet = b["snippet"].as_str().unwrap_or("").trim();
                                let display: String = snippet.chars().take(120).collect();
                                self.tui_dim(format!("  [{label}]  {display}"));
                            }
                            self.tui_blank();
                        }
                        Err(e) => self.tui_err(format!("  Memory search error: {e}")),
                        _ => {}
                    }
                }
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

            SlashCmd::Providers => match self.client.list_providers().await {
                Ok(body) => {
                    let empty = vec![];
                    let providers = body["providers"].as_array().unwrap_or(&empty);
                    self.tui_blank();
                    self.tui_hdr(format!("  Configured providers ({}):", providers.len()));
                    for p in providers {
                        let name = p["name"].as_str().unwrap_or("?");
                        let kind = p["kind"].as_str().unwrap_or("?");
                        let live = p["live"].as_bool().unwrap_or(false);
                        let source = p["source"].as_str().unwrap_or("db");
                        let enabled = p["enabled"].as_bool().unwrap_or(true);
                        let status = if live { "✓ live" } else { "✗ offline" };
                        let display_name = if enabled {
                            name.to_string()
                        } else {
                            format!("{name} (disabled)")
                        };
                        if live {
                            self.tui_ok(format!(
                                "  {status:<10} {display_name:<18} [{kind}] ({source})"
                            ));
                        } else {
                            self.tui_err(format!(
                                "  {status:<10} {display_name:<18} [{kind}] ({source})"
                            ));
                        }
                    }
                    self.tui_blank();
                    self.tui_dim("  /connect <name>    — add a provider");
                    self.tui_dim("  /disconnect <name> — remove a provider");
                    let presets = self.client.list_provider_presets().await;
                    if !presets.is_empty() {
                        self.tui_dim("  OpenAI-compatible presets:");
                        for p in &presets {
                            let n = p["name"].as_str().unwrap_or("?");
                            let u = p["base_url"].as_str().unwrap_or("?");
                            self.tui_dim(format!("    /connect {n:<14} — {u}"));
                        }
                    }
                    self.tui_blank();
                }
                Err(e) => self.tui_err(e.to_string()),
            },

            SlashCmd::Connect(preset) => {
                self.handle_connect(preset, &mut stdout).await?;
            }

            SlashCmd::Disconnect(name) => {
                if name.is_empty() {
                    self.tui_err("/disconnect requires a provider name");
                } else {
                    self.tui_dim(format!("  Disconnecting provider '{name}'…"));
                    match self.client.remove_provider(&name).await {
                        Ok(_) => self.tui_ok(format!("  ✓ Provider '{name}' removed")),
                        Err(e) => self.tui_err(e.to_string()),
                    }
                }
            }

            SlashCmd::Permissions => {
                let mode = self.permissions.mode();
                let allow = self.permissions.allow_rules();
                let deny = self.permissions.deny_rules();

                let (icon, label, _) = mode_display(mode);
                let mode_hint = match mode {
                    cade_core::permissions::PermissionMode::Default => {
                        "ask before each tool call"
                    }
                    cade_core::permissions::PermissionMode::AcceptEdits => {
                        "file edits auto-approved; Bash still prompts"
                    }
                    cade_core::permissions::PermissionMode::Plan => {
                        "read-only; write operations blocked"
                    }
                    cade_core::permissions::PermissionMode::BypassPermissions => {
                        "all tools auto-approved (deny rules still apply)"
                    }
                };
                self.tui_blank();
                self.tui_hdr(format!("  Mode: {icon} {label}  —  {mode_hint}"));
                self.tui_blank();

                if allow.is_empty() && deny.is_empty() {
                    self.tui_dim("  No allow/deny rules active.");
                } else {
                    if !allow.is_empty() {
                        self.tui_ok(format!("  Allow rules ({}):", allow.len()));
                        for r in &allow {
                            self.tui_dim(format!(
                                "    {:<12} {}",
                                r.tool(),
                                r.arg_display()
                            ));
                        }
                        let _ = self
                            .app
                            .lock()
                            .push(RenderLine::Blank);
                    }
                    if !deny.is_empty() {
                        self.tui_err(format!("  Deny rules ({}):", deny.len()));
                        for r in &deny {
                            self.tui_dim(format!(
                                "    {:<12} {}",
                                r.tool(),
                                r.arg_display()
                            ));
                        }
                        self.tui_blank();
                    }
                }
                self.tui_dim("  /approve-always <pattern>    /deny-always <pattern>");
                self.tui_dim(
                    "  Pattern:  Bash(cargo test)  ·  Read(src/**)  ·  Bash(rm -rf:*)",
                );
            }

            SlashCmd::ApproveAlways(pattern) => {
                if pattern.is_empty() {
                    self.tui_dim("  /approve-always <pattern>");
                    self.tui_dim("  Examples:  Bash(cargo test)  Read(src/**)  Bash(git commit:*)  Bash");
                } else if let Some(rule) =
                    cade_core::permissions::PermissionRule::parse(&pattern)
                {
                    self.permissions.add_allow_rule(rule.clone());
                    self.tui_ok(format!(
                        "  ✓ Allow  {:<12} {}",
                        rule.tool(),
                        rule.arg_display()
                    ));
                    use crate::ui::question::{Question, QuestionOption};
                    let opts = vec![
                        QuestionOption {
                            label: "Yes — save to settings.json".to_string(),
                            description: String::new(),
                        },
                        QuestionOption {
                            label: "No — session only".to_string(),
                            description: String::new(),
                        },
                    ];
                    let q = Question {
                        header: "Save rule?".to_string(),
                        text: "Persist this rule to settings.json?".to_string(),
                        options: opts.clone(),
                        multi_select: false,
                        allow_other: false,
                        progress: None,
                    };
                    let save = {
                        let mut app = self.app.lock();
                        let r = app.ask_question(&q)?;
                        app.scroll = 0;
                        let _ = app.draw();
                        matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
                    };
                    if save {
                        let mut settings = self.settings.lock();
                        match settings.save_allow_rule(&pattern) {
                            Ok(_) => self.tui_ok("  ✓ Saved"),
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }
                } else {
                    self.tui_err(format!("invalid pattern: {pattern:?}  Expected: Tool  or  Tool(arg)  or  Tool(prefix:*)"));
                }
            }

            SlashCmd::DenyAlways(pattern) => {
                if pattern.is_empty() {
                    self.tui_dim("  /deny-always <pattern>");
                    self.tui_dim(
                        "  Examples:  Bash(rm -rf:*)  Bash(git push --force)  Bash",
                    );
                } else if let Some(rule) =
                    cade_core::permissions::PermissionRule::parse(&pattern)
                {
                    self.permissions.add_deny_rule(rule.clone());
                    self.tui_err(format!(
                        "  ✗ Deny   {:<12} {}",
                        rule.tool(),
                        rule.arg_display()
                    ));
                    use crate::ui::question::{Question, QuestionOption};
                    let opts = vec![
                        QuestionOption {
                            label: "Yes — save to settings.json".to_string(),
                            description: String::new(),
                        },
                        QuestionOption {
                            label: "No — session only".to_string(),
                            description: String::new(),
                        },
                    ];
                    let q = Question {
                        header: "Save rule?".to_string(),
                        text: "Persist this rule to settings.json?".to_string(),
                        options: opts.clone(),
                        multi_select: false,
                        allow_other: false,
                        progress: None,
                    };
                    let save = {
                        let mut app = self.app.lock();
                        let r = app.ask_question(&q)?;
                        app.scroll = 0;
                        let _ = app.draw();
                        matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
                    };
                    if save {
                        let mut settings = self.settings.lock();
                        match settings.save_deny_rule(&pattern) {
                            Ok(_) => self.tui_ok("  ✓ Saved"),
                            Err(e) => self.tui_err(e.to_string()),
                        }
                    }
                } else {
                    self.tui_err(format!("invalid pattern: {pattern:?}  Expected: Tool  or  Tool(arg)  or  Tool(prefix:*)"));
                }
            }

            SlashCmd::Hooks => {
                let merged = self.settings.lock().merged_hooks();
                self.tui_blank();
                if merged.is_empty() {
                    self.tui_dim("  No hooks configured.");
                    self.tui_dim(
                        "  Configure in ~/.cade/settings.json or .cade/settings.json",
                    );
                    self.tui_blank();
                    self.tui_dim("  Example: { \"hooks\": { \"PreToolUse\": [{ \"matcher\": \"Bash\", \"hooks\": [{ \"type\": \"command\", \"command\": \"./validate.sh\" }] }] } }");
                    self.tui_dim(
                        "  Exit codes:  0=allow  1=log+continue  2=block (stderr→agent)",
                    );
                } else {
                    self.tui_hdr("  Hooks");
                    self.tui_blank();
                    let show_section = |name: &str, entries: &[cade_core::settings::manager::HookEntry]| {
                        if !entries.is_empty() {
                            self.tui_hdr(format!("  {name}  ({}):", entries.len()));
                            for entry in entries {
                                let m = entry.matcher.as_deref().unwrap_or("*");
                                self.tui_dim(format!("    matcher: {m}"));
                                for hook in &entry.hooks {
                                    self.tui_dim(format!("      {hook}"));
                                }
                            }
                            self.tui_blank();
                        }
                    };
                    show_section("PreToolUse", &merged.pre_tool_use);
                    show_section("PostToolUse", &merged.post_tool_use);
                    show_section("PostToolUseFailure", &merged.post_tool_use_failure);
                    show_section("PermissionRequest", &merged.permission_request);
                    show_section("UserPromptSubmit", &merged.user_prompt_submit);
                    show_section("Stop", &merged.stop);
                    show_section("SubagentStop", &merged.subagent_stop);
                    show_section("SessionStart", &merged.session_start);
                    show_section("SessionEnd", &merged.session_end);
                    show_section("Notification", &merged.notification);
                    self.tui_dim("  Config: ~/.cade/settings.json  ·  .cade/settings.json  ·  .cade/settings.local.json");
                }
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
