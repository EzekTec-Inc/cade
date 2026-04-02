pub mod capability_gate;
pub mod commands_providers;
pub mod format;
pub mod pickers;
pub mod tool_intercepts;
pub mod turn_loop;
pub mod ui_push;

use crate::Result;
use serde_json::json;
use std::io;

use std::sync::{Arc, Mutex};

use crate::ui::{RenderLine, ToastLevel, TuiApp, cycle_mode, cycle_mode_back};
use cade_agent::agent::session::SessionStore;
use cade_agent::agent::{CadeClient, client::AgentState};
use cade_agent::subagents::{BackgroundResult, discover_all_subagents};
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
    pub(crate) client: CadeClient,
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
    /// Set to `true` when the user presses Ctrl+C while no turn is running,
    /// signalling a clean exit from the REPL loop.  A single application-
    /// lifetime SIGINT task writes this flag instead of spawning a new
    /// listener each turn (which leaked signal registrations).
    pub(crate) shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
    pub(crate) session_stats: std::sync::Arc<std::sync::Mutex<SessionStats>>,
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
        client: CadeClient,
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
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            conversation_id: Arc::new(Mutex::new(conversation_id)),
            mcp,
            subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(cap)),
            skill_reload_rx,
            mcp_reload_rx,
            streaming_enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            session_input_tokens: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            session_output_tokens: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            session_stats: std::sync::Arc::new(std::sync::Mutex::new(SessionStats::new())),
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
        self.agent_id.lock().expect("lock poisoned").clone()
    }
    fn agent_name(&self) -> String {
        self.agent_name.lock().expect("lock poisoned").clone()
    }
    fn model(&self) -> String {
        self.current_model.lock().expect("lock poisoned").clone()
    }
    fn conversation_id(&self) -> Option<String> {
        self.conversation_id.lock().expect("lock poisoned").clone()
    }

    /// Reload MCP servers, hooks, and permissions from current settings.
    /// Called from the tick-loop watcher poll and from `/mcp reload`.
    async fn do_settings_reload(&mut self) {
        self.tui_dim(
            "  ↺ Settings changed — reloading MCP servers, hooks, and permissions…".to_string(),
        );

        // 1. Reload raw settings from disk
        let _ = self.settings.lock().expect("lock poisoned").reload();

        // 2. Extract merged config slices
        let (new_mcp, new_hooks, new_perms) = {
            let guard = self.settings.lock().expect("lock poisoned");
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
        let toolset = *self.current_toolset.lock().expect("lock poisoned");
        tokio::spawn(async move {
            use cade_agent::agent::tools::{register_cade_tools, register_mcp_tools};
            let tools = register_cade_tools(&client, toolset)
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
        //   1. Sets `cancel_turn`  — aborts any active SSE stream.
        //   2. Sets `shutdown_flag` — signals the idle REPL loop to exit cleanly.
        // This replaces the per-turn tokio::signal registrations that previously
        // leaked kernel signal interests and left no active OS handler once the
        // turn ended, causing the process to freeze unrecoverably on Ctrl+C.
        {
            let cancel = self.cancel_turn.clone();
            let shutdown = self.shutdown_flag.clone();
            tokio::spawn(async move {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{SignalKind, signal};
                    // Loop so every Ctrl+C press is handled, not just the first.
                    if let Ok(mut sig) = signal(SignalKind::interrupt()) {
                        loop {
                            sig.recv().await;
                            use std::sync::atomic::Ordering;
                            cancel.store(true, Ordering::SeqCst);
                            shutdown.store(true, Ordering::SeqCst);
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    // Windows: use tokio's ctrl_c future.
                    loop {
                        let _ = tokio::signal::ctrl_c().await;
                        use std::sync::atomic::Ordering;
                        cancel.store(true, Ordering::SeqCst);
                        shutdown.store(true, Ordering::SeqCst);
                    }
                }
            });
        }

        // Push banner + agent info into TuiApp content.
        {
            let mut app = self.app.lock().expect("lock poisoned");
            let agent_id = self.agent_id.lock().expect("lock poisoned").clone();
            let agent_name = self.agent_name.lock().expect("lock poisoned").clone();
            let model = self.current_model.lock().expect("lock poisoned").clone();
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
                let mut results = self.background_results.lock().expect("lock poisoned");
                for r in results.drain(..) {
                    let msg = format!("  ✓ Subagent '{}' finished:\n{}", r.subagent, r.result);
                    let _ = self
                        .app
                        .lock()
                        .expect("lock poisoned")
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
                *self.skills.lock().expect("lock poisoned") = new_skills.clone();
                let names: Vec<String> = new_skills.iter().map(|s| s.name.clone()).collect();
                let list = names.join(", ");
                self.tui_ok(format!(
                    "  ↺ Skills auto-reloaded ({new_count} skills): {list}"
                ));
                tracing::info!("Skills auto-reloaded: {new_count} skills");
            }

            // Update app footer to reflect current mode/model before reading input.
            {
                let mut app = self.app.lock().expect("lock poisoned");
                app.update_mode(self.permissions.mode());
                app.update_model(self.current_model.lock().expect("lock poisoned").clone());
                app.update_agent_name(self.agent_name());
            }

            // Check if the application-lifetime SIGINT handler fired while we
            // were idle (no turn was running).  Break to exit cleanly so the
            // TuiApp Drop impl restores the terminal.
            if self.shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }

            // Read input — either from pending (menu dispatch) or from the user.
            let input = if let Some(cmd) = pending_input.take() {
                cmd
            } else {
                match self
                    .app
                    .lock()
                    .expect("lock poisoned")
                    .read_input(&mut history, &mut hist_idx)?
                {
                    Some(s) => s,
                    None => break,
                }
            };
            let input = input.trim().to_string();

            // Handle Tab / BackTab mode-cycle sentinels.
            if input == "__TAB__" {
                let next = cycle_mode(self.permissions.mode());
                self.permissions.set_mode(next);
                self.app.lock().expect("lock poisoned").update_mode(next);
                continue;
            }
            if input == "__BACKTAB__" {
                let prev = cycle_mode_back(self.permissions.mode());
                self.permissions.set_mode(prev);
                self.app.lock().expect("lock poisoned").update_mode(prev);
                continue;
            }

            // Drain any pasted images staged by the TUI on the last submission.
            let submit_images: Vec<serde_json::Value> = {
                let mut app = self.app.lock().expect("lock poisoned");
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
                .expect("lock poisoned")
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
                                .expect("lock poisoned")
                                .push(RenderLine::SystemMsg(text.clone()));
                            if !silent {
                                // Send command + output to agent
                                let agent_msg =
                                    format!("Command: `{cmd_str}`\n\nOutput:\n```\n{text}\n```");
                                self.agent_turn(&mut stdout, &agent_msg).await?;
                                let _ = self.app.lock().expect("lock poisoned").commit_streaming();
                            }
                        }
                        Err(e) => {
                            let _ = self
                                .app
                                .lock()
                                .expect("lock poisoned")
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
                .expect("lock poisoned")
                .iter()
                .map(|s| s.id.clone())
                .collect();
            if let Some(cmd) = parse_slash_with_skills(&input, &skill_ids) {
                match cmd {
                    SlashCmd::Exit => {
                        use std::sync::atomic::Ordering;
                        let in_tok = self.session_input_tokens.load(Ordering::SeqCst);
                        let out_tok = self.session_output_tokens.load(Ordering::SeqCst);
                        if in_tok > 0 || out_tok > 0 {
                            let _ = self.app.lock().expect("lock poisoned").push(
                                RenderLine::SystemMsg(format!(
                                    "  Session tokens — in: {in_tok}  out: {out_tok}  total: {}",
                                    in_tok + out_tok
                                )),
                            );
                        }
                        let _ = self
                            .app
                            .lock()
                            .expect("lock poisoned")
                            .push(RenderLine::SystemMsg("Bye!".to_string()));
                        break;
                    }
                    // SlashCmd::Clear is handled below (with context clearing)
                    SlashCmd::RunSkill(skill_id) => {
                        // Find the skill, build a prompt that injects its content,
                        // and send it as an agent turn so the agent follows the skill.
                        let skill_body = self
                            .skills
                            .lock()
                            .expect("lock poisoned")
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
                        continue;
                    }
                    SlashCmd::Help => {
                        // Open full-screen command browser (filtered by capabilities)
                        let chosen = {
                            let mut app = self.app.lock().expect("lock poisoned");
                            let colors = app.colors.clone();
                            crate::ui::menu::show_command_menu_with_caps(
                                &mut app.terminal,
                                &colors,
                                Some(&self.capabilities),
                            )?
                        };
                        let _ = self.app.lock().expect("lock poisoned").draw();
                        if let Some(cmd) = chosen {
                            // If it's a tool hint (no slash) or a command that needs arguments,
                            // insert it into the editor instead of executing immediately.
                            let needs_args = !cmd.starts_with('/')
                                || (cmd.contains(' ') && !["/stats model", "/skills reload"].contains(&cmd.as_str()))
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
                                let mut app = self.app.lock().expect("lock poisoned");
                                app.editor.insert_str(&format!("{cmd} "));
                            } else {
                                pending_input = Some(cmd);
                            }
                        }
                        continue;
                    }
                    SlashCmd::Agent => {
                        let msg = format!("  Agent: {} ({})", self.agent_name(), self.agent_id());
                        let _ = self
                            .app
                            .lock()
                            .expect("lock poisoned")
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
                            .expect("lock poisoned")
                            .push(RenderLine::SystemMsg(msg));
                    }
                    SlashCmd::Yolo => {
                        self.permissions.set_mode(PermissionMode::BypassPermissions);
                        self.app
                            .lock()
                            .expect("lock poisoned")
                            .update_mode(PermissionMode::BypassPermissions);
                        let _ =
                            self.app
                                .lock()
                                .expect("lock poisoned")
                                .push(RenderLine::SystemMsg(
                                "⚡ Permission mode: bypassPermissions — all tools auto-approved"
                                    .to_string(),
                            ));
                    }
                    SlashCmd::Mcp => {
                        if self.require_capability(cade_core::capabilities::Capability::Mcp, "/mcp")
                        {
                            continue;
                        }
                        // Support "/mcp reload" subcommand
                        let sub = input.trim().strip_prefix("/mcp").unwrap_or("").trim();
                        if sub == "reload" {
                            self.do_settings_reload().await;
                            continue;
                        }

                        let statuses = self.mcp.status().await;
                        self.tui_blank();
                        self.tui_hdr("  MCP Servers");
                        self.tui_blank();
                        if statuses.is_empty() {
                            self.tui_dim("  No MCP servers configured.");
                            self.tui_blank();
                            self.tui_dim("  Add servers to ~/.cade/settings.json:");
                            self.tui_dim("  {");
                            self.tui_dim("    \"mcpServers\": {");
                            self.tui_dim(
                                "      \"git\": { \"command\": \"/path/to/git-mcp-server\" }",
                            );
                            self.tui_dim("    }");
                            self.tui_dim("  }");
                        } else {
                            let mut rows = Vec::new();
                            for s in &statuses {
                                let tool_list = s
                                    .tools
                                    .iter()
                                    .map(|t| t.split_once("__").map(|x| x.1).unwrap_or(t))
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                rows.push(vec![
                                    s.key.clone(),
                                    format!("{} tools", s.tools.len()),
                                    crate::ui::truncate_str(&tool_list, 60),
                                ]);
                            }
                            let _ =
                                self.app
                                    .lock()
                                    .expect("lock poisoned")
                                    .push(RenderLine::Table {
                                        headers: vec![
                                            "Server".to_string(),
                                            "Count".to_string(),
                                            "Tools".to_string(),
                                        ],
                                        rows,
                                    });
                        }
                    }
                    SlashCmd::Link => {
                        self.tui_dim("  Linking tools…");
                        let client2 = self.client.clone();
                        let mcp2 = std::sync::Arc::clone(&self.mcp);
                        let toolset2 = *self.current_toolset.lock().expect("lock poisoned");
                        let agent_id = self.agent_id();
                        use cade_agent::agent::tools::{register_cade_tools, register_mcp_tools};
                        let native_ids: Vec<String> = register_cade_tools(&client2, toolset2)
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
                            .expect("lock poisoned")
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
                        let model = self.current_model.lock().expect("lock poisoned").clone();
                        let window = cade_ai::catalogue::context_window_for_model(&model) as u64;
                        let pct_opt = self.app.lock().expect("lock poisoned").context_pct;
                        let agent_id = self.agent_id();
                        let conv_id = self.conversation_id();

                        // -- Per-category token estimates

                        // 1. Memory blocks
                        let mem_blocks =
                            self.client.get_memory(&agent_id).await.unwrap_or_default();
                        let mem_tok = (mem_blocks
                            .iter()
                            .map(|b| b.value.chars().count())
                            .sum::<usize>()
                            / 3) as u64;

                        // 2. Skills loaded in this session
                        let skills_tok = {
                            let skills = self.skills.lock().expect("lock poisoned");
                            (skills.iter().map(|s| s.body.chars().count()).sum::<usize>() / 3)
                                as u64
                        };

                        // 3. MCP tool schemas (schema JSON / 3 chars-per-token)
                        let mcp_schemas = self.mcp.all_tool_schemas().await;
                        let mcp_tok = (mcp_schemas
                            .iter()
                            .filter_map(|s| serde_json::to_string(s).ok())
                            .map(|s| s.len())
                            .sum::<usize>()
                            / 3) as u64;

                        // 4. Conversation messages
                        let msgs = self
                            .client
                            .get_conversation_messages(&agent_id, conv_id.as_deref().unwrap_or(""))
                            .await
                            .unwrap_or_default();
                        let msg_tok = (msgs
                            .iter()
                            .map(|m| m["content"].as_str().map(|s| s.len()).unwrap_or(0))
                            .sum::<usize>()
                            / 3) as u64;

                        // 5. System prompt
                        let sys_tok = self
                            .client
                            .get_agent(&agent_id)
                            .await
                            .ok()
                            .and_then(|a| a.system_prompt)
                            .map(|s| (s.chars().count() / 3) as u64)
                            .unwrap_or(0);

                        // 6. Native tool schemas (residual = server pct - known; 0 if pct unavailable)
                        let known = mem_tok + skills_tok + mcp_tok + msg_tok + sys_tok;
                        let tools_tok = pct_opt
                            .map(|p| (p as u64 * window / 100).saturating_sub(known))
                            .unwrap_or(0);
                        let total_used = known + tools_tok;

                        // 7. Buffer ≈ 3% of window (reserved for autocompact)
                        let buffer_tok = window * 3 / 100;
                        let free_tok = window.saturating_sub(total_used + buffer_tok);

                        // -- Grid construction (10 rows × 20 cells = 200 total)
                        let cells_for = |tok: u64| -> usize {
                            if window == 0 {
                                return 0;
                            }
                            ((tok as f64 / window as f64) * 200.0).round() as usize
                        };

                        let sys_c = cells_for(sys_tok);
                        let tool_c = cells_for(tools_tok);
                        let mcp_c = cells_for(mcp_tok);
                        let mem_c = cells_for(mem_tok);
                        let sk_c = cells_for(skills_tok);
                        let msg_c = cells_for(msg_tok);
                        let buf_c = cells_for(buffer_tok);
                        let used_c = sys_c + tool_c + mcp_c + mem_c + sk_c + msg_c;
                        let free_c = 200usize.saturating_sub(used_c + buf_c);

                        let mut flat: Vec<(char, u8)> = Vec::with_capacity(200);
                        for _ in 0..sys_c {
                            flat.push(('⛁', 0));
                        }
                        for _ in 0..tool_c {
                            flat.push(('⛁', 1));
                        }
                        for _ in 0..mcp_c {
                            flat.push(('⛁', 2));
                        }
                        for _ in 0..mem_c {
                            flat.push(('⛁', 3));
                        }
                        for _ in 0..sk_c {
                            flat.push(('⛁', 4));
                        }
                        for _ in 0..msg_c {
                            flat.push(('⛁', 5));
                        }
                        for _ in 0..free_c {
                            flat.push(('⛶', 6));
                        }
                        for _ in 0..buf_c {
                            flat.push(('⛝', 7));
                        }
                        while flat.len() < 200 {
                            flat.push(('⛶', 6));
                        }
                        flat.truncate(200);

                        let rows: Vec<Vec<(char, u8)>> =
                            flat.chunks(20).map(|c| c.to_vec()).collect();

                        // -- Right-side labels
                        let fmt = |n: u64| -> String {
                            if n >= 1_000_000 {
                                format!("{:.1}M", n as f64 / 1_000_000.0)
                            } else if n >= 1_000 {
                                format!("{:.1}k", n as f64 / 1_000.0)
                            } else {
                                n.to_string()
                            }
                        };
                        let pct_of = |n: u64| -> f64 {
                            if window == 0 {
                                0.0
                            } else {
                                100.0 * n as f64 / window as f64
                            }
                        };
                        let model_short = model.rsplit('/').next().unwrap_or(&model).to_string();
                        let pct_val = pct_opt.unwrap_or_else(|| {
                            if window == 0 {
                                0
                            } else {
                                (total_used * 100 / window).min(100) as u8
                            }
                        });

                        let right_labels: Vec<String> = vec![
                            format!(
                                "{}  ·  {}/{} tokens  ({}%)",
                                model_short,
                                fmt(total_used),
                                fmt(window),
                                pct_val
                            ),
                            String::new(),
                            "Estimated usage by category".to_string(),
                            format!(
                                "⛁ System prompt:  {}  ({:.1}%)",
                                fmt(sys_tok),
                                pct_of(sys_tok)
                            ),
                            format!(
                                "⛁ Tools:          {}  ({:.1}%)",
                                fmt(tools_tok),
                                pct_of(tools_tok)
                            ),
                            format!(
                                "⛁ MCP tools:      {}  ({:.1}%)",
                                fmt(mcp_tok),
                                pct_of(mcp_tok)
                            ),
                            format!(
                                "⛁ Memory:         {}  ({:.1}%)",
                                fmt(mem_tok),
                                pct_of(mem_tok)
                            ),
                            format!(
                                "⛁ Skills:         {}  ({:.1}%)",
                                fmt(skills_tok),
                                pct_of(skills_tok)
                            ),
                            format!(
                                "⛁ Messages:       {}  ({:.1}%)",
                                fmt(msg_tok),
                                pct_of(msg_tok)
                            ),
                            format!(
                                "⛶ Free:           {}  ({:.1}%)",
                                fmt(free_tok),
                                pct_of(free_tok)
                            ),
                        ];

                        // -- Emit grid rows
                        let mut app = self.app.lock().expect("lock poisoned");
                        let _ = app.push(RenderLine::Blank);
                        let _ = app.push(RenderLine::InfoHeader("  ◆ Context Usage".to_string()));
                        let _ = app.push(RenderLine::Blank);

                        if window == 0 {
                            let _ = app.push(RenderLine::DimMsg(
                                "  Context window size unknown for this model. Run a turn first."
                                    .to_string(),
                            ));
                        } else {
                            for (i, row) in rows.iter().enumerate() {
                                let label = right_labels.get(i).cloned().unwrap_or_default();
                                let label_color = if (3..=9).contains(&i) {
                                    Some((i - 3) as u8)
                                } else {
                                    None
                                };
                                let _ = app.push(RenderLine::ContextGridRow {
                                    cells: row.clone(),
                                    label,
                                    label_color,
                                });
                            }
                            // Buffer note (below grid)
                            if buf_c > 0 {
                                let _ = app.push(RenderLine::DimMsg(format!(
                                    "  {}⛝ Autocompact buffer:  {}  ({:.1}%)",
                                    " ".repeat(43),
                                    fmt(buffer_tok),
                                    pct_of(buffer_tok)
                                )));
                            }
                        }

                        // -- MCP Tools section
                        let _ = app.push(RenderLine::Blank);
                        let _ = app.push(RenderLine::InfoHeader(format!(
                            "  MCP Tools  ·  /mcp  (~{} tokens)",
                            fmt(mcp_tok)
                        )));
                        drop(app);

                        let mcp_statuses = self.mcp.status().await;
                        let loaded: Vec<_> = mcp_statuses.iter().filter(|s| !s.disabled).collect();
                        let disabled: Vec<_> = mcp_statuses.iter().filter(|s| s.disabled).collect();

                        let mut app = self.app.lock().expect("lock poisoned");
                        if loaded.is_empty() {
                            let _ = app.push(RenderLine::DimMsg(
                                "  (no MCP servers connected)".to_string(),
                            ));
                        } else {
                            let _ = app.push(RenderLine::DimMsg(format!(
                                "  Loaded  ({} server{})",
                                loaded.len(),
                                if loaded.len() == 1 { "" } else { "s" }
                            )));
                            for s in &loaded {
                                // Show first few tool names, truncate if long
                                let tool_preview: String = {
                                    let names: Vec<&str> = s
                                        .tools
                                        .iter()
                                        .map(|t| {
                                            t.rfind("__").map(|p| &t[p + 2..]).unwrap_or(t.as_str())
                                        })
                                        .collect();
                                    let preview = names
                                        .iter()
                                        .take(5)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    if names.len() > 5 {
                                        format!("{}  +{} more", preview, names.len() - 5)
                                    } else {
                                        preview
                                    }
                                };
                                let _ = app.push(RenderLine::DimMsg(format!(
                                    "  └ {}:  {}",
                                    s.key, tool_preview
                                )));
                            }
                        }
                        if !disabled.is_empty() {
                            let _ = app.push(RenderLine::DimMsg("  Disabled".to_string()));
                            for s in &disabled {
                                let _ = app.push(RenderLine::DimMsg(format!(
                                    "  └ {}  (reconnect failed)",
                                    s.key
                                )));
                            }
                        }

                        // -- Memory section
                        let _ = app.push(RenderLine::Blank);
                        let _ = app.push(RenderLine::InfoHeader(format!(
                            "  Memory  ·  /memory  (~{} tokens)",
                            fmt(mem_tok)
                        )));
                        if mem_blocks.is_empty() {
                            let _ =
                                app.push(RenderLine::DimMsg("  (no memory blocks)".to_string()));
                        } else {
                            for b in &mem_blocks {
                                let tok = (b.value.chars().count() / 3) as u64;
                                let desc = b.description.as_deref().unwrap_or("");
                                let suffix = if desc.is_empty() {
                                    String::new()
                                } else {
                                    format!("  —  {desc}")
                                };
                                let _ = app.push(RenderLine::DimMsg(format!(
                                    "  └ {}:  ~{} tokens{}",
                                    b.label,
                                    fmt(tok),
                                    suffix
                                )));
                            }
                        }

                        // -- Skills section
                        let _ = app.push(RenderLine::Blank);
                        let _ = app.push(RenderLine::InfoHeader(format!(
                            "  Skills  ·  /skills  (~{} tokens)",
                            fmt(skills_tok)
                        )));
                        {
                            let skills = self.skills.lock().expect("lock poisoned");
                            if skills.is_empty() {
                                let _ = app
                                    .push(RenderLine::DimMsg("  (no skills loaded)".to_string()));
                            } else {
                                for s in skills.iter() {
                                    let tok = (s.body.chars().count() / 3) as u64;
                                    let _ = app.push(RenderLine::DimMsg(format!(
                                        "  └ {}  —  {}  (~{} tokens)",
                                        s.id,
                                        s.description,
                                        fmt(tok)
                                    )));
                                }
                            }
                        }

                        let _ = app.push(RenderLine::Blank);
                        let _ = app.push(RenderLine::DimMsg(
                            "  /stats  session totals  ·  /stats model  per-model breakdown"
                                .to_string(),
                        ));
                        let _ = app.push(RenderLine::Blank);
                        drop(app);

                        // D2: Real server-side context accounting
                        let conv_id = self.conversation_id();
                        if let Ok(stats) = self
                            .client
                            .get_context_stats(&agent_id, conv_id.as_deref())
                            .await
                        {
                            let t_inc = stats["turns_included"].as_u64().unwrap_or(0);
                            let t_tot = stats["turns_total"].as_u64().unwrap_or(0);
                            let t_omit = stats["turns_omitted"].as_u64().unwrap_or(0);
                            let c_used = stats["chars_used"].as_u64().unwrap_or(0);
                            let c_bud = stats["message_budget_chars"].as_u64().unwrap_or(0);
                            let consol = stats["needs_consolidation"].as_bool().unwrap_or(false);
                            let pct_c = if c_bud > 0 {
                                format!("{:.0}%", 100.0 * c_used as f64 / c_bud as f64)
                            } else {
                                "?".to_string()
                            };

                            let mut app = self.app.lock().expect("lock poisoned");
                            let _ = app.push(RenderLine::InfoHeader(
                                "  ◆ Server Context Accounting (live)".to_string(),
                            ));
                            let _ = app.push(RenderLine::Blank);

                            let turns_line = if t_omit > 0 {
                                format!(
                                    "  Turns:   {t_inc} of {t_tot} included  \
                                     ({t_omit} omitted — use conversation_search to recover)"
                                )
                            } else {
                                format!("  Turns:   {t_inc} of {t_tot} included  (none omitted)")
                            };
                            let _ = app.push(RenderLine::DimMsg(turns_line));
                            let _ = app.push(RenderLine::DimMsg(format!(
                                "  History: {c_used} / {c_bud} chars used  ({pct_c})"
                            )));
                            let consol_str = if consol {
                                "yes — Sleeptime will summarise dropped turns after 60 s idle"
                            } else {
                                "none pending"
                            };
                            let _ = app
                                .push(RenderLine::DimMsg(format!("  Consolidation: {consol_str}")));
                            let _ = app.push(RenderLine::Blank);
                        }
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
                                .map(|s| s.render_model_detail())
                                .unwrap_or_else(|_| {
                                    vec![crate::ui::RenderLine::DimMsg(
                                        "(stats unavailable)".to_string(),
                                    )]
                                }),
                            _ => {
                                // full session card (default)
                                let auth_method = self
                                    .settings
                                    .lock()
                                    .map(|s| {
                                        if s.api_key().is_some() {
                                            "API Key".to_string()
                                        } else {
                                            "OAuth / Browser".to_string()
                                        }
                                    })
                                    .unwrap_or_default();
                                let session_id = self.conversation_id().unwrap_or_default();
                                self.session_stats
                                    .lock()
                                    .map(|s| s.render_card(&auth_method, &session_id))
                                    .unwrap_or_else(|_| {
                                        vec![crate::ui::RenderLine::DimMsg(
                                            "(stats unavailable)".to_string(),
                                        )]
                                    })
                            }
                        };
                        self.tui_blank();
                        for line in lines {
                            let _ = self.app.lock().expect("lock poisoned").push(line);
                        }
                        self.tui_blank();
                    }
                    SlashCmd::Logout => {
                        if let Ok(mut s) = self.settings.lock() {
                            s.clear_api_key();
                        }
                        self.tui_ok("  ✓ API key cleared. Restart CADE to re-authenticate.");
                        return Ok(());
                    }
                    SlashCmd::Plan => {
                        self.permissions.set_mode(PermissionMode::Plan);
                        if let Ok(mut app) = self.app.lock() {
                            if let Some(plan) = &mut app.active_plan {
                                plan.is_visible = true;
                            }
                            app.show_toast("Plan mode enabled", ToastLevel::Info);
                        }
                        self.tui_hdr("📖 Permission mode: plan (read-only) — write/exec tools blocked. Use /default to resume.");
                    }
                    SlashCmd::Todos => {
                        if let Ok(mut app) = self.app.lock() {
                            let mut has_plan = false;
                            let mut now_visible = false;
                            if let Some(plan) = &mut app.active_plan {
                                plan.is_visible = !plan.is_visible;
                                now_visible = plan.is_visible;
                                has_plan = true;
                            }
                            if !has_plan {
                                let _ = app.push(crate::ui::RenderLine::SystemMsg(
                                    "No active plan. Ask the agent to create one.".to_string(),
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
                    SlashCmd::Default => {
                        self.permissions.set_mode(PermissionMode::Default);
                        self.app
                            .lock()
                            .expect("lock poisoned")
                            .show_toast("Permission mode: default", ToastLevel::Success);
                        self.tui_ok("✅ Permission mode: default — tools require approval");
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
                                        self.app.lock().expect("lock poisoned").show_toast(
                                            format!("{icon} {label}"),
                                            ToastLevel::Success,
                                        );
                                        self.tui_ok(format!("{icon} Permission mode: {label}"));
                                    }
                                    Some("plan") => {
                                        self.permissions.set_mode(PermissionMode::Plan);
                                        let (icon, label, hint) =
                                            mode_display(PermissionMode::Plan);
                                        self.app.lock().expect("lock poisoned").show_toast(
                                            format!("{icon} {label}"),
                                            ToastLevel::Info,
                                        );
                                        self.tui_hdr(format!(
                                            "{icon} Permission mode: {label} {hint}"
                                        ));
                                    }
                                    Some("yolo") => {
                                        self.permissions
                                            .set_mode(PermissionMode::BypassPermissions);
                                        let (icon, label, _) =
                                            mode_display(PermissionMode::BypassPermissions);
                                        self.app.lock().expect("lock poisoned").show_toast(
                                            format!("{icon} {label}"),
                                            ToastLevel::Warning,
                                        );
                                        self.tui_sys(format!("{icon} Permission mode: {label}"));
                                    }
                                    Some("acceptEdits") => {
                                        self.permissions.set_mode(PermissionMode::AcceptEdits);
                                        let (icon, label, _) =
                                            mode_display(PermissionMode::AcceptEdits);
                                        self.app.lock().expect("lock poisoned").show_toast(
                                            format!("{icon} {label}"),
                                            ToastLevel::Success,
                                        );
                                        self.tui_ok(format!("{icon} Permission mode: {label}"));
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
                                    let _ = self.app.lock().expect("lock poisoned").draw();
                                    continue;
                                }
                            }
                        } else {
                            m
                        };
                        let new_toolset = Toolset::for_model(&m);
                        let old_toolset = *self.current_toolset.lock().expect("lock poisoned");
                        self.tui_dim(format!("  Switching model → {m}…"));
                        match self.client.patch_agent_model(&self.agent_id(), &m).await {
                            Ok(new_model) => {
                                *self.current_model.lock().expect("lock poisoned") =
                                    new_model.clone();
                                if new_toolset != old_toolset {
                                    *self.current_toolset.lock().expect("lock poisoned") =
                                        new_toolset;
                                    self.spawn_tool_reregister();
                                    self.tui_hdr(format!(
                                        "  Toolset → {}",
                                        new_toolset.display_name()
                                    ));
                                }
                                self.tui_ok(format!("  ✓ Model: {new_model}"));
                                {
                                    let mut app = self.app.lock().expect("lock poisoned");
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
                                    let _ = self.app.lock().expect("lock poisoned").draw();
                                    continue;
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
                            *self.reasoning_effort.lock().expect("lock poisoned") = effort.clone();
                            {
                                let mut app = self.app.lock().expect("lock poisoned");
                                app.reasoning_effort = effort;
                                app.show_toast(format!("Reasoning → {r}"), ToastLevel::Success);
                            }
                            self.tui_ok(format!("  ✓ Reasoning effort: {r}"));
                        }
                    }

                    // -- New commands
                    SlashCmd::Clear => {
                        let _ = self.app.lock().expect("lock poisoned").clear_content();
                        match self.client.clear_messages(&self.agent_id()).await {
                            Ok(n) => self
                                .tui_ok(format!("✓ Context window cleared ({n} messages deleted)")),
                            Err(e) => self
                                .tui_sys(format!("⚠ Screen cleared (context clear failed: {e})")),
                        }
                    }

                    SlashCmd::Cost => {
                        let (total_cost, by_model) = {
                            let stats = self.session_stats.lock().expect("lock poisoned");
                            stats.compute_cost()
                        };
                        let (wall_ms, api_ms, lines_added, lines_removed) = {
                            let stats = self.session_stats.lock().expect("lock poisoned");
                            (
                                stats.started_at.elapsed().as_millis() as u64,
                                stats.agent_active_ms,
                                stats.lines_added,
                                stats.lines_removed,
                            )
                        };
                        let per_model_snap: Vec<(String, ModelStats)> = {
                            let stats = self.session_stats.lock().expect("lock poisoned");
                            stats
                                .per_model
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect()
                        };

                        let fmt_dur = |ms: u64| -> String {
                            let s = ms / 1000;
                            if s >= 3600 {
                                format!("{}h {}m {}s", s / 3600, (s % 3600) / 60, s % 60)
                            } else if s >= 60 {
                                format!("{}m {}s", s / 60, s % 60)
                            } else {
                                format!("{}s", s)
                            }
                        };
                        let fmt_tok = |n: u64| -> String {
                            if n >= 1_000_000 {
                                format!("{:.1}M", n as f64 / 1_000_000.0)
                            } else if n >= 1_000 {
                                format!("{:.1}k", n as f64 / 1_000.0)
                            } else {
                                n.to_string()
                            }
                        };

                        let mut lines: Vec<crate::ui::RenderLine> = vec![
                            crate::ui::RenderLine::Blank,
                            crate::ui::RenderLine::InfoHeader("  ◆ Session Cost".to_string()),
                            crate::ui::RenderLine::Blank,
                            crate::ui::RenderLine::Pair {
                                label: "Total cost".to_string(),
                                value: format!("${:.2}", total_cost),
                            },
                            crate::ui::RenderLine::Pair {
                                label: "Total duration (API)".to_string(),
                                value: fmt_dur(api_ms),
                            },
                            crate::ui::RenderLine::Pair {
                                label: "Total duration (wall)".to_string(),
                                value: fmt_dur(wall_ms),
                            },
                        ];
                        if lines_added != 0 || lines_removed != 0 {
                            lines.push(crate::ui::RenderLine::Pair {
                                label: "Total code changes".to_string(),
                                value: format!(
                                    "{} lines added, {} lines removed",
                                    lines_added,
                                    lines_removed.abs()
                                ),
                            });
                        }
                        if !by_model.is_empty() {
                            lines.push(crate::ui::RenderLine::Blank);
                            lines.push(crate::ui::RenderLine::DimMsg(
                                "  Usage by model:".to_string(),
                            ));
                            for (model, cost) in &by_model {
                                if let Some(ms) = per_model_snap
                                    .iter()
                                    .find(|(k, _)| k == model)
                                    .map(|(_, v)| v)
                                {
                                    let model_short =
                                        model.rsplit('/').next().unwrap_or(model.as_str());
                                    lines.push(crate::ui::RenderLine::DimMsg(format!(
                                        "     {}   (${:.2})",
                                        model_short, cost,
                                    )));
                                    let mut fields: Vec<String> = Vec::new();
                                    if ms.input_tokens > 0 {
                                        fields.push(format!("{} input", fmt_tok(ms.input_tokens)));
                                    }
                                    if ms.output_tokens > 0 {
                                        fields
                                            .push(format!("{} output", fmt_tok(ms.output_tokens)));
                                    }
                                    if ms.cache_read_tokens > 0 {
                                        fields.push(format!(
                                            "{} cache read",
                                            fmt_tok(ms.cache_read_tokens)
                                        ));
                                    }
                                    if ms.cache_write_tokens > 0 {
                                        fields.push(format!(
                                            "{} cache write",
                                            fmt_tok(ms.cache_write_tokens)
                                        ));
                                    }
                                    if !fields.is_empty() {
                                        lines.push(crate::ui::RenderLine::DimMsg(format!(
                                            "       {}",
                                            fields.join(" · ")
                                        )));
                                    }
                                }
                            }
                        }
                        lines.push(crate::ui::RenderLine::Blank);
                        lines.push(crate::ui::RenderLine::DimMsg(
                            "  Pricing estimates — check provider docs for current rates."
                                .to_string(),
                        ));
                        lines.push(crate::ui::RenderLine::Blank);
                        let mut app = self.app.lock().expect("lock poisoned");
                        for line in lines {
                            let _ = app.push(line);
                        }
                    }

                    SlashCmd::Copy => {
                        let mut app = self.app.lock().expect("lock poisoned");
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
                                self.app.lock().expect("lock poisoned").show_toast(
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
                                self.app.lock().expect("lock poisoned").show_toast(
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
                        let agent_id = self.agent_id();
                        loop {
                            match self.client.list_checkpoints(&agent_id).await {
                                Err(e) => {
                                    self.tui_err(format!("  ✗ list_checkpoints: {e}"));
                                    break;
                                }
                                Ok(checkpoints) if checkpoints.is_empty() => {
                                    self.tui_dim(
                                        "  No checkpoints yet. Use /checkpoint [label] to create one."
                                            .to_string(),
                                    );
                                    break;
                                }
                                Ok(checkpoints) => {
                                    // Show the fullscreen tree browser
                                    let action = {
                                        let mut app = self.app.lock().expect("lock poisoned");
                                        let colors = app.colors.clone();
                                        cade_tui::show_session_tree(
                                            &mut app.terminal,
                                            &checkpoints,
                                            &colors,
                                        )
                                    };
                                    match action {
                                        Ok(cade_tui::TreeAction::Cancel) => {
                                            self.app.lock().expect("lock poisoned").show_toast(
                                                "Checkpoint browser closed",
                                                ToastLevel::Info,
                                            );
                                            self.tui_dim("  /tree cancelled".to_string());
                                            break;
                                        }
                                        Ok(cade_tui::TreeAction::Delete { checkpoint_id }) => {
                                            // Confirm with question
                                            let title = checkpoints
                                                .iter()
                                                .find(|cp| {
                                                    cp["id"].as_str() == Some(&checkpoint_id)
                                                })
                                                .and_then(|cp| cp["label"].as_str())
                                                .unwrap_or("(unlabelled)")
                                                .to_string();
                                            use crate::ui::question::{Question, QuestionOption};
                                            let q = Question {
                                                header: "Delete Checkpoint?".to_string(),
                                                text: format!("Delete checkpoint \"{title}\"?"),
                                                options: vec![
                                                    QuestionOption {
                                                        label: "Yes — delete".to_string(),
                                                        description: String::new(),
                                                    },
                                                    QuestionOption {
                                                        label: "No — keep".to_string(),
                                                        description: String::new(),
                                                    },
                                                ],
                                                multi_select: false,
                                                allow_other: false,
                                                progress: None,
                                            };
                                            let ans = {
                                                let mut app =
                                                    self.app.lock().expect("lock poisoned");
                                                let r = app.ask_question(&q);
                                                app.scroll = 0;
                                                let _ = app.draw();
                                                r
                                            };
                                            if let Ok(Some(a)) = ans
                                                && a.as_str().starts_with("Yes")
                                            {
                                                // Drop git stash if exists
                                                let stash_ref = checkpoints
                                                    .iter()
                                                    .find(|cp| {
                                                        cp["id"].as_str() == Some(&checkpoint_id)
                                                    })
                                                    .and_then(|cp| cp["git_stash_ref"].as_str())
                                                    .map(String::from);
                                                if let Some(s) = stash_ref {
                                                    use cade_agent::tools::git_checkpoint;
                                                    let _ = git_checkpoint::delete_git_checkpoint(
                                                        &s, &self.cwd,
                                                    )
                                                    .await;
                                                }
                                                // Delete from server
                                                match self
                                                    .client
                                                    .delete_checkpoint(&agent_id, &checkpoint_id)
                                                    .await
                                                {
                                                    Ok(_) => {
                                                        self.app
                                                            .lock()
                                                            .expect("lock poisoned")
                                                            .show_toast(
                                                                format!(
                                                                    "Deleted checkpoint {title}"
                                                                ),
                                                                ToastLevel::Success,
                                                            );
                                                        self.tui_ok(format!(
                                                            "  ✓ Deleted checkpoint {title}"
                                                        ));
                                                    }
                                                    Err(e) => self.tui_err(format!(
                                                        "  ✗ Failed to delete checkpoint: {e}"
                                                    )),
                                                }
                                            }
                                            continue;
                                        }
                                        Ok(cade_tui::TreeAction::Restore { checkpoint_id }) => {
                                            self.tui_dim(format!(
                                                "  Restoring checkpoint {checkpoint_id}…"
                                            ));
                                            // Find git stash ref in the checkpoint list
                                            let stash_ref = checkpoints
                                                .iter()
                                                .find(|cp| {
                                                    cp["id"].as_str() == Some(&checkpoint_id)
                                                })
                                                .and_then(|cp| cp["git_stash_ref"].as_str())
                                                .map(String::from);
                                            if let Some(s) = stash_ref {
                                                use cade_agent::tools::git_checkpoint;
                                                match git_checkpoint::restore_git_checkpoint(
                                                    &s, &self.cwd,
                                                )
                                                .await
                                                {
                                                    Ok(()) => self.tui_ok(format!(
                                                        "  ✓ Git stash applied: {s}"
                                                    )),
                                                    Err(e) => self
                                                        .tui_err(format!("  ✗ Git restore: {e}")),
                                                }
                                            }
                                            let _ = self
                                                .client
                                                .restore_checkpoint(&agent_id, &checkpoint_id)
                                                .await;
                                            self.app.lock().expect("lock poisoned").show_toast(
                                                format!("Restored checkpoint {checkpoint_id}"),
                                                ToastLevel::Success,
                                            );
                                            self.tui_ok(format!(
                                                "  ✓ Restored to checkpoint {checkpoint_id}"
                                            ));
                                            break;
                                        }
                                        Err(e) => {
                                            self.tui_err(format!("  ✗ Tree error: {e}"));
                                            break;
                                        }
                                    }
                                }
                            }
                        }
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
                                        *self.conversation_id.lock().expect("lock poisoned") =
                                            Some(cid.clone());
                                        if let Ok(mut s) = self.session.lock() {
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
                                            let s = self.settings.lock().expect("lock poisoned");
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
                            continue;
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
                            continue;
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
                                *self.conversation_id.lock().expect("lock poisoned") =
                                    Some(cid.clone());
                                if let Ok(mut s) = self.session.lock() {
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
                        let _ = self
                            .app
                            .lock()
                            .expect("lock poisoned")
                            .push(RenderLine::SystemMsg("  Creating new agent…".to_string()));

                        // S5: Offer to copy `human` and `project` blocks from current agent
                        let prev_agent_id = self.agent_id();
                        let inherit_blocks: Vec<(String, String, String)> = {
                            let blocks = self
                                .client
                                .get_memory(&prev_agent_id)
                                .await
                                .unwrap_or_default();
                            blocks
                                .into_iter()
                                .filter(|b| {
                                    (b.label == "human" || b.label == "project")
                                        && !b.value.trim().is_empty()
                                })
                                .map(|b| {
                                    (
                                        b.label.clone(),
                                        b.value.clone(),
                                        b.description.clone().unwrap_or_default(),
                                    )
                                })
                                .collect()
                        };
                        let copy_memory = if !inherit_blocks.is_empty() {
                            let summary: String = inherit_blocks
                                .iter()
                                .map(|(l, v, _)| format!("{} ({} chars)", l, v.chars().count()))
                                .collect::<Vec<_>>()
                                .join(", ");
                            let q = crate::ui::question::Question {
                                header: "Copy memory".to_string(),
                                text: format!("Copy memory to new agent? ({summary})"),
                                options: vec![
                                    crate::ui::question::QuestionOption {
                                        label: "Yes — copy human + project blocks".to_string(),
                                        description: "Start new agent with existing context"
                                            .to_string(),
                                    },
                                    crate::ui::question::QuestionOption {
                                        label: "No — start fresh".to_string(),
                                        description: "New agent gets empty memory blocks"
                                            .to_string(),
                                    },
                                ],
                                multi_select: false,
                                allow_other: false,
                                progress: None,
                            };
                            let ans = {
                                let mut app = self.app.lock().expect("lock poisoned");
                                let r = app.ask_question(&q);
                                app.scroll = 0;
                                let _ = app.draw();
                                r
                            };
                            matches!(&ans, Ok(Some(a)) if a.as_str().starts_with("Yes"))
                        } else {
                            false
                        };

                        let model = self.model();
                        let req = cade_agent::agent::client::CreateAgentRequest {
                            name: Some(format!(
                                "CADE-{}",
                                chrono::Local::now().format("%Y%m%d-%H%M%S")
                            )),
                            model,
                            description: Some("CADE coding agent".to_string()),
                            system_prompt: None,
                            memory_blocks: vec![],
                            tool_ids: vec![],
                        };
                        match self.client.create_agent(req).await {
                            Ok(a) => {
                                *self.agent_id.lock().expect("lock poisoned") = a.id.clone();
                                *self.agent_name.lock().expect("lock poisoned") = a.name.clone();
                                *self.conversation_id.lock().expect("lock poisoned") = None;
                                if let Ok(mut s) = self.settings.lock() {
                                    let _ = s.set_last_agent(&a.id);
                                }
                                if let Ok(mut s) = self.session.lock() {
                                    let _ = s.set_agent(a.id.clone(), Some(a.name.clone()));
                                }
                                let _ = self.app.lock().expect("lock poisoned").push(
                                    RenderLine::SystemMsg(format!(
                                        "  ✓ New agent: {} ({})",
                                        a.name, a.id
                                    )),
                                );

                                // S5: copy inherited blocks to new agent
                                if copy_memory {
                                    for (label, value, desc) in &inherit_blocks {
                                        let desc_opt = if desc.is_empty() {
                                            None
                                        } else {
                                            Some(desc.as_str())
                                        };
                                        let _ = self
                                            .client
                                            .upsert_memory(&a.id, label, value, desc_opt)
                                            .await;
                                    }
                                    let n = inherit_blocks.len();
                                    let _ = self.app.lock().expect("lock poisoned").push(
                                        RenderLine::SystemMsg(format!(
                                            "  ✓ Copied {n} memory block(s) from previous agent"
                                        )),
                                    );
                                }

                                // Attach native + MCP tools in background
                                let client2 = self.client.clone();
                                let mcp2 = std::sync::Arc::clone(&self.mcp);
                                let toolset2 = *self.current_toolset.lock().expect("lock poisoned");
                                let new_id = a.id.clone();
                                tokio::spawn(async move {
                                    use cade_agent::agent::tools::{
                                        register_cade_tools, register_mcp_tools,
                                    };
                                    let native_ids: Vec<String> =
                                        register_cade_tools(&client2, toolset2)
                                            .await
                                            .unwrap_or_default()
                                            .into_iter()
                                            .map(|t| t.id)
                                            .collect();
                                    if !native_ids.is_empty() {
                                        let _ =
                                            client2.attach_agent_tools(&new_id, &native_ids).await;
                                    }
                                    let mcp_ids: Vec<String> =
                                        register_mcp_tools(&client2, mcp2.all_tool_schemas().await)
                                            .await
                                            .unwrap_or_default()
                                            .into_iter()
                                            .map(|t| t.id)
                                            .collect();
                                    if !mcp_ids.is_empty() {
                                        let _ = client2.attach_agent_tools(&new_id, &mcp_ids).await;
                                    }
                                });
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        }
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
                                            .expect("lock poisoned")
                                            .push(RenderLine::DimMsg(
                                            "  No saved conversations yet. Use /new to start one."
                                                .to_string(),
                                        ));
                                } else if let Some(picked) = self
                                    .conversation_picker(Arc::clone(&self.app), &convs, &agent_id)
                                    .await?
                                {
                                    let cid = picked["id"].as_str().unwrap_or("").to_string();
                                    *self.conversation_id.lock().expect("lock poisoned") =
                                        Some(cid.clone());
                                    if let Ok(mut s) = self.session.lock() {
                                        let _ = s.set_conversation(Some(cid));
                                    }
                                    self.first_turn
                                        .store(false, std::sync::atomic::Ordering::SeqCst);
                                    let _ = self.app.lock().expect("lock poisoned").push(
                                        RenderLine::SuccessMsg(format!(
                                            "  ✓ Switched to: {}",
                                            picked["title"].as_str().unwrap_or("(untitled)")
                                        )),
                                    );
                                }
                                let _ = self.app.lock().expect("lock poisoned").draw();
                            }
                            Err(e) => {
                                let _ = self
                                    .app
                                    .lock()
                                    .expect("lock poisoned")
                                    .push(RenderLine::ErrorMsg(e.to_string()));
                            }
                        }
                    }

                    SlashCmd::Pin => {
                        let id = self.agent_id();
                        let name = self.agent_name();
                        if let Ok(mut s) = self.settings.lock() {
                            match s.pin_agent(&id, &name) {
                                Ok(_) => {
                                    self.app.lock().expect("lock poisoned").show_toast(
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
                        if self.require_capability(
                            cade_core::capabilities::Capability::Agentic,
                            "/agents",
                        ) {
                            continue;
                        }
                        self.tui_dim("  Fetching agents…");
                        match self.client.list_agents().await {
                            Ok(agents) if agents.is_empty() => {
                                self.tui_dim("  (no agents found)");
                            }
                            Ok(mut agents) => {
                                if let Some(result) = self
                                    .agent_picker(Arc::clone(&self.app), &mut agents)
                                    .await?
                                {
                                    match result {
                                        AgentPickerResult::Switch(a) => {
                                            *self.agent_id.lock().expect("lock poisoned") =
                                                a.id.clone();
                                            *self.agent_name.lock().expect("lock poisoned") =
                                                a.name.clone();
                                            if let Ok(mut s) = self.settings.lock() {
                                                let _ = s.set_last_agent(&a.id);
                                            }
                                            self.tui_ok(format!(
                                                "  ✓ Switched to: {} ({})",
                                                a.name, a.id
                                            ));
                                        }
                                        AgentPickerResult::Rename { agent, new_name } => match self
                                            .client
                                            .rename_agent(&agent.id, &new_name)
                                            .await
                                        {
                                            Ok(_) => {
                                                if agent.id == self.agent_id() {
                                                    *self
                                                        .agent_name
                                                        .lock()
                                                        .expect("lock poisoned") = new_name.clone();
                                                }
                                                self.tui_ok(format!(
                                                    "  ✓ Renamed '{}' → '{new_name}'",
                                                    agent.name
                                                ));
                                            }
                                            Err(e) => self.tui_err(e.to_string()),
                                        },
                                        AgentPickerResult::DeleteMany(to_delete) => {
                                            let current_id = self.agent_id();
                                            let mut deleted_active = false;
                                            for a in &to_delete {
                                                match self.client.delete_agent(&a.id).await {
                                                    Ok(_) => {
                                                        self.tui_ok(format!(
                                                            "  ✓ Deleted: {}",
                                                            a.name
                                                        ));
                                                        if a.id == current_id {
                                                            deleted_active = true;
                                                        }
                                                    }
                                                    Err(e) => self.tui_err(e.to_string()),
                                                }
                                            }
                                            if deleted_active {
                                                match self.client.list_agents().await {
                                                    Ok(remaining) if !remaining.is_empty() => {
                                                        let first = &remaining[0];
                                                        *self
                                                            .agent_id
                                                            .lock()
                                                            .expect("lock poisoned") =
                                                            first.id.clone();
                                                        *self
                                                            .agent_name
                                                            .lock()
                                                            .expect("lock poisoned") =
                                                            first.name.clone();
                                                        if let Ok(mut s) = self.settings.lock() {
                                                            let _ = s.set_last_agent(&first.id);
                                                        }
                                                        self.tui_dim(format!(
                                                            "  → Now using: {}",
                                                            first.name
                                                        ));
                                                    }
                                                    _ => {
                                                        self.tui_dim("  No remaining agents — run /new to create one");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                let _ = self.app.lock().expect("lock poisoned").draw();
                            }
                            Err(e) => self.tui_err(e.to_string()),
                        }
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
                                        let mut app = self.app.lock().expect("lock poisoned");
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
                        self.tui_dim(format!("  Analysing project at {}…", self.cwd.display()));

                        let explore_prompt = format!(
                            "Analyse the project at '{}'. \
                             Read: README.md, Cargo.toml / package.json / pyproject.toml / go.mod (whichever exist), \
                             src/ or lib/ directory structure (top-level only), .env.example if present. \
                             Return a concise report covering: \
                             (1) Project name and purpose (2 sentences), \
                             (2) Language + framework / stack, \
                             (3) Key source directories and their purpose, \
                             (4) Build / test commands, \
                             (5) Any important conventions or notes from README. \
                             Be specific and factual. Maximum 400 words.",
                            self.cwd.display()
                        );

                        let agent_id = self.agent_id();
                        let client = self.client.clone();
                        let cwd = self.cwd.clone();
                        let all_defs = cade_agent::subagents::discover_all_subagents(&cwd);
                        let explore_def =
                            cade_agent::subagents::find_subagent("explore", &all_defs).cloned();
                        let main_model = self.model();
                        let hooks = self.hooks.clone();

                        // Run explore subagent synchronously
                        let summary = {
                            use crate::cli::headless::run_headless;
                            use cade_core::permissions::PermissionManager;

                            let _system_prompt =
                                explore_def.map(|d| d.system_prompt).unwrap_or_else(|| {
                                    "You are an expert code explorer. Be concise and precise."
                                        .to_string()
                                });

                            let req = cade_agent::agent::client::CreateAgentRequest {
                                name: Some("init-explore".to_string()),
                                model: main_model,
                                description: Some("Ephemeral init analysis".to_string()),
                                system_prompt: Some(
                                    "You are an expert code explorer. Be concise and precise."
                                        .to_string(),
                                ),
                                memory_blocks: vec![],
                                tool_ids: vec![],
                            };
                            match client.create_agent(req).await {
                                Ok(sub) => {
                                    let perm = PermissionManager::default();
                                    let mcp_empty =
                                        std::sync::Arc::new(cade_agent::mcp::McpManager::empty());
                                    let result = run_headless(
                                        &client,
                                        &sub.id,
                                        &explore_prompt,
                                        &perm,
                                        &mcp_empty,
                                        &hooks,
                                        None,
                                    )
                                    .await;
                                    let _ = client.delete_agent(&sub.id).await;
                                    result
                                        .map(|(s, _)| s)
                                        .unwrap_or_else(|e| format!("Analysis failed: {e}"))
                                }
                                Err(e) => format!("Could not spawn explore agent: {e}"),
                            }
                        };

                        // Write summary into project memory block
                        let _ = self
                            .client
                            .upsert_memory(&agent_id, "project", &summary, None)
                            .await;

                        // Tell the main agent what was discovered
                        let init_prompt = format!(
                            "[/init completed] Project analysis summary:\n\n{summary}\n\n\
                             I've stored this in your 'project' memory block. \
                             Acknowledge and summarise what you learned in 2-3 sentences."
                        );
                        self.agent_turn(&mut stdout, &init_prompt).await?;
                        let _ = self.app.lock().expect("lock poisoned").commit_streaming();
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
                        let _ = self.app.lock().expect("lock poisoned").commit_streaming();
                    }

                    SlashCmd::Memory => {
                        // Parse subcommand from the raw input line
                        let raw = input.trim();
                        let mem_arg = raw.strip_prefix("/memory").unwrap_or("").trim().to_string();
                        let parts: Vec<&str> = mem_arg.splitn(4, ' ').collect();
                        let sub = parts.first().copied().unwrap_or("");

                        match sub {
                            // /memory view <label> — show full value untruncated
                            "view" | "show" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.get_memory(&id).await {
                                    Ok(blocks) => {
                                        if let Some(b) = blocks.iter().find(|b| b.label == label) {
                                            self.tui_blank();
                                            self.tui_hdr(format!("  [{label}]"));
                                            if let Some(desc) = &b.description
                                                && !desc.is_empty()
                                            {
                                                self.tui_dim(format!("  {desc}"));
                                            }
                                            self.tui_blank();
                                            if b.value.is_empty() {
                                                self.tui_dim("  (empty)");
                                            } else {
                                                for ln in b.value.lines() {
                                                    self.tui_sys(ln.to_string());
                                                }
                                            }
                                        } else {
                                            self.tui_err(format!("Block '{label}' not found"));
                                        }
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            // /memory set <label> <value>
                            "set" if parts.len() >= 3 => {
                                let label = parts[1];
                                let value = parts[2..].join(" ");
                                let id = self.agent_id();
                                match self.client.upsert_memory(&id, label, &value, None).await {
                                    Ok(_) => self.tui_ok(format!("  ✓ [{label}] updated")),
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            // /memory delete <label>
                            "delete" | "del" | "rm" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.delete_memory(&id, label).await {
                                    Ok(_) => self.tui_ok(format!("  ✓ [{label}] deleted")),
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            // /memory edit <label> — inline multi-line editor via QuestionWidget
                            "edit" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                let current = self
                                    .client
                                    .get_memory(&id)
                                    .await
                                    .unwrap_or_default()
                                    .into_iter()
                                    .find(|b| b.label == label)
                                    .map(|b| b.value)
                                    .unwrap_or_default();
                                use crate::ui::question::{Question, QuestionOption};
                                let opts = vec![
                                    QuestionOption {
                                        label: format!(
                                            "Keep: {}…",
                                            current.chars().take(60).collect::<String>()
                                        ),
                                        description: String::new(),
                                    },
                                    QuestionOption {
                                        label: "Clear (erase block)".to_string(),
                                        description: String::new(),
                                    },
                                ];
                                let q = Question {
                                    header: "Edit memory".to_string(),
                                    text: format!("Type new value for [{label}] or pick action:"),
                                    options: opts.clone(),
                                    multi_select: false,
                                    allow_other: true,
                                    progress: None,
                                };
                                let ans = {
                                    let mut app = self.app.lock().expect("lock poisoned");
                                    app.ask_question(&q)?
                                };
                                if let Some(a) = &ans {
                                    let val = a.as_str();
                                    let new_value = if val.starts_with("Clear") {
                                        String::new()
                                    } else if val.starts_with("Keep") {
                                        current
                                    } else {
                                        val.to_string()
                                    };
                                    match self
                                        .client
                                        .upsert_memory(&id, label, &new_value, None)
                                        .await
                                    {
                                        Ok(_) => self.tui_ok(format!("  ✓ [{label}] updated")),
                                        Err(e) => self.tui_err(e.to_string()),
                                    }
                                }
                            }
                            // /memory history <label> — show last 5 revisions
                            "history" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.list_memory_history(&id, label, 5).await {
                                    Ok(revs) if revs.is_empty() => {
                                        let _ = self.app.lock().expect("lock poisoned").push(
                                            RenderLine::SystemMsg(format!(
                                                "  [{label}] no history recorded yet"
                                            )),
                                        );
                                    }
                                    Ok(revs) => {
                                        let _ = self
                                            .app
                                            .lock()
                                            .expect("lock poisoned")
                                            .push(RenderLine::Blank);
                                        for (i, rev) in revs.iter().enumerate() {
                                            let rev_id = rev["id"].as_str().unwrap_or("");
                                            let ts = rev["updated_at"].as_i64().unwrap_or(0);
                                            let val = rev["value"].as_str().unwrap_or("");
                                            let preview: String = val.chars().take(120).collect();
                                            let ellipsis = if val.len() > 120 { "…" } else { "" };
                                            let _ = self.app.lock().expect("lock poisoned").push(
                                                RenderLine::SystemMsg(format!(
                                                    "  [{i}] {ts}  id={rev_id}"
                                                )),
                                            );
                                            let _ = self.app.lock().expect("lock poisoned").push(
                                                RenderLine::SystemMsg(format!(
                                                    "      {preview}{ellipsis}"
                                                )),
                                            );
                                            let _ = self
                                                .app
                                                .lock()
                                                .expect("lock poisoned")
                                                .push(RenderLine::Blank);
                                        }
                                        let _ = self.app.lock().expect("lock poisoned").push(
                                            RenderLine::SystemMsg(format!(
                                                "  Use: /memory restore {label} <id>"
                                            )),
                                        );
                                    }
                                    Err(e) => {
                                        let _ = self
                                            .app
                                            .lock()
                                            .expect("lock poisoned")
                                            .push(RenderLine::ErrorMsg(format!("  ✗ {e}")));
                                    }
                                }
                            }
                            // /memory restore <label> <rev_id>
                            "restore" if parts.len() >= 3 => {
                                let label = parts[1];
                                let rev_id = parts[2];
                                let id = self.agent_id();
                                match self.client.restore_memory(&id, label, rev_id).await {
                                    Ok(_) => {
                                        let _ = self.app.lock().expect("lock poisoned").push(
                                            RenderLine::SystemMsg(format!(
                                                "  ✓ [{label}] restored to revision {rev_id}"
                                            )),
                                        );
                                    }
                                    Err(e) => {
                                        let _ = self
                                            .app
                                            .lock()
                                            .expect("lock poisoned")
                                            .push(RenderLine::ErrorMsg(format!("  ✗ {e}")));
                                    }
                                }
                            }
                            // /memory pin <label>
                            "pin" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.pin_memory(&id, label).await {
                                    Ok(_) => self
                                        .tui_ok(format!("  📌 [{label}] pinned — always injected")),
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            // /memory unpin <label>
                            "unpin" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.promote_memory(&id, label).await {
                                    Ok(_) => {
                                        self.tui_ok(format!("  ● [{label}] unpinned → short-term"))
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            // /memory promote <label> — reactivate archived block
                            "promote" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.promote_memory(&id, label).await {
                                    Ok(_) => {
                                        self.tui_ok(format!("  ● [{label}] promoted → short-term"))
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                            // /memory demote <label> — manually archive block
                            "demote" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                match self.client.demote_memory(&id, label).await {
                                    Ok(_) => self.tui_ok(format!(
                                        "  ○ [{label}] demoted → long-term (archived)"
                                    )),
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }

                            // /memory why <label> — show provenance chain
                            "why" if parts.len() >= 2 => {
                                let label = parts[1];
                                let id = self.agent_id();
                                self.tui_dim(format!("  Looking up provenance for '{label}'…"));
                                match self.client.get_memory_why(&id, label).await {
                                    Ok(summary) => {
                                        self.tui_blank();
                                        for line in summary.lines() {
                                            self.tui_sys(format!("  {line}"));
                                        }
                                    }
                                    Err(e) => self.tui_err(format!("  ✗ {e}")),
                                }
                            }

                            // /memory typed [type] — filter blocks by memory_type
                            "typed" => {
                                let filter = parts.get(1).copied();
                                let id = self.agent_id();
                                match self.client.get_memory(&id).await {
                                    Ok(blocks) => {
                                        let label = filter.unwrap_or("all");
                                        self.tui_hdr(format!("  Memory blocks (type={label}):"));
                                        let mut shown = 0;
                                        for b in &blocks {
                                            // Only blocks with a type label match (server doesn't
                                            // return memory_type yet; shown inline via describe)
                                            shown += 1;
                                            self.tui_dim(format!(
                                                "  [{badge}]  {label}",
                                                badge = b.tier.as_deref().unwrap_or("short"),
                                                label = b.label,
                                            ));
                                        }
                                        if shown == 0 {
                                            self.tui_dim("  (none)".to_string());
                                        }
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }

                            // /memory audit — find stale / low-confidence blocks
                            "audit" => {
                                let id = self.agent_id();
                                match self.client.get_memory(&id).await {
                                    Ok(blocks) => {
                                        let empty_blocks: Vec<_> = blocks
                                            .iter()
                                            .filter(|b| b.value.trim().is_empty())
                                            .collect();
                                        let long_blocks: Vec<_> = blocks
                                            .iter()
                                            .filter(|b| b.tier.as_deref() == Some("long"))
                                            .collect();
                                        self.tui_hdr(format!(
                                            "  Memory audit — {} total blocks:",
                                            blocks.len()
                                        ));
                                        if !empty_blocks.is_empty() {
                                            self.tui_dim(format!(
                                                "  ⚠  {} empty block(s): {}",
                                                empty_blocks.len(),
                                                empty_blocks
                                                    .iter()
                                                    .map(|b| b.label.as_str())
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            ));
                                        }
                                        if !long_blocks.is_empty() {
                                            self.tui_dim(format!(
                                                "  ○  {} archived block(s): {}",
                                                long_blocks.len(),
                                                long_blocks
                                                    .iter()
                                                    .map(|b| b.label.as_str())
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            ));
                                        }
                                        if empty_blocks.is_empty() && long_blocks.is_empty() {
                                            self.tui_ok(
                                                "  ✓ All blocks active and populated.".to_string(),
                                            );
                                        }
                                        self.tui_dim("  Use /reflect to trigger automatic extraction from conversation.".to_string());
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }

                            // /memory suggest — run lightweight reflection
                            "suggest" => {
                                let id = self.agent_id();
                                self.tui_dim("  Triggering reflection…".to_string());
                                match self.client.trigger_reflect(&id, None).await {
                                    Ok(summary) => self.tui_ok(format!("  {summary}")),
                                    Err(e) => self.tui_err(format!("  ✗ {e}")),
                                }
                            }

                            // /memory (list)
                            _ => {
                                let id = self.agent_id();
                                match self.client.get_memory(&id).await {
                                    Ok(mut blocks) => {
                                        loop {
                                            match self.memory_picker(std::sync::Arc::clone(&self.app), &mut blocks).await {
                                                Ok(Some(MemoryPickerResult::Edit(b))) => {
                                                    pending_input = Some(format!("/memory edit {}", b.label));
                                                    break;
                                                }
                                                Ok(Some(MemoryPickerResult::Delete(b))) => {
                                                    pending_input = Some(format!("/memory delete {}", b.label));
                                                    break;
                                                }
                                                Ok(Some(MemoryPickerResult::TogglePin(b))) => {
                                                    let is_pinned = b.tier.as_deref() == Some("pinned");
                                                    let cmd = if is_pinned { "unpin" } else { "pin" };
                                                    pending_input = Some(format!("/memory {cmd} {}", b.label));
                                                    break;
                                                }
                                                Ok(None) => break, // cancelled
                                                Err(e) => {
                                                    self.tui_err(e.to_string());
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => self.tui_err(e.to_string()),
                                }
                            }
                        }
                    }

                    SlashCmd::Search(query) => {
                        if query.is_empty() {
                            self.tui_dim("  Usage: /search <query>");
                            continue;
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
                        let sub = arg.as_deref().unwrap_or("list");
                        let (sub_cmd, sub_arg) = sub
                            .splitn(2, ' ')
                            .collect::<Vec<_>>()
                            .split_first()
                            .map(|(c, r)| (*c, r.join(" ")))
                            .unwrap_or(("list", String::new()));

                        match sub_cmd {
                            "list" | "" => {
                                let skills = self.skills.lock().expect("lock poisoned");
                                let agent_id = self.agent_id();
                                if skills.is_empty() {
                                    let mut app = self.app.lock().expect("lock poisoned");
                                    let _ = app.push(RenderLine::Blank);
                                    let _ = app.push(RenderLine::InfoHeader(
                                        "  ◆ Skills  (none loaded)".to_string(),
                                    ));
                                    let _ = app.push(RenderLine::Blank);
                                    let _ = app.push(RenderLine::DimMsg(
                                        "  No skills found. Searched:".to_string(),
                                    ));
                                    let _ = app.push(RenderLine::Pair {
                                        label: "project".to_string(),
                                        value: ".skills/".to_string(),
                                    });
                                    let _ = app.push(RenderLine::Pair {
                                        label: "global".to_string(),
                                        value: "~/.cade/skills/".to_string(),
                                    });
                                    let _ = app.push(RenderLine::Pair {
                                        label: "agent".to_string(),
                                        value: format!("~/.cade/agents/{agent_id}/skills/"),
                                    });
                                    let _ = app.push(RenderLine::Blank);
                                    let _ = app.push(RenderLine::DimMsg(
                                        "  /skills create <name>  to scaffold your first skill"
                                            .to_string(),
                                    ));
                                    let _ = app.push(RenderLine::Blank);
                                } else {
                                    let scope_ord = |s: &str| match s {
                                        "project" => 0u8,
                                        "agent" => 1,
                                        "global" => 2,
                                        _ => 3,
                                    };
                                    let mut sorted: Vec<_> = skills.iter().cloned().collect();
                                    sorted.sort_by(|a, b| {
                                        scope_ord(&a.scope.to_string())
                                            .cmp(&scope_ord(&b.scope.to_string()))
                                            .then(a.id.cmp(&b.id))
                                    });
                                    drop(skills);

                                    let chosen = {
                                        let mut app = self.app.lock().expect("lock poisoned");
                                        let colors = app.colors.clone();
                                        crate::ui::skills::show_skills_manager(
                                            &mut app.terminal,
                                            sorted,
                                            &colors,
                                        )?
                                    };
                                    let _ = self.app.lock().expect("lock poisoned").draw();

                                    if let Some(crate::ui::skills::SkillsAction::Reload) = chosen {
                                        pending_input = Some("/skills reload".to_string());
                                    }
                                }
                            }

                            "create" => {
                                let name_raw = sub_arg.trim().to_string();
                                if name_raw.is_empty() {
                                    self.tui_dim("  Usage: /skills create <name>");
                                } else {
                                    let slug: String = name_raw
                                        .to_lowercase()
                                        .chars()
                                        .map(|c| if c.is_alphanumeric() { c } else { '-' })
                                        .collect::<String>()
                                        .trim_matches('-')
                                        .to_string();
                                    let skill_dir = self.skills_dir.join(&slug);
                                    let skill_file = skill_dir.join("SKILL.MD");
                                    if skill_file.exists() {
                                        self.tui_err(format!(
                                            "Skill '{}' already exists: {}",
                                            slug,
                                            skill_file.display()
                                        ));
                                    } else {
                                        match std::fs::create_dir_all(&skill_dir) {
                                            Ok(_) => {
                                                let title: String = slug
                                                    .replace('-', " ")
                                                    .split_whitespace()
                                                    .map(|w| {
                                                        let mut c = w.chars();
                                                        match c.next() {
                                                            None => String::new(),
                                                            Some(f) => {
                                                                f.to_uppercase().collect::<String>()
                                                                    + c.as_str()
                                                            }
                                                        }
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .join(" ");
                                                let template = format!(
                                                    "---\nname: {title}\ndescription: One-line description of what this skill does\ncategory: general\ntags: []\n---\n\n\
                                                    # {title}\n\nDescribe the skill here. This text is injected into the agent's\n\
                                                    system prompt when this skill is loaded.\n\n\
                                                    You can use markdown, code blocks, examples, step-by-step instructions, etc.\n"
                                                );
                                                match std::fs::write(&skill_file, template) {
                                                    Ok(_) => {
                                                        self.tui_ok(format!(
                                                            "  ✓ Created: {}",
                                                            skill_file.display()
                                                        ));
                                                        self.tui_dim(format!("  /skills edit {slug}  to open now  ·  /skills reload  to activate"));
                                                    }
                                                    Err(e) => self.tui_err(format!(
                                                        "Failed to write skill file: {e}"
                                                    )),
                                                }
                                            }
                                            Err(e) => self.tui_err(format!(
                                                "Failed to create directory: {e}"
                                            )),
                                        }
                                    }
                                }
                            }

                            "show" => {
                                self.tui_dim("  The /skills show command has been deprecated.");
                                self.tui_dim(
                                    "  Please type /skills to open the interactive skills manager.",
                                );
                            }

                            "reload" => {
                                let agent_id = self.agent_id();
                                let new_skills = cade_core::skills::discover_all_skills(
                                    &self.cwd,
                                    Some(&agent_id),
                                    None,
                                );
                                let prev_count = self.skills.lock().expect("lock poisoned").len();
                                let new_count = new_skills.len();

                                let existing =
                                    self.client.get_memory(&agent_id).await.unwrap_or_default();
                                for block in &existing {
                                    if block.label.starts_with("skill:") {
                                        let _ = self
                                            .client
                                            .delete_memory(&agent_id, &block.label)
                                            .await;
                                    }
                                }
                                let mut names = vec![];
                                for skill in &new_skills {
                                    let label = format!("skill:{}", skill.id);
                                    let _ = self
                                        .client
                                        .upsert_memory(
                                            &agent_id,
                                            &label,
                                            &skill.to_context_block(),
                                            None,
                                        )
                                        .await;
                                    names.push(skill.name.clone());
                                }

                                let listing = cade_core::skills::skills_listing(&new_skills);
                                let _ = self
                                    .client
                                    .upsert_memory(
                                        &agent_id,
                                        "skills",
                                        listing.as_deref().unwrap_or(""),
                                        None,
                                    )
                                    .await;

                                *self.skills.lock().expect("lock poisoned") = new_skills;

                                self.tui_ok(format!(
                                    "  ✓ Skills reloaded  ({new_count} loaded, was {prev_count})"
                                ));

                                if new_count > 0 {
                                    let list = names.join(", ");
                                    let notify = format!(
                                        "[System: Skills reloaded. Now active: {list}. \
                                                 Use load_skill(id) to load any skill's full content.]"
                                    );
                                    self.agent_turn(&mut stdout, &notify).await?;
                                    let _ =
                                        self.app.lock().expect("lock poisoned").commit_streaming();
                                }
                            }

                            "edit" => {
                                self.tui_dim("  The /skills edit command has been deprecated.");
                                self.tui_dim(
                                    "  Please type /skills to open the interactive skills manager.",
                                );
                            }

                            "delete" | "rm" => {
                                let id = sub_arg.trim();
                                if id.is_empty() {
                                    self.tui_err("  Usage: /skills delete <id>");
                                } else {
                                    let skill_dir = self.skills_dir.join(id);
                                    if !skill_dir.exists() {
                                        self.tui_err(format!(
                                            "  Skill directory not found: {}",
                                            skill_dir.display()
                                        ));
                                        self.tui_dim("  Run /skills to list available skills.");
                                    } else {
                                        self.tui_sys(format!(
                                            "  Deleting skill '{id}' at: {}",
                                            skill_dir.display()
                                        ));
                                        match std::fs::remove_dir_all(&skill_dir) {
                                            Ok(_) => {
                                                // Remove from in-memory list
                                                self.skills
                                                    .lock()
                                                    .expect("lock poisoned")
                                                    .retain(|s| s.id != id);
                                                // Update memory
                                                let agent_id = self.agent_id();
                                                let skills_snap = self
                                                    .skills
                                                    .lock()
                                                    .expect("lock poisoned")
                                                    .clone();
                                                let listing =
                                                    cade_core::skills::skills_listing(&skills_snap);
                                                let _ = self
                                                    .client
                                                    .upsert_memory(
                                                        &agent_id,
                                                        "skills",
                                                        listing.as_deref().unwrap_or(""),
                                                        None,
                                                    )
                                                    .await;
                                                let _ = self
                                                    .client
                                                    .delete_memory(
                                                        &agent_id,
                                                        &format!("skill:{id}"),
                                                    )
                                                    .await;
                                                self.tui_ok(format!("  ✓ Deleted skill '{id}'"));
                                                self.tui_dim(
                                                    "  /skills reload  to update agent context",
                                                );
                                            }
                                            Err(e) => {
                                                self.tui_err(format!("  Failed to delete: {e}"))
                                            }
                                        }
                                    }
                                }
                            }

                            other => {
                                self.tui_err(format!("  Unknown /skills subcommand: '{other}'"));
                                self.tui_blank();
                                self.tui_dim("  /skills                    — open interactive skills manager");
                                self.tui_dim("  /skills create <name>      — scaffold a new skill");
                                self.tui_dim(
                                    "  /skills delete <id>        — remove a skill directory",
                                );
                                self.tui_dim(
                                    "  /skills reload             — rescan all skill directories",
                                );
                                self.tui_blank();
                            }
                        }
                    }

                    SlashCmd::Subagents => {
                        if self.require_capability(
                            cade_core::capabilities::Capability::Agentic,
                            "/subagents",
                        ) {
                            continue;
                        }
                        let all = discover_all_subagents(&self.cwd);
                        match self.subagent_picker(std::sync::Arc::clone(&self.app), &all).await? {
                            Some(SubagentPickerResult::Run(name)) => {
                                pending_input = Some(format!("run_subagent(subagent_type=\"{name}\", prompt=\"\")"));
                            }
                            Some(SubagentPickerResult::Edit(path)) => {
                                // Drop the TUI temporarily, open $EDITOR, then return
                                let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                                self.tui_sys(format!("  Opening {} in {}...", path.display(), editor));
                                
                                let _ = self.app.lock().expect("lock poisoned").suspend_for(|| {
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
                                    .expect("lock poisoned")
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
                                let mut app = self.app.lock().expect("lock poisoned");
                                let r = app.ask_question(&q)?;
                                app.scroll = 0;
                                let _ = app.draw();
                                matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
                            };
                            if save {
                                let mut settings = self.settings.lock().expect("lock poisoned");
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
                                let mut app = self.app.lock().expect("lock poisoned");
                                let r = app.ask_question(&q)?;
                                app.scroll = 0;
                                let _ = app.draw();
                                matches!(&r, Some(a) if a.as_str().starts_with("Yes"))
                            };
                            if save {
                                let mut settings = self.settings.lock().expect("lock poisoned");
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
                        let merged = self.settings.lock().expect("lock poisoned").merged_hooks();
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
                        let new_theme = if let Some(t) = theme_arg {
                            t.trim().to_string()
                        } else {
                            String::new()
                        };

                        let name = if new_theme.is_empty() {
                            match self.interactive_theme_picker(std::sync::Arc::clone(&self.app)).await? {
                                Some(picked) => picked,
                                None => {
                                    self.tui_dim("  /theme cancelled");
                                    continue;
                                }
                            }
                        } else {
                            new_theme
                        };

                        let (target_theme_colors, found_name) = if name == "dark" {
                            (cade_tui::ThemeColors::dark(), "dark".to_string())
                        } else if name == "light" {
                            (cade_tui::ThemeColors::light(), "light".to_string())
                        } else {
                            let agent_dir = self
                                .settings
                                .lock()
                                .expect("lock poisoned")
                                .global_path()
                                .parent()
                                .unwrap()
                                .to_path_buf();
                            let discovered = cade_core::resources::discover_themes(&self.cwd, &agent_dir);
                            if let Some(t) = discovered.iter().find(|t| t.name == name) {
                                (cade_tui::ThemeColors::from_theme(t), t.name.clone())
                            } else {
                                (cade_tui::ThemeColors::dark(), String::new())
                            }
                        };

                        if found_name.is_empty() {
                            self.tui_err(format!("  ✗ Theme '{name}' not found."));
                        } else {
                            // Apply it dynamically
                            {
                                let mut app = self.app.lock().expect("lock poisoned");
                                app.apply_theme(target_theme_colors);
                            }

                            // Save to settings
                            {
                                let mut s = self.settings.lock().expect("lock poisoned");
                                s.global_settings_mut().theme = Some(found_name.clone());
                                let _ = s.save_global();
                            }

                            self.tui_ok(format!("  ✓ Theme changed to '{found_name}'"));
                        }
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
                                let mut app = self.app.lock().expect("lock poisoned");
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
                                    *self.agent_name.lock().expect("lock poisoned") = name.clone();
                                    self.tui_ok(format!("  ✓ Renamed to: {name}"));
                                }
                                Err(e) => self.tui_err(e.to_string()),
                            }
                        }
                    }

                    SlashCmd::Toolset(arg) => {
                        let old_toolset = *self.current_toolset.lock().expect("lock poisoned");
                        let new_toolset = if let Some(name) = arg.as_deref() {
                            match cade_core::toolsets::Toolset::from_name(name) {
                                Some(t) => t,
                                None => {
                                    self.tui_dim("  Toolsets: default | codex | gemini");
                                    continue;
                                }
                            }
                        } else {
                            self.tui_hdr(format!("  Current toolset: {old_toolset:?}"));
                            self.tui_dim("  /toolset default | codex | gemini");
                            continue;
                        };
                        if new_toolset != old_toolset {
                            *self.current_toolset.lock().expect("lock poisoned") = new_toolset;
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
            let _ = self.app.lock().expect("lock poisoned").commit_streaming();

            // I-01: drain queued messages into pending_input.
            // Follow-up runs after the turn completes naturally.
            // Steering runs after a cancelled turn.
            // Follow-up takes priority — if both are set (edge case), run
            // follow-up first; steering is re-queued on the next iteration.
            let queued_msg = {
                let mut q = self.queued_followup.lock().expect("lock poisoned");
                q.pop_front().map(|msg| (msg, q.len()))
            };

            if let Some((follow, count)) = queued_msg {
                self.app.lock().expect("lock poisoned").queued_count = count;
                pending_input = Some(follow);
            } else if let Some(steer) = self.queued_steering.lock().expect("lock poisoned").take() {
                pending_input = Some(steer);
            }
        }

        // SessionEnd hook (non-blocking)
        self.hooks.session_end(&self.agent_id()).await;

        Ok(())
    }
}