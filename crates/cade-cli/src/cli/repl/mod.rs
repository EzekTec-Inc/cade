pub mod capability_gate;
pub mod commands;
pub mod commands_memory;
pub mod commands_providers;
pub mod commands_skills;
pub mod format;
pub mod pickers;
pub mod tool_intercepts;
pub mod turn_loop;
pub mod ui_push;

use crate::Result;
use serde_json::json;
use std::io;

use std::sync::Arc;
use parking_lot::Mutex;

use crate::ui::{RenderLine, TuiApp, cycle_mode, cycle_mode_back};
use cade_agent::agent::session::SessionStore;
use cade_agent::agent::{HttpTransport, client::AgentState};
use cade_agent::subagents::BackgroundResult;
use cade_core::permissions::{PermissionManager, PermissionMode};
use cade_core::settings::SettingsManager;
use cade_core::skills::Skill;
use cade_core::toolsets::Toolset;

const BANNER: &str = r#"
   ___    _    ____  _____
  / __|  / \  |  _ \| ____|
 | |    / _ \ | | | |  _|
 | |_  / ___ \| |_| | |___
  \__|/_/   \_|____/|_____|

 Coding AI assistant with Desktop Extensions
 Type /help for commands, /exit to quit
"#;

/// Injected as a follow-up user message when the LLM produces an empty response
/// after a tool execution (no text, no new tool call).  Prevents silent turn ends.
pub(crate) const EMPTY_YIELD_REPROMPT: &str = "Tool execution complete. \
Please provide a text response explaining the result, what you found, \
or what you are doing next.";

// -- Slash commands

/// Result from the agent TUI picker.
pub(crate) enum AgentPickerResult {
    Switch(AgentState),
    DeleteMany(Vec<AgentState>),
    Rename { agent: AgentState, new_name: String },
}

#[derive(Debug)]
pub(crate) enum MemoryPickerResult {
    Edit(cade_agent::agent::client::MemoryBlock),
    TogglePin(cade_agent::agent::client::MemoryBlock),
    Delete(cade_agent::agent::client::MemoryBlock),
}

pub(crate) enum SubagentPickerResult {
    Run(String),
    Edit(std::path::PathBuf),
}

pub mod slash;
pub mod stats;
pub(crate) use slash::*;
pub(crate) use stats::*;

// -- Session Statistics

// -- Session footer helpers

