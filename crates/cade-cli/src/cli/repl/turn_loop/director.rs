//! Deep `TurnDirector` module encapsulating REPL turn execution,
//! 16ms redraw cadence, terminal keyboard interception, cancellation,
//! and streaming event synchronization.

use std::io;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::super::Repl;
use super::super::input_driver::{DriverWake, EventPump};
use super::super::slash::{SlashCmd, parse_slash_with_skills};
use super::now_epoch_ms;
use crate::error::Result;
use cade_tui::RenderLine;

/// Admit unowned Enter input or a command explicitly returned by an overlay.
/// Decision Dialog keystrokes never reach this path.
fn admit_busy_input(
    input: String,
    template_names: &[String],
    handle_lua: impl FnOnce(&str, Vec<String>) -> bool,
    followups: &parking_lot::Mutex<std::collections::VecDeque<String>>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<SlashCmd> {
    if let Some(command) = busy_control_command(&input, template_names) {
        let mut words = input.split_whitespace();
        let name = words.next().unwrap_or_default();
        if handle_lua(name, words.map(str::to_owned).collect()) {
            return None;
        }
        if matches!(command, SlashCmd::Exit) {
            // The transport owns cancellation/confirmation. Exit gets priority
            // when that bounded observation ends, ahead of ordinary follow-ups.
            followups.lock().push_front(input);
            cancel.store(true, Ordering::SeqCst);
        }
        return Some(command);
    }
    followups.lock().push_back(input);
    None
}

fn busy_control_command(input: &str, template_names: &[String]) -> Option<SlashCmd> {
    let input = input.trim(); // same admission normalization as the outer REPL
    // Match the outer REPL's template precedence. Defer template expansion to
    // that owner rather than accidentally executing a shadowed builtin here.
    let template = input
        .strip_prefix('/')
        .and_then(|text| text.split(' ').next())
        .is_some_and(|name| template_names.iter().any(|template| template == name));
    if template {
        return None;
    }
    parse_slash_with_skills(input, &[]).filter(|command| {
        matches!(
            command,
            SlashCmd::Approvals
                | SlashCmd::Approve(_)
                | SlashCmd::Deny(_)
                | SlashCmd::Steer(_)
                | SlashCmd::Exit
        )
    })
}

fn launch_busy_control(
    command: SlashCmd,
    client: cade_agent::agent::client::HttpTransport,
    app: std::sync::Arc<parking_lot::Mutex<cade_tui::TuiApp>>,
) {
    if matches!(command, SlashCmd::Exit) {
        return;
    }
    tokio::spawn(async move {
        let result = super::super::commands::dispatch_run_control(&client, command).await;
        super::super::commands::present_run_control(&app, result);
    });
}

fn admit_lua_busy_controls(
    lua: &cade_tui::lua_engine::LuaEngine,
    template_names: &[String],
    followups: &parking_lot::Mutex<std::collections::VecDeque<String>>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Vec<SlashCmd> {
    let mut batch = Vec::new();
    let mut more_controls = false;
    {
        let mut queue = lua
            .command_queue
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queue.retain_mut(|input| {
            if busy_control_command(input, template_names).is_none() {
                return true;
            }
            if batch.len() < cade_tui::lua_engine::LUA_UI_BATCH_SIZE {
                batch.push(std::mem::take(input));
                false
            } else {
                more_controls = true;
                true
            }
        });
    }
    // Lua overrides may enqueue more commands. Never hold the command mutex
    // while invoking Lua, and never consume ordinary/template commands here.
    let controls = batch
        .into_iter()
        .filter_map(|input| {
            admit_busy_input(
                input,
                template_names,
                |command, args| lua.handle_command(command, args),
                followups,
                cancel,
            )
        })
        .collect();
    // Queue producers already notify. Only re-arm for an eligible batch tail;
    // deferred idle commands must not keep the active driver spinning.
    if more_controls {
        lua.work_ready.notify_one();
    }
    controls
}

/// Outcome of an executed REPL agent turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Completed {
        summary: String,
        elapsed_secs: u64,
        token_usage: Option<u64>,
    },
    Cancelled,
    Error(String),
}

/// Terminal hotkeys the turn loop owns while an agent is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnHotkey {
    Cancel,
    ToggleReasoning,
    ClearAndRedraw,
    ToggleSubagentTray,
    ToggleSubagentTrayFocus,
    ToggleExpandAll,
}

/// Event categories the turn loop translates from Crossterm input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnInputEvent {
    Key,
    Resize,
    Mouse,
    Paste,
    Other,
}

fn route_turn_hotkey(key: KeyEvent) -> Option<TurnHotkey> {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c' | 'C'), KeyModifiers::CONTROL) => Some(TurnHotkey::Cancel),
        (KeyCode::Char('t'), KeyModifiers::CONTROL) | (KeyCode::Char('\x14'), _) => {
            Some(TurnHotkey::ToggleReasoning)
        }
        (KeyCode::Char('l' | 'L'), KeyModifiers::CONTROL) | (KeyCode::Char('\x0c'), _) => {
            Some(TurnHotkey::ClearAndRedraw)
        }
        (KeyCode::F(5), _) => Some(TurnHotkey::ToggleSubagentTray),
        (KeyCode::Char('w' | 'W'), KeyModifiers::CONTROL) => {
            Some(TurnHotkey::ToggleSubagentTrayFocus)
        }
        (KeyCode::Char('o' | 'O'), KeyModifiers::CONTROL) | (KeyCode::Char('\x0f'), _) => {
            Some(TurnHotkey::ToggleExpandAll)
        }
        _ => None,
    }
}

fn translate_turn_event(event: &Event) -> TurnInputEvent {
    match event {
        Event::Key(_) => TurnInputEvent::Key,
        Event::Resize(_, _) => TurnInputEvent::Resize,
        Event::Mouse(_) => TurnInputEvent::Mouse,
        Event::Paste(_) => TurnInputEvent::Paste,
        _ => TurnInputEvent::Other,
    }
}

pub(crate) fn resolve_turn_outcome(
    confirmed_cancelled: bool,
    stream_error: Option<String>,
    summary: String,
    elapsed_secs: u64,
    token_usage: Option<u64>,
) -> TurnOutcome {
    if let Some(error) = stream_error {
        TurnOutcome::Error(error)
    } else if confirmed_cancelled {
        TurnOutcome::Cancelled
    } else {
        TurnOutcome::Completed {
            summary,
            elapsed_secs,
            token_usage,
        }
    }
}

