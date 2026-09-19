//! Deep `TurnDirector` module encapsulating REPL turn execution,
//! 16ms redraw cadence, terminal keyboard interception, cancellation,
//! and streaming event synchronization.

use std::io;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::super::Repl;
use super::now_epoch_ms;
use crate::error::Result;
use cade_tui::RenderLine;

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

fn resolve_turn_outcome(
    is_cancelled: bool,
    stream_error: Option<String>,
    summary: String,
    elapsed_secs: u64,
    token_usage: Option<u64>,
) -> TurnOutcome {
    if is_cancelled {
        TurnOutcome::Cancelled
    } else if let Some(error) = stream_error {
        TurnOutcome::Error(error)
    } else {
        TurnOutcome::Completed {
            summary,
            elapsed_secs,
            token_usage,
        }
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
        let tick_cancellations = self.repl.subagent_cancellations.clone();
        let tick_client = self.repl.client.clone();

        let tick_handle = tokio::spawn(async move {
            use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
            use futures::StreamExt;
            let mut reader = EventStream::new();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(16)) => {
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
                                *tick_bar.lock() = format!("{base} (Ctrl+c to interrupt · {secs}s)");
                            }
                        }
                        if let Some(mut app) = tick_app.try_lock()
                            && (app.draw_dirty || app.thinking.is_some() || app.toast.is_some()) {
                                let _ = app.draw();
                            }
                    }
                    Some(Ok(evt)) = reader.next() => {
                        let needs_question_key = matches!(translate_turn_event(&evt), TurnInputEvent::Key)
                            && matches!(&evt, Event::Key(KeyEvent { kind: KeyEventKind::Press, .. }));

                        if needs_question_key {
                            if let Event::Key(k) = evt {
                                loop {
                                    if let Some(mut app) = tick_app.try_lock() {
                                        let has_async_overlay = app.overlays.last().is_some_and(|o| o.id() == "active_question" || o.id() == "password");
                                        if has_async_overlay {
                                            if let Some(top) = app.overlays.last_mut() {
                                                let res = top.handle_input(k);
                                                if matches!(res, cade_tui::overlay_component::OverlayInputResult::Dismiss) {
                                                    app.overlays.pop();
                                                    app.draw_dirty = true;
                                                    let _ = app.draw();
                                                } else if matches!(res, cade_tui::overlay_component::OverlayInputResult::Consumed) {
                                                    app.draw_dirty = true;
                                                    let _ = app.draw();
                                                }
                                            }
                                        } else {
                                            match (k.code, k.modifiers) {
                                                _ if route_turn_hotkey(k) == Some(TurnHotkey::Cancel) => {
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
                                                    app.set_last_status(Some("Cancelling...".to_string()));
                                                    let _ = app.draw();
                                                }
                                                (KeyCode::Esc, _) => {
                                                    let esc_now_ms = now_epoch_ms();
                                                    let esc_last_close = tick_modal_close_ms.load(Ordering::SeqCst);
                                                    let esc_post_modal = esc_last_close > 0 && esc_now_ms.saturating_sub(esc_last_close) < 500;
                                                    if !esc_post_modal && tick_start.elapsed().as_millis() >= 200 && !app.editor.is_empty() {
                                                        app.editor.clear();
                                                        app.editor.set_cursor_pos(0);
                                                        app.set_last_status(None);
                                                        let _ = app.draw();
                                                    }
                                                }
                                                (KeyCode::Char('v') | KeyCode::Char('V'), m)
                                                    if m.contains(KeyModifiers::CONTROL) || m.contains(KeyModifiers::ALT) =>
                                                {
                                                    if app.paste_from_clipboard() {
                                                        let _ = app.draw();
                                                    }
                                                }
                                                _ if route_turn_hotkey(k) == Some(TurnHotkey::ToggleReasoning) => {
                                                    if let Some(plan) = &mut app.active_plan {
                                                        plan.is_visible = !plan.is_visible;
                                                        app.draw_dirty = true;
                                                        let _ = app.draw();
                                                    }
                                                }
                                                _ if route_turn_hotkey(k) == Some(TurnHotkey::ClearAndRedraw) => {
                                                    let _ = app.terminal.clear();
                                                    app.draw_dirty = true;
                                                    let _ = app.draw();
                                                }
                                                _ if route_turn_hotkey(k) == Some(TurnHotkey::ToggleExpandAll) => {
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
                                                _ if route_turn_hotkey(k) == Some(TurnHotkey::ToggleSubagentTray) => {
                                                    app.toggle_subagent_tray();
                                                    let _ = app.draw();
                                                }
                                                _ if route_turn_hotkey(k) == Some(TurnHotkey::ToggleSubagentTrayFocus) => {
                                                    app.toggle_subagent_tray_focus();
                                                    let _ = app.draw();
                                                }
                                                _ if app.subagent_tray.is_visible && app.subagent_tray.is_focused => {
                                                    let trackers = app.subagent_trackers.clone();
                                                    if app.subagent_tray.handle_key(k, &trackers) {
                                                        let action = app.subagent_tray.take_pending_action();
                                                        let _ = app.draw();
                                                        if action != cade_tui::app::subagent_tray::SubagentTrayAction::None {
                                                            match action {
                                                                cade_tui::app::subagent_tray::SubagentTrayAction::None => {}
                                                                cade_tui::app::subagent_tray::SubagentTrayAction::Kill { subagent_id } => {
                                                                    let cancellations = tick_cancellations.clone();
                                                                    let client = tick_client.clone();
                                                                    let subagent_id_c = subagent_id.clone();
                                                                    if let Some(t) = app.subagent_trackers.iter_mut().find(|t| t.task_id == subagent_id) {
                                                                        t.status = cade_tui::subagent_tracker::SubagentStatus::Failed {
                                                                            finished_at: std::time::Instant::now(),
                                                                            error: "Killed from Control Tray".into(),
                                                                        };
                                                                    }
                                                                    app.show_toast(format!("Subagent {subagent_id} killed"), cade_tui::ToastLevel::Info);
                                                                    let _ = app.draw();
                                                                    tokio::spawn(async move {
                                                                        let tx_opt = {
                                                                            let map = cancellations.lock().await;
                                                                            map.get(&subagent_id_c).cloned()
                                                                        };
                                                                        if let Some(tx) = tx_opt {
                                                                            let _ = tx.send(()).await;
                                                                        } else {
                                                                            let _ = client
                                                                                .raw_post(
                                                                                    &format!("/subagents/{subagent_id_c}/cancel"),
                                                                                    &serde_json::json!({ "action": "cancel", "id": subagent_id_c }),
                                                                                )
                                                                                .await;
                                                                        }
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
                                                                    if let Some(t) = app.subagent_trackers.iter_mut().find(|t| t.task_id == subagent_id) {
                                                                        t.push_output(format!("[MODEL HOT-SWAP]: {model}"));
                                                                    }
                                                                    app.show_toast(format!("Model hot-swap to {model} for {subagent_id}"), cade_tui::ToastLevel::Info);
                                                                    let _ = app.draw();
                                                                    tokio::spawn(async move {
                                                                        let body = serde_json::json!({
                                                                            "action": "hot_swap",
                                                                            "id": subagent_id_c,
                                                                            "model": model_c,
                                                                        });
                                                                        let _ = client.raw_post(&format!("/subagents/{subagent_id_c}/model"), &body).await;
                                                                    });
                                                                }
                                                                cade_tui::app::subagent_tray::SubagentTrayAction::PauseResume { subagent_id } => {
                                                                    let client = tick_client.clone();
                                                                    let subagent_id_c = subagent_id.clone();
                                                                    app.show_toast(format!("Pause/Resume signal sent to {subagent_id}"), cade_tui::ToastLevel::Info);
                                                                    let _ = app.draw();
                                                                    tokio::spawn(async move {
                                                                        let body = serde_json::json!({
                                                                            "action": "pause_resume",
                                                                            "id": subagent_id_c,
                                                                        });
                                                                        let _ = client.raw_post(&format!("/subagents/{subagent_id_c}/pause"), &body).await;
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
                                                    let next_mode = cade_tui::app::cycle_mode_back(app.mode);
                                                    app.update_mode(next_mode);
                                                    tick_permissions.set_mode(next_mode);
                                                    let _ = app.draw();
                                                }
                                                (KeyCode::Enter, m) if m == KeyModifiers::CONTROL => {
                                                    app.editor.expand_pastes();
                                                    let msg = app.editor.text().trim().to_string();
                                                    if !msg.is_empty() {
                                                        *tick_queued_steering.lock() = Some(msg);
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
                                                        tick_queued_followup.lock().push_back(msg);
                                                        app.editor.clear();
                                                        app.editor.set_cursor_pos(0);
                                                        app.set_last_status(None);
                                                        let _ = app.draw();
                                                    }
                                                }
                                                (KeyCode::Char(_), _) | (KeyCode::Backspace, _) | (KeyCode::Delete, _) | (KeyCode::Left, _) | (KeyCode::Right, _) | (KeyCode::Home, _) | (KeyCode::End, _) | (KeyCode::Up, _) | (KeyCode::Down, _) => {
                                                    let w = app.last_input_width;
                                                    app.editor.handle_input(k, w);
                                                    let _ = app.draw();
                                                }
                                                _ => {}
                                            }
                                        }
                                        break;
                                    }
                                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                                }
                            }
                        } else if let Some(mut app) = tick_app.try_lock() {
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
                                _ => {}
                            }
                        }
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

        let is_cancelled = self.repl.cancel_turn.load(Ordering::SeqCst);
        self.repl.cancel_turn.store(false, Ordering::SeqCst);

        if is_cancelled {
            let aid = self.repl.agent_id();
            let _ = self.repl.app.lock().push(RenderLine::SystemMsg(format!(
                "Turn detached via Ctrl+C. Agent: {aid} | Run persisting in background."
            )));
            let _ = self.repl.app.lock().push(RenderLine::SystemMsg(
                "Press Ctrl+C again or type /exit to end session.".to_string(),
            ));
        }

        // Blank line after every agent turn for visual block separation
        let _ = self.repl.app.lock().push(RenderLine::Blank);

        // Stop thinking animation
        tick_handle.abort();
        let _ = tick_handle.await;
        let secs = self.repl.app.lock().stop_thinking();
        {
            let mut stats = self.repl.session_stats.lock();
            stats.agent_active_ms += turn_start.elapsed().as_millis() as u64;
        }

        {
            let mut app = self.repl.app.lock();
            app.stop_thinking();
            app.set_last_status(None);
            if app.follow {
                app.scroll_to_bottom();
            }
            let _ = app.draw();
        }

        let stream_error = stream_res.as_ref().err().map(ToString::to_string);
        let summary = stream_res
            .as_ref()
            .ok()
            .and_then(|msgs| {
                msgs.iter()
                    .rfind(|m| m.msg_type() == "assistant_message")
                    .and_then(|m| m.data["content"].as_str())
            })
            .unwrap_or("")
            .to_string();
        let token_usage = Some(self.repl.session_output_tokens.load(Ordering::SeqCst));

        Ok(resolve_turn_outcome(
            is_cancelled,
            stream_error,
            summary,
            secs,
            token_usage,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};

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

        let cancelled = resolve_turn_outcome(
            true,
            Some("ignored after cancellation".to_string()),
            String::new(),
            5,
            Some(150),
        );
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
}