pub(crate) fn fmt_tok_short(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        // Use whole thousands to match compact footer style (e.g. 13k, 248k)
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub(crate) fn fmt_window_tokens_short(n: u32) -> String {
    if n == 0 {
        "?".to_string()
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub(crate) fn short_mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Default => "auto",
        PermissionMode::AcceptEdits => "edits",
        PermissionMode::Plan => "plan",
        PermissionMode::BypassPermissions => "yolo",
    }
}

// -- Tool preflight result

#[derive(Debug)]
pub(crate) enum ToolPreflightResult {
    Approved,
    Blocked(cade_agent::tools::ToolResult),
}

// -- Repl

use crate::cli::repl::format::mode_display;

pub struct Repl {
    pub(crate) client: HttpTransport,
    /// Shared-mutable so /new and /agents can hot-swap the agent mid-session
    pub(crate) agent_id: Arc<Mutex<String>>,
    pub(crate) agent_name: Arc<Mutex<String>>,
    pub(crate) permissions: PermissionManager,
    pub(crate) current_model: Arc<Mutex<String>>,
    pub(crate) reasoning_effort: Arc<Mutex<Option<String>>>,
    pub(crate) settings: Arc<Mutex<SettingsManager>>,
    pub(crate) session: Arc<Mutex<SessionStore>>,
    /// Working directory (for /init context)
    pub(crate) cwd: std::path::PathBuf,
    /// Currently loaded skills
    pub(crate) skills: Arc<Mutex<Vec<Skill>>>,
    /// Loaded prompt templates (for /template_name expansion)
    pub(crate) prompts: Vec<cade_core::resources::PromptTemplate>,
    /// Active execution backend (local / docker / ssh / readonly).
    pub(crate) exec_backend: std::sync::Arc<dyn cade_agent::backends::ExecutionBackend>,
    /// Directory from which skills are discovered
    pub(crate) skills_dir: std::path::PathBuf,
    /// Completed background subagent results waiting to be shown
    pub(crate) background_results: Arc<Mutex<Vec<BackgroundResult>>>,
    /// Active toolset — switches with /model
    pub(crate) current_toolset: Arc<Mutex<Toolset>>,
    /// Hook engine — fires user-defined scripts at lifecycle events
    pub(crate) hooks: cade_core::hooks::HookEngine,
    /// `true` until the first real user message is sent this session.
    /// Used to inject the environment context block (OS, cwd, git) on turn 1.
    pub(crate) first_turn: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Set to `true` by a SIGINT handler while a turn is running.
    /// `stream_turn()` checks this flag and aborts the SSE stream early.
    pub(crate) cancel_turn: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Set to `true` while an agent turn is actively executing.
    /// The application-lifetime SIGINT task checks this flag to determine
    /// whether a Ctrl+C should cancel the current turn.
    pub(crate) turn_active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Active conversation ID — None means the default (legacy) conversation.
    pub(crate) conversation_id: Arc<Mutex<Option<String>>>,
    /// MCP server manager — routes tool calls with `{server}__` prefix.
    pub(crate) mcp: std::sync::Arc<cade_agent::mcp::McpManager>,
    /// Active capability set — controls which tools and commands are available.
    pub(crate) capabilities: cade_core::capabilities::CapabilitySet,
    /// Semaphore limiting concurrent subagent LLM calls.
    /// Capacity is read from CADE_MAX_SUBAGENTS at startup (default: 4).
    pub(crate) subagent_semaphore: std::sync::Arc<tokio::sync::Semaphore>,
    /// Receives a signal whenever a SKILL.MD file changes on disk.
    /// The REPL polls this each loop iteration and triggers a reload.
    pub(crate) skill_reload_rx: tokio::sync::mpsc::Receiver<()>,
    /// Receives a signal whenever a CADE settings file changes on disk.
    /// The REPL polls this each loop iteration and triggers an MCP reload.
    pub(crate) mcp_reload_rx: tokio::sync::mpsc::Receiver<()>,
    /// Whether SSE token streaming is enabled (toggled by /stream).
    pub(crate) streaming_enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Cumulative token usage for the session (input, output).
    pub(crate) session_input_tokens: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub(crate) session_output_tokens: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Rich session statistics (per-model token breakdown, tool calls, timing).
    pub(crate) session_stats: std::sync::Arc<parking_lot::Mutex<SessionStats>>,
    /// Fullscreen ratatui TUI — single render path for all output + input.
    pub(crate) app: Arc<Mutex<TuiApp>>,
    /// I-01: steering message typed during a turn (Enter key) — cancel current
    /// turn and run this message as the very next turn.
    pub(crate) queued_steering: Arc<Mutex<Option<String>>>,
    /// I-01: follow-up messages typed during a turn (Enter / Alt+Enter) — run
    /// in submission order after the current turn completes, without interrupting.
    /// VecDeque allows multiple messages to be queued while the agent is busy.
    pub(crate) queued_followup: Arc<Mutex<std::collections::VecDeque<String>>>,
    /// Buffered reasoning text from the most recent turn (for hook payloads).
    pub(crate) last_reasoning: Arc<Mutex<String>>,
    /// Buffered assistant text from the most recent turn (for hook payloads).
    pub(crate) last_assistant_text: Arc<Mutex<String>>,
    /// Millisecond timestamp of the last time a blocking question modal closed
    /// (`blocking_question_active` transitioned true → false).
    /// The I-01 Enter handler ignores Enter events within 300 ms of a modal
    /// close to prevent the confirmation Enter from cancelling the subsequent
    /// stream_turn — mirrors the 200 ms Esc grace period.
    pub(crate) last_modal_close_ms: Arc<std::sync::atomic::AtomicU64>,
    /// Images staged by `agent_turn_with_images` for the current turn.
    /// Consumed (and cleared) by the first `send_message*` call inside `agent_turn`.
    pub(crate) pending_turn_images: Vec<serde_json::Value>,
    /// Cumulative count of file-write / edit / bash tool calls this session.
    /// Used to trigger the one-time `working_set` reminder (C3).
    pub(crate) write_tool_calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Set to `true` once the working_set reminder has been injected so it
    /// fires at most once per session.
    pub(crate) working_set_notified: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// `true` if an auto-checkpoint has been taken for the current turn.
    pub(crate) turn_checkpoint_taken: bool,
}

impl Repl {
    pub fn new(
        client: HttpTransport,
        agent_id: String,
        agent_name: String,
        permissions: PermissionManager,
        current_model: String,
        reasoning_effort: Option<String>,
        settings: Arc<Mutex<SettingsManager>>,
        session: Arc<Mutex<SessionStore>>,
        cwd: std::path::PathBuf,
        skills: Vec<Skill>,
        skills_dir: std::path::PathBuf,
        toolset: Toolset,
        hooks: cade_core::hooks::HookEngine,
        conversation_id: Option<String>,
        mcp: std::sync::Arc<cade_agent::mcp::McpManager>,
        theme: cade_tui::ThemeColors,
        exec_backend: std::sync::Arc<dyn cade_agent::backends::ExecutionBackend>,
        capabilities: cade_core::capabilities::CapabilitySet,
    ) -> Self {
        let perm_mode = permissions.mode();
        let agent_name_clone = agent_name.clone();
        let current_model_clone = current_model.clone();
        let cap = std::env::var("CADE_MAX_SUBAGENTS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(4);
        tracing::info!("Subagent concurrency cap: {cap} (set CADE_MAX_SUBAGENTS to override)");
        let skill_reload_rx = cade_core::skills::spawn_skill_watcher(&cwd);
        let mcp_reload_rx = cade_agent::mcp::watcher::spawn_mcp_watcher(&cwd);
        Self {
            client,
            agent_id: Arc::new(Mutex::new(agent_id)),
            agent_name: Arc::new(Mutex::new(agent_name)),
            permissions,
            current_model: Arc::new(Mutex::new(current_model)),
            reasoning_effort: Arc::new(Mutex::new(reasoning_effort.clone())),
            settings,
            session,
            prompts: {
                let agent_dir = dirs::home_dir()
                    .map(|h| h.join(".cade"))
                    .unwrap_or_default();
                cade_core::resources::discover_prompts(&cwd, &agent_dir)
            },
            exec_backend,
            cwd,
            skills: Arc::new(Mutex::new(skills)),
            skills_dir,
            background_results: Arc::new(Mutex::new(vec![])),
            current_toolset: Arc::new(Mutex::new(toolset)),
            hooks,
            first_turn: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            cancel_turn: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            turn_active: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            conversation_id: Arc::new(Mutex::new(conversation_id)),
            mcp,
            subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(cap)),
            skill_reload_rx,
            mcp_reload_rx,
            streaming_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            session_input_tokens: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            session_output_tokens: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            session_stats: std::sync::Arc::new(parking_lot::Mutex::new(SessionStats::new())),
            app: Arc::new(Mutex::new(TuiApp::new_with_theme(
                perm_mode,
                agent_name_clone.clone(),
                current_model_clone.clone(),
                reasoning_effort.clone(),
                theme,
            ))),
            queued_steering: Arc::new(Mutex::new(None)),
            queued_followup: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            last_reasoning: Arc::new(Mutex::new(String::new())),
            last_assistant_text: Arc::new(Mutex::new(String::new())),
            last_modal_close_ms: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            pending_turn_images: Vec::new(),
            write_tool_calls: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
            working_set_notified: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            turn_checkpoint_taken: false,
            capabilities,
        }
    }

    fn agent_id(&self) -> String {
        self.agent_id.lock().clone()
    }
    fn agent_name(&self) -> String {
        self.agent_name.lock().clone()
    }
    fn model(&self) -> String {
        self.current_model.lock().clone()
    }
    fn conversation_id(&self) -> Option<String> {
        self.conversation_id.lock().clone()
    }

    /// Reload MCP servers, hooks, and permissions from current settings.
    /// Called from the tick-loop watcher poll and from `/mcp reload`.
    async fn do_settings_reload(&mut self) {
        self.tui_dim(
            "  ↺ Settings changed — reloading MCP servers, hooks, and permissions…".to_string(),
        );

        // 1. Reload raw settings from disk
        let _ = self.settings.lock().reload();

        // 2. Extract merged config slices
        let (new_mcp, new_hooks, new_perms) = {
            let guard = self.settings.lock();
            (
                guard.merged_mcp_servers(),
                guard.merged_hooks(),
                guard.permission_settings().clone(),
            )
        };

        // 3. Apply new hooks and permissions
        self.hooks = cade_core::hooks::HookEngine::new(new_hooks, self.cwd.clone());
        self.permissions.reload_from_settings(&new_perms);

        // 4. Reload MCP servers
        let summary = self.mcp.reload(&new_mcp).await;

        if !summary.stopped.is_empty() {
            self.tui_dim(format!("  stopped: {}", summary.stopped.join(", ")));
        }
        if !summary.failed.is_empty() {
            self.tui_err(format!("  failed to start: {}", summary.failed.join(", ")));
        }

        let changed = !summary.started.is_empty() || !summary.stopped.is_empty();
        if changed {
            self.spawn_tool_reregister();
        }

        let msg = format!(
            "  ↺ Settings reloaded — {} MCP started, {} stopped, {} kept{}",
            summary.started.len(),
            summary.stopped.len(),
            summary.kept.len(),
            if summary.failed.is_empty() {
                String::new()
            } else {
                format!(", {} failed", summary.failed.len())
            }
        );
        self.tui_ok(msg);
    }

    /// Spawn a background task that re-registers all tools (native + MCP) and
    /// re-attaches them to the agent. Called after toolset/model switches and
    /// MCP config reloads so the agent always sees an up-to-date tool list.
    fn spawn_tool_reregister(&self) {
        let agent_id = self.agent_id();
        let client = self.client.clone();
        let mcp_arc = std::sync::Arc::clone(&self.mcp);
        let toolset = *self.current_toolset.lock();
        let allow_agent_mode = self
            .settings
            .lock()
            .permission_settings()
            .allow_agent_mode_changes;
        tokio::spawn(async move {
            use cade_agent::agent::tools::{register_cade_tools, register_mcp_tools};
            let tools = register_cade_tools(&client, toolset, allow_agent_mode)
                .await
                .unwrap_or_default();
            let ids: Vec<String> = tools.into_iter().map(|t| t.id).collect();
            if !ids.is_empty() {
                let _ = client.attach_agent_tools(&agent_id, &ids).await;
            }
            let mcp_ids: Vec<String> =
                register_mcp_tools(&client, mcp_arc.all_tool_schemas().await)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|t| t.id)
                    .collect();
            if !mcp_ids.is_empty() {
                let _ = client.attach_agent_tools(&agent_id, &mcp_ids).await;
            }
        });
    }

    /// Called when `--continue` is set — suppress first-turn env injection.
    pub fn mark_continued(&self) {
        use std::sync::atomic::Ordering;
        self.first_turn.store(false, Ordering::SeqCst);
    }

    pub async fn run(mut self) -> Result<()> {
        let mut stdout = io::stdout();

        // Spawn exactly ONE application-lifetime SIGINT watcher.
        // On every Ctrl+C press it:
        //   1. Checks `turn_active`.
        //   2. If true, sets `cancel_turn`  — aborts any active SSE stream.
        // This replaces the per-turn tokio::signal registrations that previously
        // leaked kernel signal interests and left no active OS handler once the
        // turn ended, causing the process to freeze unrecoverably on Ctrl+C.
        {
            let cancel = self.cancel_turn.clone();
            let turn_active = self.turn_active.clone();
            tokio::spawn(async move {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{SignalKind, signal};
                    // Loop so every Ctrl+C press is handled, not just the first.
                    if let Ok(mut sig) = signal(SignalKind::interrupt()) {
                        loop {
                            sig.recv().await;
                            use std::sync::atomic::Ordering;
                            if turn_active.load(Ordering::SeqCst) {
                                cancel.store(true, Ordering::SeqCst);
                            }
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    // Windows: use tokio's ctrl_c future.
                    loop {
                        let _ = tokio::signal::ctrl_c().await;
                        use std::sync::atomic::Ordering;
                        if turn_active.load(Ordering::SeqCst) {
                            cancel.store(true, Ordering::SeqCst);
                        }
                    }
                }
            });
        }

        // Push banner + agent info into TuiApp content.
        {
            let mut app = self.app.lock();
            let agent_id = self.agent_id.lock().clone();
            let agent_name = self.agent_name.lock().clone();
            let model = self.current_model.lock().clone();
            let mode_str = format!("{}", self.permissions.mode());
            let banner_text = format!(
                "{BANNER}\n  Agent  : {agent_name}  ({agent_id})\n  Model  : {model}\n  Mode   : {mode_str}"
            );
            app.push_silent(RenderLine::SystemMsg(banner_text));
            app.draw()?;
        }

        // SessionStart hook (non-blocking)
        self.hooks.session_start(&self.agent_id()).await;

        let mut history: Vec<String> = Vec::new();
        let mut hist_idx: Option<usize> = None;

        let mut pending_input: Option<String> = None;
        loop {
            // Check for completed background subagent results
            {
                let mut results = self.background_results.lock();
                for r in results.drain(..) {
                    let msg = format!("  ✓ Subagent '{}' finished:\n{}", r.subagent, r.result);
                    let _ = self
                        .app
                        .lock()
                        .push(RenderLine::SystemMsg(msg));
                    let notify = format!(
                        "[Background subagent '{}' completed (task ID: {})]:\n{}",
                        r.subagent, r.task_id, r.result
                    );
                    let _ = self
                        .client
                        .send_message(&self.agent_id(), &notify, false)
                        .await;
                }
            }

            // Check if MCP schemas changed after a reconnect — re-register if so
            if self
                .mcp
                .schemas_dirty
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                self.tui_dim(
                    "  ↺ MCP tool schemas changed after reconnect — re-registering…".to_string(),
                );
                self.spawn_tool_reregister();
            }

            // Check for settings file changes — reload MCP servers if signalled
            let mut mcp_changed = false;
            while self.mcp_reload_rx.try_recv().is_ok() {
                mcp_changed = true;
            }
            if mcp_changed {
                self.do_settings_reload().await;
            }

            // Check for skill file changes (live watcher) — reload if signalled
            while self.skill_reload_rx.try_recv().is_ok() {
                let new_skills = cade_core::skills::discover_all_skills(&self.cwd, None, None);
                let new_count = new_skills.len();
                *self.skills.lock() = new_skills.clone();
                let names: Vec<String> = new_skills.iter().map(|s| s.name.clone()).collect();
                let list = names.join(", ");
                self.tui_ok(format!(
                    "  ↺ Skills auto-reloaded ({new_count} skills): {list}"
                ));
                tracing::info!("Skills auto-reloaded: {new_count} skills");
            }

            // Update app footer to reflect current mode/model before reading input.
            {
                let mut app = self.app.lock();
                app.update_mode(self.permissions.mode());
                app.update_model(self.current_model.lock().clone());
                app.update_agent_name(self.agent_name());
            }

            // Read input — either from pending (menu dispatch) or from the user.
            let input = if let Some(cmd) = pending_input.take() {
                cmd
            } else {
                match self
                    .app
                    .lock()
                    .read_input(&mut history, &mut hist_idx)?
                {
                    Some(s) => s,
                    None => break,
                }
            };
            let input = input.trim().to_string();

            // Clear status immediately upon submit
            self.app.lock().set_last_status(None);

            // Handle Tab / BackTab mode-cycle sentinels.
            if input == "__TAB__" {
                let next = cycle_mode(self.permissions.mode());
                self.permissions.set_mode(next);
                self.app.lock().update_mode(next);
                continue;
            }
            if input == "__BACKTAB__" {
                let prev = cycle_mode_back(self.permissions.mode());
                self.permissions.set_mode(prev);
                self.app.lock().update_mode(prev);
                continue;
            }

            // Drain any pasted images staged by the TUI on the last submission.
            let submit_images: Vec<serde_json::Value> = {
                let mut app = self.app.lock();
                std::mem::take(&mut app.pending_submit_images)
                    .into_iter()
                    .map(|img| {
                        json!({
                            "media_type": img.media_type,
                            "data": img.data
                        })
                    })
                    .collect()
            };

            if input.is_empty() && submit_images.is_empty() {
                continue;
            }
            if !input.is_empty() {
                history.push(input.clone());
            }
            hist_idx = None;

            // Echo user message.
            let echo_text = if submit_images.is_empty() {
                input.clone()
            } else {
                let count = submit_images.len();
                let suffix = if count == 1 { "image" } else { "images" };
                if input.is_empty() {
                    format!("[Attached {} {}]", count, suffix)
                } else {
                    format!(
                        "{}

[Attached {} {}]",
                        input, count, suffix
                    )
                }
            };
            let _ = self
                .app
                .lock()
                .push(RenderLine::UserMessage(echo_text));

            // Direct bash:
            //   !!cmd  — run silently: show output locally, do NOT send to agent.
            //   !cmd   — run and send: show output AND forward it to the agent as context.
            if input.starts_with('!') {
                let (silent, cmd_str) = if let Some(rest) = input.strip_prefix("!!") {
                    (true, rest.trim())
                } else {
                    (false, input.strip_prefix('!').unwrap_or("").trim())
                };
                if !cmd_str.is_empty() {
                    let mut cmd = tokio::process::Command::new("sh");
                    cade_core::agent_env::apply_agent_env(&mut cmd);
                    let run = cmd.arg("-c").arg(cmd_str).output().await;
                    match run {
                        Ok(out) => {
                            let text = if out.stdout.is_empty() {
                                String::from_utf8_lossy(&out.stderr).to_string()
                            } else {
                                String::from_utf8_lossy(&out.stdout).to_string()
                            };
                            let _ = self
                                .app
                                .lock()
                                .push(RenderLine::SystemMsg(text.clone()));
                            if !silent {
                                // Send command + output to agent
                                let agent_msg =
                                    format!("Command: `{cmd_str}`\n\nOutput:\n```\n{text}\n```");
                                self.agent_turn(&mut stdout, &agent_msg).await?;
                                let _ = self.app.lock().commit_streaming();
                            }
                        }
                        Err(e) => {
                            let _ = self
                                .app
                                .lock()
                                .push(RenderLine::ErrorMsg(format!("bash: {e}")));
                        }
                    }
                }
                continue;
            }

            // Prompt template expansion: /template_name [args...]
            // Check before slash command dispatch so templates can be invoked naturally.
            let input = if let Some(stripped) = input.strip_prefix('/') {
                let parts: Vec<&str> = stripped.splitn(2, ' ').collect();
                let name = parts[0];
                let args_str = parts.get(1).copied().unwrap_or("");
                if let Some(tmpl) = self.prompts.iter().find(|t| t.name == name) {
                    let expanded = cade_core::resources::expand_template(&tmpl.content, args_str);
                    self.tui_dim(format!(
                        "  Expanded /{name} template ({} chars)",
                        expanded.len()
                    ));
                    expanded
                } else {
                    input
                }
            } else {
                input
            };

            // Slash commands (include loaded skill ids so /commit etc. work)
            let skill_ids: Vec<String> = self
                .skills
                .lock()
                .iter()
                .map(|s| s.id.clone())
                .collect();
            if let Some(cmd) = parse_slash_with_skills(&input, &skill_ids) {
                if self.handle_slash_command(cmd, &input, &mut stdout, &mut pending_input).await? {
                    break;
                }
                continue;
            }

            // UserPromptSubmit hook — can block the turn
            if let cade_core::hooks::HookOutcome::Block { reason } =
                self.hooks.user_prompt_submit(&input).await
            {
                self.tui_sys(format!("  ⚠ Hook blocked prompt: {reason}"));
                continue;
            }

            // Send to agent and handle tool loop
            self.agent_turn_with_images(&mut stdout, &input, submit_images)
                .await?;
            let _ = self.app.lock().commit_streaming();

            // I-01: drain queued messages into pending_input.
            // Follow-up runs after the turn completes naturally.
            // Steering runs after a cancelled turn.
            // Follow-up takes priority — if both are set (edge case), run
            // follow-up first; steering is re-queued on the next iteration.
            let queued_msg = {
                let mut q = self.queued_followup.lock();
                q.pop_front().map(|msg| (msg, q.len()))
            };

            if let Some((follow, count)) = queued_msg {
                self.app.lock().queued_count = count;
                pending_input = Some(follow);
            } else if let Some(steer) = self.queued_steering.lock().take() {
                pending_input = Some(steer);
            }
        }

        // SessionEnd hook (non-blocking)
        self.hooks.session_end(&self.agent_id()).await;

        Ok(())
    }
}