fn observed_turn_outcome(
    stream: &Result<Vec<cade_agent::agent::client::CadeMessage>>,
    elapsed_secs: u64,
    token_usage: Option<u64>,
) -> TurnOutcome {
    let summary = stream
        .as_ref()
        .ok()
        .and_then(|messages| {
            messages
                .iter()
                .rfind(|m| m.msg_type() == "assistant_message")
                .and_then(|m| m.assistant_text())
        })
        .unwrap_or_default()
        .to_owned();
    use cade_agent::agent::client::RunOutcome;
    let outcome = match stream {
        Ok(messages) => RunOutcome::from_messages(messages).map_err(|e| e.to_string()),
        Err(error) => Err(error.to_string()),
    };
    match outcome {
        Ok(RunOutcome::Completed) => {
            resolve_turn_outcome(false, None, summary, elapsed_secs, token_usage)
        }
        Ok(RunOutcome::Cancelled) => TurnOutcome::Cancelled,
        Ok(RunOutcome::Failed(error)) | Err(error) => TurnOutcome::Error(error),
    }
}

/// The deep execution engine governing a single REPL conversation turn.
pub struct TurnDirector<'a> {
    repl: &'a mut Repl,
}

impl<'a> TurnDirector<'a> {
    pub fn new(repl: &'a mut Repl) -> Self {
        Self { repl }
    }

    /// Execute the full interactive agent turn: drives the 16ms redraw ticker,
    /// polls terminal event interrupts (F5, Ctrl+W, Ctrl+L, resize, cancel, paste),
    /// consumes the SSE event stream, and returns the structured outcome.
    pub async fn execute_turn(
        &mut self,
        stdout: &mut io::Stdout,
        effective_input: &str,
        turn_start: Instant,
        out_tok_before: u64,
    ) -> Result<TurnOutcome> {
        // -- Thinking animation
        let bar_text = {
            let mut app = self.repl.app.lock();
            app.scroll_to_bottom();
            app.start_thinking("assessing… (Ctrl+c to interrupt · 0s · 0↑)")
        };

        let tick_app = self.repl.app.clone();
        let tick_cancel = self.repl.cancel_turn.clone();
        let tick_tokens = self.repl.session_output_tokens.clone();
        let tick_base = out_tok_before;
        let tick_start = turn_start;
        let tick_bar = bar_text.clone();
        let tick_queued_steering = self.repl.queued_steering.clone();
        let tick_queued_followup = self.repl.queued_followup.clone();
        let tick_modal_close_ms = self.repl.last_modal_close_ms.clone();
        let tick_permissions = self.repl.permissions.clone();
        let tick_client = self.repl.client.clone();
        let template_names: Vec<String> = self
            .repl
            .prompts
            .iter()
            .map(|prompt| prompt.name.clone())
            .collect();
        let pump_lua_work = self.repl.lua_work_pump();
        let lua_wake = self
            .repl
            .app
            .lock()
            .lua_engine
            .as_ref()
            .map(|lua| lua.work_ready.clone())
            .unwrap_or_else(|| std::sync::Arc::new(tokio::sync::Notify::new()));
        let input_owner = super::super::input_driver::TerminalInputGuard::claim(
            self.repl.terminal_driver_active.clone(),
        )?;
        let pending_event = self.repl.pending_terminal_event.clone();

        let tick_handle = tokio::spawn(async move {
            use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
            let _owner = input_owner;
            let mut reader = EventPump::active(EventStream::new(), lua_wake, pending_event);
            let pump_lua_controls = |app: &mut cade_tui::TuiApp| {
                let controls = app
                    .lua_engine
                    .as_ref()
                    .map(|lua| {
                        admit_lua_busy_controls(
                            lua,
                            &template_names,
                            &tick_queued_followup,
                            &tick_cancel,
                        )
                    })
                    .unwrap_or_default();
                for control in controls {
                    if matches!(control, SlashCmd::Exit) {
                        app.set_last_status(Some(
                            "Exit requested; awaiting Run cancellation outcome…".into(),
                        ));
                    }
                    launch_busy_control(control, tick_client.clone(), tick_app.clone());
                }
                app.queued_count = tick_queued_followup.lock().len()
                    + usize::from(tick_queued_steering.lock().is_some());
            };
            loop {
                match reader.next().await {
                    DriverWake::Work => {
                        pump_lua_work();
                        if let Some(mut app) = tick_app.try_lock() {
                            app.pump_lua_ui_events();
                            pump_lua_controls(&mut app);
                            if app.draw_dirty {
                                let _ = app.draw();
                            }
                        }
                    }
                    DriverWake::Tick => {
                        pump_lua_work();
                        let secs = tick_start.elapsed().as_secs();
                        let toks = tick_tokens.load(Ordering::SeqCst).saturating_sub(tick_base);
                        {
                            let cur = tick_bar.lock().clone();
                            if cur.starts_with("assessing") || cur.starts_with("CADE thinking") {
                                *tick_bar.lock() =
                                    format!("assessing… (Ctrl+c to interrupt · {secs}s · {toks}↑)");
                            } else if cur.starts_with('●') {
                                let base = if let Some(idx) = cur.find(" (Ctrl+c") {
                                    &cur[..idx]
                                } else {
                                    &cur
                                };
                                *tick_bar.lock() =
                                    format!("{base} (Ctrl+c to interrupt · {secs}s)");
                            }
                        }
                        if let Some(mut app) = tick_app.try_lock() {
                            app.pump_lua_ui_events();
                            pump_lua_controls(&mut app);
                            if app.draw_dirty
                                || app.thinking.is_some()
                                || app.toast.is_some()
                                || app.slots.requires_tick()
                            {
                                let _ = app.draw();
                            }
                        }
                    }
                    DriverWake::Terminal(Ok(evt)) => {
                        let needs_question_key =
                            matches!(translate_turn_event(&evt), TurnInputEvent::Key)
                                && matches!(
                                    &evt,
                                    Event::Key(KeyEvent {
                                        kind: KeyEventKind::Press,
                                        ..
                                    })
                                );

                        if needs_question_key {
                            if let Event::Key(k) = evt {
                                if let Some(mut app) = tick_app.try_lock() {
                                    let overlay_count = app.overlays.len();
                                    let (owned, action) = app
                                        .dispatch_overlay_event(&Event::Key(k))
                                        .unwrap_or((true, None));
                                    if owned {
                                        if app.overlays.len() < overlay_count {
                                            tick_modal_close_ms
                                                .store(now_epoch_ms(), Ordering::SeqCst);
                                        }
                                        if let Some(Some(command)) = action {
                                            let control = admit_busy_input(
                                                command,
                                                &template_names,
                                                |command, args| {
                                                    app.lua_engine.as_ref().is_some_and(|lua| {
                                                        lua.handle_command(command, args)
                                                    })
                                                },
                                                &tick_queued_followup,
                                                &tick_cancel,
                                            );
                                            if let Some(control) = control {
                                                if matches!(control, SlashCmd::Exit) {
                                                    app.set_last_status(Some("Exit requested; awaiting Run cancellation outcome…".into()));
                                                }
                                                launch_busy_control(
                                                    control,
                                                    tick_client.clone(),
                                                    tick_app.clone(),
                                                );
                                            }
                                            app.queued_count = tick_queued_followup.lock().len()
                                                + usize::from(
                                                    tick_queued_steering.lock().is_some(),
                                                );
                                        }
                                        let _ = app.draw();
                                    } else if app.handle_focused_slot_key(k)
                                        || app.handle_lua_key(k)
                                        || app.handle_scroll_key(k.code, k.modifiers)
                                    {
                                        let _ = app.draw();
                                    } else {
                                        match (k.code, k.modifiers) {
                                            _ if route_turn_hotkey(k)
                                                == Some(TurnHotkey::Cancel) =>
                                            {
                                                app.editor.expand_pastes();
                                                let msg = app.editor.text().trim().to_string();
                                                if !msg.is_empty() {
                                                    *tick_queued_steering.lock() = Some(msg);
                                                    app.editor.clear();
                                                    app.editor.set_cursor_pos(0);
                                                    app.set_last_status(None);
                                                    let _ = app.draw();
                                                }
                                                tick_cancel.store(true, Ordering::SeqCst);
                                                app.set_last_status(Some(
                                                    "Cancelling...".to_string(),
                                                ));
                                                let _ = app.draw();
                                            }
                                            (KeyCode::Esc, _) => {
                                                let esc_now_ms = now_epoch_ms();
                                                let esc_last_close =
                                                    tick_modal_close_ms.load(Ordering::SeqCst);
                                                let esc_post_modal = esc_last_close > 0
                                                    && esc_now_ms.saturating_sub(esc_last_close)
                                                        < 500;
                                                if !esc_post_modal
                                                    && tick_start.elapsed().as_millis() >= 200
                                                    && !app.editor.is_empty()
                                                {
                                                    app.editor.clear();
                                                    app.editor.set_cursor_pos(0);
                                                    app.set_last_status(None);
                                                    let _ = app.draw();
                                                }
                                            }
                                            (KeyCode::Char('v') | KeyCode::Char('V'), m)
                                                if m.contains(KeyModifiers::CONTROL)
                                                    || m.contains(KeyModifiers::ALT) =>
                                            {
                                                if app.paste_from_clipboard() {
                                                    let _ = app.draw();
                                                }
                                            }
                                            _ if route_turn_hotkey(k)
                                                == Some(TurnHotkey::ToggleReasoning) =>
                                            {
                                                if let Some(plan) = &mut app.active_plan {
                                                    plan.is_visible = !plan.is_visible;
                                                    app.draw_dirty = true;
                                                    let _ = app.draw();
                                                }
                                            }
                                            _ if route_turn_hotkey(k)
                                                == Some(TurnHotkey::ClearAndRedraw) =>
                                            {
                                                let _ = app.terminal.clear();
                                                app.draw_dirty = true;
                                                let _ = app.draw();
                                            }
                                            _ if route_turn_hotkey(k)
                                                == Some(TurnHotkey::ToggleExpandAll) =>
                                            {
                                                app.expand_all = !app.expand_all;
                                                app.content_version += 1;
                                                let msg = if app.expand_all {
                                                    "All blocks expanded"
                                                } else {
                                                    "All blocks collapsed"
                                                };
                                                app.show_toast(msg, cade_tui::ToastLevel::Info);
                                                app.draw_dirty = true;
                                                let _ = app.draw();
                                            }
                                            _ if route_turn_hotkey(k)
                                                == Some(TurnHotkey::ToggleSubagentTray) =>
                                            {
                                                app.toggle_subagent_tray();
                                                let _ = app.draw();
                                            }
                                            _ if route_turn_hotkey(k)
                                                == Some(TurnHotkey::ToggleSubagentTrayFocus) =>
                                            {
                                                app.toggle_subagent_tray_focus();
                                                let _ = app.draw();
                                            }
                                            _ if app.subagent_tray.is_visible
                                                && app.subagent_tray.is_focused =>
                                            {
                                                let trackers = app.subagent_trackers.clone();
                                                if app.subagent_tray.handle_key(k, &trackers) {
                                                    let action =
                                                        app.subagent_tray.take_pending_action();
                                                    let _ = app.draw();
                                                    if action != cade_tui::app::subagent_tray::SubagentTrayAction::None {
                                                            match action {
                                                                cade_tui::app::subagent_tray::SubagentTrayAction::None => {}
                                                                cade_tui::app::subagent_tray::SubagentTrayAction::Kill { subagent_id } => {
                                                                     let client = tick_client.clone();
                                                                     let subagent_id_c = subagent_id.clone();
                                                                     app.show_toast(format!("Requesting cancellation for {subagent_id}"), cade_tui::ToastLevel::Info);
                                                                     let _ = app.draw();
                                                                     let app_for_ack = tick_app.clone();
                                                                     tokio::spawn(async move {
                                                                         let result = client
                                                                                .raw_post(
                                                                                    &format!("/subagents/{subagent_id_c}/cancel"),
                                                                                    &serde_json::json!({ "action": "cancel", "id": subagent_id_c }),
                                                                                 )
                                                                                 .await;
                                                                         let mut app = app_for_ack.lock();
                                                                         app.show_toast(match result {
                                                                             Ok(_) => format!("Cancellation requested for {subagent_id_c}"),
                                                                             Err(e) => format!("Could not cancel {subagent_id_c}: {e}"),
                                                                         }, cade_tui::ToastLevel::Info);
                                                                         app.draw_dirty = true;
                                                                     });
                                                                }
                                                                cade_tui::app::subagent_tray::SubagentTrayAction::Steer { subagent_id, message } => {
                                                                    let client = tick_client.clone();
                                                                    let subagent_id_c = subagent_id.clone();
                                                                    let msg_c = message.clone();
                                                                    if let Some(t) = app.subagent_trackers.iter_mut().find(|t| t.task_id == subagent_id) {
                                                                        t.push_output(format!("[STEERING GUIDANCE]: {message}"));
                                                                    }
                                                                    app.show_toast(format!("Steering guidance sent to {subagent_id}"), cade_tui::ToastLevel::Success);
                                                                    let _ = app.draw();
                                                                    tokio::spawn(async move {
                                                                        let body = serde_json::json!({
                                                                            "action": "steer",
                                                                            "id": subagent_id_c,
                                                                            "message": msg_c,
                                                                        });
                                                                        let _ = client.raw_post(&format!("/subagents/{subagent_id_c}/steer"), &body).await;
                                                                    });
                                                                }
                                                                cade_tui::app::subagent_tray::SubagentTrayAction::HotSwapModel { subagent_id, model } => {
                                                                    let client = tick_client.clone();
                                                                    let subagent_id_c = subagent_id.clone();
                                                                    let model_c = model.clone();
                                                                    let ui = tick_app.clone();
                                                                    // The server must acknowledge delivery before we claim the model changed.
                                                                    tokio::spawn(async move {
                                                                        let body = serde_json::json!({
                                                                            "model": model_c,
                                                                        });
                                                                        let result = client.raw_post(&format!("/subagents/{subagent_id_c}/model"), &body).await;
                                                                        let mut app = ui.lock();
                                                                        if result.is_ok()
                                                                            && let Some(t) = app.subagent_trackers.iter_mut().find(|t| t.task_id == subagent_id_c) {
                                                                            t.push_output(format!("[MODEL HOT-SWAP QUEUED]: {model_c}"));
                                                                        }
                                                                        let level = if result.is_ok() { cade_tui::ToastLevel::Info } else { cade_tui::ToastLevel::Error };
                                                                        app.show_toast(match result {
                                                                            Ok(_) => format!("Model hot-swap to {model_c} accepted for {subagent_id_c} (next turn)"),
                                                                            Err(e) => format!("Could not change model for {subagent_id_c}: {e}"),
                                                                        }, level);
                                                                        app.draw_dirty = true;
                                                                    });
                                                                }
                                                                 cade_tui::app::subagent_tray::SubagentTrayAction::PauseResume { subagent_id } => {
                                                                     let client = tick_client.clone();
                                                                     let subagent_id_c = subagent_id.clone();
                                                                     let app_ref = tick_app.clone();
                                                                     tokio::spawn(async move {
                                                                         let result = match client.raw_get(&format!("/subagents/{subagent_id_c}/pause")).await {
                                                                             Ok(state) => {
                                                                                 let action = if state["status"] == "paused" { "resume" } else { "pause" };
                                                                                 client.raw_post(&format!("/subagents/{subagent_id_c}/{action}"), &serde_json::json!({})).await
                                                                             }
                                                                             Err(e) => Err(e),
                                                                         };
                                                                         if let Some(mut app) = app_ref.try_lock() {
                                                                             let message = match result {
                                                                                 Ok(body) => format!("Subagent {subagent_id_c}: {}", body["status"].as_str().unwrap_or("unknown")),
                                                                                 Err(e) => format!("Could not control {subagent_id_c}: {e}"),
                                                                             };
                                                                             app.show_toast(message, cade_tui::ToastLevel::Info);
                                                                             app.draw_dirty = true;
                                                                             let _ = app.draw();
                                                                         }
                                                                     });
                                                                }
                                                            }
                                                        }
                                                }
                                            }
                                            (KeyCode::Tab, _) if app.editor.is_empty() => {
                                                let next_mode = cade_tui::app::cycle_mode(app.mode);
                                                app.update_mode(next_mode);
                                                tick_permissions.set_mode(next_mode);
                                                let _ = app.draw();
                                            }
                                            (KeyCode::BackTab, _) => {
                                                let next_mode =
                                                    cade_tui::app::cycle_mode_back(app.mode);
                                                app.update_mode(next_mode);
                                                tick_permissions.set_mode(next_mode);
                                                let _ = app.draw();
                                            }
                                            (KeyCode::Enter, m) if m == KeyModifiers::CONTROL => {
                                                app.editor.expand_pastes();
                                                let msg = app.editor.text().trim().to_string();
                                                if !msg.is_empty() {
                                                    *tick_queued_steering.lock() = Some(msg);
                                                    app.queued_count =
                                                        tick_queued_followup.lock().len() + 1;
                                                    app.editor.clear();
                                                    app.editor.set_cursor_pos(0);
                                                    app.set_last_status(None);
                                                    let _ = app.draw();
                                                    tick_cancel.store(true, Ordering::SeqCst);
                                                }
                                            }
                                            (KeyCode::Enter, _) => {
                                                app.editor.expand_pastes();
                                                let msg = app.editor.text().trim().to_string();
                                                if !msg.is_empty() {
                                                    let control = admit_busy_input(
                                                        msg,
                                                        &template_names,
                                                        |command, args| {
                                                            app.lua_engine.as_ref().is_some_and(
                                                                |lua| {
                                                                    lua.handle_command(
                                                                        command, args,
                                                                    )
                                                                },
                                                            )
                                                        },
                                                        &tick_queued_followup,
                                                        &tick_cancel,
                                                    );
                                                    let exiting =
                                                        matches!(control, Some(SlashCmd::Exit));
                                                    if let Some(control) = control {
                                                        launch_busy_control(
                                                            control,
                                                            tick_client.clone(),
                                                            tick_app.clone(),
                                                        );
                                                    }
                                                    let count = tick_queued_followup.lock().len()
                                                        + usize::from(
                                                            tick_queued_steering.lock().is_some(),
                                                        );
                                                    app.queued_count = count;
                                                    app.editor.clear();
                                                    app.editor.set_cursor_pos(0);
                                                    app.set_last_status(exiting.then(|| "Exit requested; awaiting Run cancellation outcome…".into()));
                                                    let _ = app.draw();
                                                }
                                            }
                                            (KeyCode::Char(_), _)
                                            | (KeyCode::Backspace, _)
                                            | (KeyCode::Delete, _)
                                            | (KeyCode::Left, _)
                                            | (KeyCode::Right, _)
                                            | (KeyCode::Home, _)
                                            | (KeyCode::End, _)
                                            | (KeyCode::Up, _)
                                            | (KeyCode::Down, _) => {
                                                let w = app.last_input_width;
                                                app.editor.handle_input(k, w);
                                                app.refresh_autocomplete();
                                                let _ = app.draw();
                                            }
                                            _ => {}
                                        }
                                    }
                                } else {
                                    reader.defer(Event::Key(k));
                                }
                            }
                        } else if let Some(mut app) = tick_app.try_lock() {
                            let overlay_count = app.overlays.len();
                            let (owned, _) =
                                app.dispatch_overlay_event(&evt).unwrap_or((true, None));
                            if owned {
                                if app.overlays.len() < overlay_count {
                                    tick_modal_close_ms.store(now_epoch_ms(), Ordering::SeqCst);
                                }
                                let _ = app.draw();
                            } else {
                                match evt {
                                    Event::Mouse(m) => {
                                        let _ = app.handle_message_area_mouse_event(m);
                                    }
                                    Event::Paste(text) => {
                                        app.handle_bracketed_paste_text(&text);
                                        let _ = app.draw();
                                    }
                                    Event::Resize(_w, _h) => {
                                        let _ = app.handle_resize();
                                    }
                                    Event::FocusGained => app.has_focus = true,
                                    Event::FocusLost => app.has_focus = false,
                                    _ => {}
                                }
                            }
                        } else {
                            reader.defer(evt);
                        }
                    }
                    DriverWake::Terminal(Err(error)) => {
                        tracing::warn!("Terminal input failed during active turn: {error}");
                        tick_cancel.store(true, Ordering::SeqCst);
                        break;
                    }
                    DriverWake::Closed => {
                        tick_cancel.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });

        let stream_res = self
            .repl
            .stream_turn(
                stdout,
                effective_input,
                false,
                "",
                "",
                "",
                false,
                None,
                Some(bar_text),
            )
            .await;

        // Blank line after every agent turn for visual block separation
        let _ = self.repl.app.lock().push(RenderLine::Blank);

        // Stop thinking animation
        tick_handle.abort();
        let _ = tick_handle.await;
        self.repl.cancel_turn.store(false, Ordering::SeqCst);
        let secs = self.repl.app.lock().stop_thinking();
        {
            let mut stats = self.repl.session_stats.lock();
            stats.agent_active_ms += turn_start.elapsed().as_millis() as u64;
        }

        {
            let mut app = self.repl.app.lock();
            app.stop_thinking();
            if app.follow {
                app.scroll_to_bottom();
            }
            let _ = app.draw();
        }

        let token_usage = Some(self.repl.session_output_tokens.load(Ordering::SeqCst));
        let outcome = observed_turn_outcome(&stream_res, secs, token_usage);
        {
            let mut app = self.repl.app.lock();
            match &outcome {
                TurnOutcome::Completed { .. } => app.set_last_status(None),
                TurnOutcome::Cancelled => {
                    let _ = app.push(RenderLine::SystemMsg("Run cancellation confirmed.".into()));
                    app.set_last_status(Some("Run cancelled".into()));
                }
                TurnOutcome::Error(error) => app.set_last_status(Some(error.clone())),
            }
            let _ = app.draw();
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};

    #[tokio::test]
    async fn finalization_storage_failure_reaches_director_as_incomplete_not_completed() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut socket).await;
            let body = "data: {\"message_type\":\"error\",\"code\":\"run_finalization_failed\",\"terminal_status_persisted\":false,\"run_id\":\"r-cli\",\"error\":\"disk full\"}\n\ndata: [DONE]\n\n";
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            std::future::pending::<()>().await;
        });
        let client = cade_agent::agent::client::HttpTransport::new(
            format!("http://{address}"),
            String::new(),
        )
        .unwrap();
        let observed = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.start_run("agent", "hello", None, |_| {}),
        )
        .await;
        peer.abort();
        let observed = observed
            .expect("director must not wait for a terminal write that failed")
            .map_err(|e| crate::Error::custom(e.to_string()));
        assert!(
            matches!(observed_turn_outcome(&observed, 1, None), TurnOutcome::Error(error) if error.contains("incomplete") && error.contains("r-cli") && error.contains("disk full"))
        );
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;
        let mut bytes = Vec::new();
        let mut buf = [0; 4096];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|n| n.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    return String::from_utf8(bytes).unwrap();
                }
            }
        }
    }

    #[tokio::test]
    async fn busy_controls_dispatch_over_http_while_the_owned_run_is_waiting() {
        use std::sync::{Arc, atomic::AtomicBool};
        use tokio::io::AsyncWriteExt;
        for (input, path, expected_body, rejected) in [
            ("/approvals", "/v1/approvals", None, false),
            (
                "/approve ap-1",
                "/v1/approvals/ap-1/action",
                Some(serde_json::json!({"action":"approve"})),
                false,
            ),
            (
                "/deny ap-1 try another way",
                "/v1/approvals/ap-1/action",
                Some(serde_json::json!({"action":"deny","feedback":"try another way"})),
                false,
            ),
            (
                "/steer child-1 look here",
                "/v1/subagents/child-1/steer",
                Some(serde_json::json!({"message":"look here"})),
                false,
            ),
            (
                "/approve stale",
                "/v1/approvals/stale/action",
                Some(serde_json::json!({"action":"approve"})),
                true,
            ),
            ("/exit", "/v1/runs/r-busy/cancel", None, false),
            ("/quit", "/v1/runs/r-busy/cancel", None, false),
        ] {
            for from_lua in [false, true] {
                let expected_body = expected_body.clone();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let exiting = matches!(parse_slash_with_skills(input, &[]), Some(SlashCmd::Exit));
                let detached = input == "/quit";
                let terminal = if exiting { "cancelled" } else { "done" };
                let peer = tokio::spawn(async move {
                    let (mut live, _) = listener.accept().await.unwrap();
                    assert!(
                        read_request(&mut live)
                            .await
                            .starts_with("POST /v1/agents/agent/run ")
                    );
                    live.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"message_type\":\"stream_start\",\"run_id\":\"r-busy\",\"seq_id\":0}\n\n").await.unwrap();
                    let (mut control, _) = listener.accept().await.unwrap();
                    let request = read_request(&mut control).await;
                    assert_eq!(request.split_whitespace().nth(1), Some(path));
                    if let Some(body) = expected_body {
                        assert_eq!(
                            serde_json::from_str::<serde_json::Value>(
                                request.split_once("\r\n\r\n").unwrap().1
                            )
                            .unwrap(),
                            body
                        );
                    }
                    if detached {
                        std::future::pending::<()>().await;
                    }
                    let body = if rejected {
                        "{\"detail\":\"decision is stale\"}"
                    } else {
                        "{\"status\":\"cancelling\",\"approvals\":[]}"
                    };
                    let code = if rejected { "409 Conflict" } else { "200 OK" };
                    control.write_all(format!("HTTP/1.1 {code}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                    live.write_all(format!("data: {{\"message_type\":\"run_done\",\"run_id\":\"r-busy\",\"seq_id\":1,\"status\":\"{terminal}\"}}\n\n").as_bytes()).await.unwrap();
                });
                let client = Arc::new(
                    cade_agent::agent::client::HttpTransport::new(
                        format!("http://{address}"),
                        String::new(),
                    )
                    .unwrap(),
                );
                let cancel = Arc::new(AtomicBool::new(false));
                let started = Arc::new(tokio::sync::Notify::new());
                let run = {
                    let (client, cancel, started) =
                        (client.clone(), cancel.clone(), started.clone());
                    tokio::spawn(async move {
                        client
                            .start_run_cancellable(
                                "agent",
                                "hello",
                                None,
                                |event| {
                                    if event.msg_type() == "stream_start" {
                                        started.notify_one();
                                    }
                                },
                                Some(&cancel),
                            )
                            .await
                    })
                };
                tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
                    .await
                    .unwrap();
                let followups = parking_lot::Mutex::new(std::collections::VecDeque::from([
                    "later prompt".into(),
                ]));
                let command = if from_lua {
                    let lua = cade_tui::lua_engine::LuaEngine::new().unwrap();
                    let mut pump = EventPump::active(
                        futures::stream::pending(),
                        lua.work_ready.clone(),
                        Default::default(),
                    );
                    lua.lua.load("local input = ...; CADE.execute_slash_command('/help'); CADE.execute_slash_command(input); CADE.execute_slash_command('/info')").call::<()>(input).unwrap();
                    tokio::time::timeout(std::time::Duration::from_secs(1), async {
                        loop {
                            if matches!(pump.next().await, DriverWake::Work) {
                                break;
                            }
                        }
                    })
                    .await
                    .expect("queued Lua work must wake the active driver without a key");
                    let mut controls = admit_lua_busy_controls(&lua, &[], &followups, &cancel);
                    assert_eq!(
                        controls.len(),
                        1,
                        "Lua busy control must pass the active wake admission"
                    );
                    assert_eq!(
                        lua.command_queue
                            .lock()
                            .unwrap()
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>(),
                        ["/help", "/info"]
                    );
                    controls.pop().unwrap()
                } else {
                    admit_busy_input(input.into(), &[], |_, _| false, &followups, &cancel)
                        .expect("busy control must not wait behind its Run")
                };
                if matches!(command, SlashCmd::Exit) {
                    assert!(cancel.load(Ordering::SeqCst));
                    assert_eq!(followups.lock().front().map(String::as_str), Some(input));
                } else {
                    let result =
                        super::super::super::commands::dispatch_run_control(&client, command).await;
                    assert_eq!(result.is_err(), rejected);
                    if rejected {
                        assert!(
                            result
                                .err()
                                .unwrap()
                                .to_string()
                                .contains("decision is stale")
                        );
                    }
                    assert_eq!(followups.lock().len(), 1);
                }
                let observed = tokio::time::timeout(std::time::Duration::from_secs(3), run)
                    .await
                    .unwrap()
                    .unwrap()
                    .map_err(|e| crate::Error::custom(e.to_string()));
                if detached {
                    assert!(
                        matches!(observed_turn_outcome(&observed, 1, None), TurnOutcome::Error(error) if error.contains("unconfirmed") && error.contains("r-busy"))
                    );
                    peer.abort();
                } else if exiting {
                    assert_eq!(
                        observed_turn_outcome(&observed, 1, None),
                        TurnOutcome::Cancelled
                    );
                } else {
                    assert!(matches!(
                        observed_turn_outcome(&observed, 1, None),
                        TurnOutcome::Completed { .. }
                    ));
                }
                if !detached {
                    peer.await.unwrap();
                }
            }
        }
    }

    #[tokio::test]
    async fn lua_busy_queue_preserves_overrides_and_does_not_spin_on_idle_commands() {
        let lua = cade_tui::lua_engine::LuaEngine::new().unwrap();
        let mut pump = EventPump::active(
            futures::stream::pending(),
            lua.work_ready.clone(),
            Default::default(),
        );
        lua.lua
            .load(
                r#"
            CADE._commands['/exit'] = function(args)
                CADE_UI.footer = 'Lua override'; CADE.execute_slash_command('/help')
            end
            CADE.execute_slash_command('/info')
            CADE.execute_slash_command('  /approve template-owned  ')
            CADE.execute_slash_command('/exit')
        "#,
            )
            .exec()
            .unwrap();
        let followups = parking_lot::Mutex::new(std::collections::VecDeque::new());
        let cancel = std::sync::atomic::AtomicBool::new(false);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if matches!(pump.next().await, DriverWake::Work) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(admit_lua_busy_controls(&lua, &["approve".into()], &followups, &cancel).is_empty());
        assert_eq!(lua.get_footer_text().as_deref(), Some("Lua override"));
        assert!(!cancel.load(Ordering::SeqCst));
        assert!(followups.lock().is_empty());
        assert_eq!(
            lua.command_queue
                .lock()
                .unwrap()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["/info", "  /approve template-owned  ", "/help"]
        );
        // The override legitimately enqueued /help, producing one coalesced
        // wake. Deferred ordinary commands must not re-notify themselves.
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if matches!(pump.next().await, DriverWake::Work) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(admit_lua_busy_controls(&lua, &["approve".into()], &followups, &cancel).is_empty());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(60), async {
                loop {
                    if matches!(pump.next().await, DriverWake::Work) {
                        break;
                    }
                }
            })
            .await
            .is_err(),
            "idle-only commands must not cause busy Work wakes"
        );
    }

    #[tokio::test]
    async fn lua_busy_batches_rearm_only_for_remaining_controls() {
        let lua = cade_tui::lua_engine::LuaEngine::new().unwrap();
        let mut pump = EventPump::active(
            futures::stream::pending(),
            lua.work_ready.clone(),
            Default::default(),
        );
        let count = cade_tui::lua_engine::LUA_UI_BATCH_SIZE + 1;
        lua.lua.load("local count=...; CADE.execute_slash_command('/help'); for i=1,count do CADE.execute_slash_command('/approve ap-'..i) end").call::<()>(count).unwrap();
        let followups = Default::default();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut received = Vec::new();
        for size in [count - 1, 1] {
            tokio::time::timeout(std::time::Duration::from_secs(1), async {
                loop {
                    if matches!(pump.next().await, DriverWake::Work) {
                        break;
                    }
                }
            })
            .await
            .unwrap();
            let controls = admit_lua_busy_controls(&lua, &[], &followups, &cancel);
            assert_eq!(controls.len(), size);
            received.extend(controls);
        }
        assert_eq!(
            received,
            (1..=count)
                .map(|i| SlashCmd::Approve(format!("ap-{i}")))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            lua.command_queue
                .lock()
                .unwrap()
                .front()
                .map(String::as_str),
            Some("/help")
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(60), async {
                loop {
                    if matches!(pump.next().await, DriverWake::Work) {
                        break;
                    }
                }
            })
            .await
            .is_err()
        );
    }

    #[tokio::test]
    #[ignore = "requires tty; exercised with script(1)"]
    async fn lua_busy_control_preserves_modal_and_pending_terminal_input() {
        let mut app = cade_tui::TuiApp::new(
            cade_core::permissions::PermissionMode::Default,
            "test".into(),
            "test".into(),
            None,
        );
        app.editor.set_text("/exit".into());
        let _answer = app
            .ask_approval_async(
                "ap-modal".into(),
                cade_tui::question::Question {
                    header: "Permission".into(),
                    text: "Allow?".into(),
                    options: vec![],
                    multi_select: false,
                    allow_other: true,
                    progress: None,
                },
            )
            .unwrap();
        let lua = app.lua_engine.as_ref().unwrap();
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let pending = std::sync::Arc::new(parking_lot::Mutex::new(Some(enter.clone())));
        let mut pump = EventPump::active(
            futures::stream::pending(),
            lua.work_ready.clone(),
            pending.clone(),
        );
        lua.lua
            .load("CADE.execute_slash_command('/approve ap-modal')")
            .exec()
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if matches!(pump.next().await, DriverWake::Work) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let followups = Default::default();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert_eq!(
            admit_lua_busy_controls(lua, &[], &followups, &cancel),
            [SlashCmd::Approve("ap-modal".into())]
        );
        assert_eq!(*pending.lock(), Some(enter.clone()));
        assert_eq!(app.overlays.len(), 1);
        let (owned, action) = app.dispatch_overlay_event(&enter).unwrap();
        assert!(owned && action.is_none());
        assert_eq!(app.editor.text(), "/exit");
        assert!(!cancel.load(Ordering::SeqCst));
        assert!(followups.lock().is_empty());
    }

    #[test]
    fn busy_control_admission_preserves_template_and_real_lua_overrides() {
        let lua = cade_tui::lua_engine::LuaEngine::new().unwrap();
        lua.lua
            .load("CADE._commands['/exit'] = function(args) CADE_UI.footer = 'override ran' end")
            .exec()
            .unwrap();
        let followups = parking_lot::Mutex::new(std::collections::VecDeque::new());
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert!(
            admit_busy_input(
                "/exit".into(),
                &["exit".into()],
                |cmd, args| lua.handle_command(cmd, args),
                &followups,
                &cancel
            )
            .is_none()
        );
        assert_ne!(lua.get_footer_text().as_deref(), Some("override ran"));
        assert_eq!(followups.lock().pop_front().as_deref(), Some("/exit"));
        assert!(
            admit_busy_input(
                "/exit".into(),
                &[],
                |cmd, args| lua.handle_command(cmd, args),
                &followups,
                &cancel
            )
            .is_none()
        );
        assert_eq!(lua.get_footer_text().as_deref(), Some("override ran"));
        assert!(followups.lock().is_empty());
        assert!(!cancel.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn http_run_outcomes_reach_the_director_without_a_local_cancel_flag() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for status in ["done", "error", "cancelled"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let body = format!(
                "data: {{\"message_type\":\"run_done\",\"status\":\"{status}\",\"run_id\":\"r-cli\",\"seq_id\":0}}\n\n"
            );
            tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0; 4096];
                let _ = socket.read(&mut buf).await.unwrap();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let client = cade_agent::agent::client::HttpTransport::new(
                format!("http://{address}"),
                String::new(),
            )
            .unwrap();
            let observed = client
                .start_run("agent", "hello", None, |_| {})
                .await
                .map_err(|e| crate::Error::custom(e.to_string()));
            let outcome = observed_turn_outcome(&observed, 1, None);
            match status {
                "done" => assert!(matches!(outcome, TurnOutcome::Completed { .. })),
                "error" => assert!(matches!(outcome, TurnOutcome::Error(_)), "{outcome:?}"),
                "cancelled" => assert_eq!(outcome, TurnOutcome::Cancelled),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn local_cancellation_cannot_overwrite_an_unconfirmed_transport_error() {
        let observed = Err(crate::Error::custom(
            "Detached from Run r-cli; cancellation unconfirmed",
        ));
        assert!(matches!(
            observed_turn_outcome(&observed, 1, None),
            TurnOutcome::Error(_)
        ));
        assert!(matches!(
            observed_turn_outcome(&Ok(vec![]), 1, None),
            TurnOutcome::Error(_)
        ));
    }

    #[test]
    fn resolves_completed_cancelled_and_error_turns() {
        let completed = resolve_turn_outcome(
            false,
            None,
            "Finished turn successfully".to_string(),
            5,
            Some(150),
        );
        assert_eq!(
            completed,
            TurnOutcome::Completed {
                summary: "Finished turn successfully".to_string(),
                elapsed_secs: 5,
                token_usage: Some(150),
            }
        );

        let cancelled = resolve_turn_outcome(true, None, String::new(), 5, Some(150));
        assert_eq!(cancelled, TurnOutcome::Cancelled);

        let error = resolve_turn_outcome(
            false,
            Some("Network timeout".to_string()),
            String::new(),
            5,
            None,
        );
        assert_eq!(error, TurnOutcome::Error("Network timeout".to_string()));
    }

    #[test]
    fn translates_terminal_events_without_terminal_access() {
        let cases = [
            (
                Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
                TurnInputEvent::Key,
            ),
            (Event::Resize(120, 40), TurnInputEvent::Resize),
            (
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Moved,
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                }),
                TurnInputEvent::Mouse,
            ),
            (Event::Paste("steer".to_string()), TurnInputEvent::Paste),
        ];

        for (event, expected) in cases {
            assert_eq!(translate_turn_event(&event), expected);
        }
    }

    #[test]
    fn routes_active_turn_hotkeys_without_terminal_access() {
        let cases = [
            (
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                TurnHotkey::Cancel,
            ),
            (
                KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
                TurnHotkey::ToggleReasoning,
            ),
            (
                KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
                TurnHotkey::ClearAndRedraw,
            ),
            (
                KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
                TurnHotkey::ToggleSubagentTray,
            ),
            (
                KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                TurnHotkey::ToggleSubagentTrayFocus,
            ),
            (
                KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
                TurnHotkey::ToggleExpandAll,
            ),
            (
                KeyEvent::new(KeyCode::Char('O'), KeyModifiers::CONTROL),
                TurnHotkey::ToggleExpandAll,
            ),
            (
                KeyEvent::new(KeyCode::Char('\x0f'), KeyModifiers::NONE),
                TurnHotkey::ToggleExpandAll,
            ),
        ];

        for (key, expected) in cases {
            assert_eq!(route_turn_hotkey(key), Some(expected));
        }
        assert_eq!(
            route_turn_hotkey(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            None,
        );
    }

    #[test]
    fn test_subagent_events_track_lifecycle() {
        use cade_tui::subagent_tracker::{SubagentStatus, SubagentTracker};

        let mut trackers: Vec<SubagentTracker> = Vec::new();

        // 1. subagent_started
        let mut tracker = SubagentTracker::new("sub-test-123".to_string(), "scout".to_string());
        tracker.current_tool = Some("init".to_string());
        tracker.push_output("[TASK]: check git".to_string());
        trackers.push(tracker);

        assert_eq!(trackers.len(), 1);
        assert_eq!(trackers[0].task_id, "sub-test-123");
        assert_eq!(trackers[0].mode, "scout");
        assert_eq!(trackers[0].current_tool, Some("init".to_string()));

        // 2. subagent_tool_start
        if let Some(t) = trackers.iter_mut().find(|t| t.task_id == "sub-test-123") {
            t.current_tool = Some("bash".to_string());
            t.tool_calls += 1;
            t.push_output("▶ tool: bash".to_string());
        }
        assert_eq!(trackers[0].tool_calls, 1);
        assert_eq!(trackers[0].current_tool, Some("bash".to_string()));

        // 3. subagent_complete
        if let Some(t) = trackers.iter_mut().find(|t| t.task_id == "sub-test-123") {
            t.current_tool = None;
            t.push_output("[RESULT]: main branch clean".to_string());
            t.status = SubagentStatus::Completed {
                finished_at: std::time::Instant::now(),
            };
        }
        assert_eq!(trackers[0].current_tool, None);
        assert!(matches!(
            trackers[0].status,
            SubagentStatus::Completed { .. }
        ));
    }

    #[test]
    fn test_resolve_turn_outcome_matrix() {
        // 1. Clean completion
        let outcome = resolve_turn_outcome(false, None, "Turn summary".to_string(), 5, Some(120));
        assert_eq!(
            outcome,
            TurnOutcome::Completed {
                summary: "Turn summary".to_string(),
                elapsed_secs: 5,
                token_usage: Some(120),
            }
        );

        // 2. A cancellation request cannot erase an observation error.
        let cancelled_with_err = resolve_turn_outcome(
            true,
            Some("stream broken".to_string()),
            "Partial summary".to_string(),
            2,
            None,
        );
        assert_eq!(
            cancelled_with_err,
            TurnOutcome::Error("stream broken".into())
        );

        // 3. Error without cancellation returns TurnOutcome::Error
        let error_outcome = resolve_turn_outcome(
            false,
            Some("connection refused".to_string()),
            "".to_string(),
            1,
            None,
        );
        assert_eq!(
            error_outcome,
            TurnOutcome::Error("connection refused".to_string())
        );
    }

    #[test]
    fn test_turn_loop_sse_message_flow() {
        use cade_agent::agent::client::CadeMessage;

        // 1. System notice construction and type verification
        let notice = CadeMessage::system_notice("Reconnecting to stream...");
        assert_eq!(notice.msg_type(), "system_notice");
        assert_eq!(notice.data["message"], "Reconnecting to stream...");

        // 2. Tool call message parsing
        let tool_msg = serde_json::from_value::<CadeMessage>(serde_json::json!({
            "message_type": "tool_call_message",
            "tool_call": {
                "id": "call_123",
                "name": "bash",
                "arguments": "{\"command\":\"cargo check\"}"
            }
        }))
        .expect("parse tool call message");
        assert_eq!(tool_msg.msg_type(), "tool_call_message");
        let (id, name, args) = tool_msg.as_tool_call().expect("as_tool_call");
        assert_eq!(id, "call_123");
        assert_eq!(name, "bash");
        assert_eq!(args["command"], "cargo check");

        // 3. Usage statistics payload
        let usage_msg = serde_json::from_value::<CadeMessage>(serde_json::json!({
            "message_type": "usage_statistics",
            "input_tokens": 150,
            "output_tokens": 80,
            "cache_read_tokens": 30,
            "cache_write_tokens": 0,
            "model": "claude-sonnet-4"
        }))
        .expect("parse usage message");
        assert_eq!(usage_msg.msg_type(), "usage_statistics");
        assert_eq!(usage_msg.data["input_tokens"], 150);
        assert_eq!(usage_msg.data["output_tokens"], 80);

        // 4. Clean turn outcome
        let outcome =
            resolve_turn_outcome(false, None, "All steps verified".to_string(), 3, Some(230));
        assert_eq!(
            outcome,
            TurnOutcome::Completed {
                summary: "All steps verified".to_string(),
                elapsed_secs: 3,
                token_usage: Some(230),
            }
        );
    }
}
