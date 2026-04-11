use super::*;
use super::Repl;
use crate::Result;
use crate::ui::RenderLine;
use cade_agent::agent::client::CadeMessage;
use std::io;

impl Repl {
    pub(crate) async fn agent_turn(&mut self, stdout: &mut io::Stdout, input: &str) -> Result<()> {
        self.turn_checkpoint_taken = false;
        use std::sync::atomic::Ordering;

        let turn_start = std::time::Instant::now();
        let out_tok_before = self.session_output_tokens.load(Ordering::SeqCst);

        // Reset cancel flag at the start of every turn so Ctrl+C presses from
        // a previous turn don't immediately abort this one.  The application-
        // lifetime SIGINT watcher (spawned once in Repl::run) will set this
        // flag again if Ctrl+C is pressed during this turn.
        self.cancel_turn.store(false, Ordering::SeqCst);

        // Mark turn as active so OS SIGINT watcher knows to cancel it
        self.turn_active.store(true, Ordering::SeqCst);

        // On the first real turn, prefix with environment context
        let effective_input = if self
            .first_turn
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let env = self.build_env_context();
            format!(
                "{env}\n\n<system>Do not introduce yourself. Answer the user's message directly.</system>\n\n{input}"
            )
        } else {
            input.to_string()
        };

        // -- Skill trigger auto-detection
        // If the input matches any skill trigger, silently pre-load the skill
        // body by injecting it as a system context note before the user message.
        let effective_input = {
            let skills = self.skills.lock();
            let matched: Vec<String> = skills
                .iter()
                .filter(|s| s.matches_trigger(&effective_input))
                .map(|s| {
                    tracing::info!(
                        "Skill trigger matched: {} (skill: {})",
                        s.triggers
                            .iter()
                            .find(|t| effective_input.to_lowercase().contains(&t.to_lowercase()))
                            .cloned()
                            .unwrap_or_default(),
                        s.id
                    );
                    s.to_context_block()
                })
                .collect();
            drop(skills);

            if matched.is_empty() {
                effective_input
            } else {
                let injection = matched.join("\n---\n");
                format!("<skill_context>\n{injection}\n</skill_context>\n\n{effective_input}")
            }
        };

        // -- Thinking animation
        let bar_text = self
            .app
            .lock()
            .start_thinking("assessing… (esc to interrupt · 0s · 0↑)");

        // Redraw tick task — updates the spinner animation and assessing timer.
        let tick_app = self.app.clone();
        let tick_cancel = self.cancel_turn.clone();
        let tick_tokens = self.session_output_tokens.clone();
        let tick_base = out_tok_before;
        let tick_start = turn_start;
        let tick_bar = bar_text.clone();
        // I-01: message-queue Arcs shared with the tick task.
        let tick_queued_steering = self.queued_steering.clone();
        let tick_queued_followup = self.queued_followup.clone();
        let tick_modal_close_ms = self.last_modal_close_ms.clone();
        let tick_permissions = self.permissions.clone();
        let tick_handle = tokio::spawn(async move {
            use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
            use futures::StreamExt;
            let mut reader = EventStream::new();
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(16)) => {
                        // Update assessing text once per second
                        let secs = tick_start.elapsed().as_secs();
                        let toks = tick_tokens.load(Ordering::SeqCst).saturating_sub(tick_base);
                        {
                            let cur = tick_bar.lock().clone();
                            if cur.starts_with("assessing") || cur.starts_with("CADE thinking") {
                                *tick_bar.lock() =
                                    format!("assessing… (esc to interrupt · {secs}s · {toks}↑)");
                            }
                        }
                        // R-01: Only draw if the app has pending state changes
                        // (draw_dirty) or the thinking animation needs refreshing.
                        // This avoids redundant full-screen redraws when nothing
                        // has changed since the last frame.
                        if let Some(mut app) = tick_app.try_lock()
                            && (app.draw_dirty || app.thinking.is_some()) {
                                let _ = app.draw();
                            }
                    }
                    Some(Ok(evt)) = reader.next() => {
                        use crossterm::event::MouseEventKind;

                        // For key events targeting an active question modal we MUST
                        // not drop the event if the lock is momentarily held — retry
                        // until we get it so the oneshot sender is always delivered.
                        let needs_question_key = matches!(&evt, Event::Key(crossterm::event::KeyEvent { kind: KeyEventKind::Press, .. }));

                        if needs_question_key {
                            if let Event::Key(k) = evt {
                                // Spin-wait until app lock is available,
                                // then process the key (async question or Esc/scroll).
                                loop {
                                    if let Some(mut app) = tick_app.try_lock() {
                                        let has_async_question = app.active_question
                                            .as_ref()
                                            .is_some_and(|aq| aq.tx.is_some());
                                        if has_async_question {
                                            app.handle_question_key(k);
                                        } else {
                                            match (k.code, k.modifiers) {
                                                    (KeyCode::Char('K'), _) => { app.follow = false; app.scroll = app.scroll.saturating_add(10); let _ = app.draw(); }
                                                    (KeyCode::Char('J'), _) => { app.scroll = 0; app.follow = true; let _ = app.draw(); }
                                                    (KeyCode::Char('o'), KeyModifiers::CONTROL) => { app.expand_all = !app.expand_all; let _ = app.draw(); }
                                                    (KeyCode::Tab, _) => {
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

                                                    // -- I-01: input during agent turn
                                                    //
                                                    // Ctrl+C      → steering: cancel + redirect
                                                    //               (or plain cancel if input empty).
                                                    // Plain Enter → queue as follow-up (no cancel).
                                                    // Ctrl+Enter  → also queue as follow-up.
                                                    // Alt/Shift+Enter → same as plain Enter.
                                                    //
                                                    // Ctrl+Enter: queue as follow-up (like plain Enter).
                                                    // Steering is handled by Ctrl+C below.
                                                    (KeyCode::Enter, m)
                                                        if m == KeyModifiers::CONTROL =>
                                                    {
                                                        app.editor.expand_pastes();
                                                        let msg = app.editor.text().trim().to_string();
                                                        if !msg.is_empty() {
                                                            let now_ms = now_epoch_ms();
                                                            let last_close = tick_modal_close_ms
                                                                .load(std::sync::atomic::Ordering::SeqCst);
                                                            let post_modal = last_close > 0
                                                                && now_ms.saturating_sub(last_close) < 300;
                                                            if !post_modal {
                                                                tick_queued_followup.lock().push_back(msg);
                                                                app.queued_count = tick_queued_followup.lock().len();
                                                                app.editor.clear();
                                                                app.editor.set_cursor_pos(0);
                                                                app.set_last_status(None);
                                                                let _ = app.draw();
                                                            }
                                                        }
                                                    }
                                                    // Plain Enter: queue as follow-up without
                                                    // cancelling the current turn.  Messages run in
                                                    // submission order once the agent is free.
                                                    (KeyCode::Enter, m)
                                                        if m == KeyModifiers::NONE =>
                                                    {
                                                        app.editor.expand_pastes();
                                                        let msg = app.editor.text().trim().to_string();
                                                        if !msg.is_empty() {
                                                            let now_ms = now_epoch_ms();
                                                            let last_close = tick_modal_close_ms
                                                                .load(std::sync::atomic::Ordering::SeqCst);
                                                            let post_modal = last_close > 0
                                                                && now_ms.saturating_sub(last_close) < 300;
                                                            if !post_modal {
                                                                tick_queued_followup.lock().push_back(msg);
                                                                app.queued_count = tick_queued_followup.lock().len();
                                                                app.editor.clear();
                                                                app.editor.set_cursor_pos(0);
                                                                app.set_last_status(None);
                                                                let _ = app.draw();
                                                            }
                                                        }
                                                    }
                                                    // Multi-line input (mirrors idle-mode behaviour).
                                                    (KeyCode::Enter, m)
                                                        if cade_tui::app::input::is_newline_shortcut(m) =>
                                                    {
                                                        app.editor.insert_newline();
                                                        let _ = app.draw();
                                                    }
                                                    (KeyCode::Esc, _) => {
                                                        // Ignore Esc events that arrive within
                                                        // the first 200 ms of the turn.  The
                                                        // terminal can buffer an Esc pressed just
                                                        // before or just after the user hit Enter
                                                        // to submit their message; without this
                                                        // guard the tick task would process that
                                                        // stale Esc and immediately cancel the
                                                        // turn before any LLM content arrives.
                                                        //
                                                        // Also ignore Esc events that arrive within
                                                        // 500 ms of a modal closing.  Terminals
                                                        // often emit residual escape sequences when
                                                        // the alternate screen is restored; without
                                                        // this guard a stale Esc fires during the
                                                        // HTTP wait of Phase-2 tool-result sending
                                                        // and aborts the turn right after the user
                                                        // confirmed "Yes" in a question modal.
                                                        let esc_now_ms = now_epoch_ms();
                                                        let esc_last_close = tick_modal_close_ms
                                                            .load(std::sync::atomic::Ordering::SeqCst);
                                                        let esc_post_modal = esc_last_close > 0
                                                            && esc_now_ms.saturating_sub(esc_last_close) < 500;
                                                        if !esc_post_modal && tick_start.elapsed().as_millis() >= 200
                                                            && !app.editor.is_empty() {
                                                                // Clear typed input rather than
                                                                // cancelling — lets user discard
                                                                // a queued message without stopping
                                                                // the agent.
                                                                app.editor.clear();
                                                                app.editor.set_cursor_pos(0);
                                                                app.set_last_status(None);
                                                                let _ = app.draw();
                                                            }
                                                    }
                                                    // Ctrl+C — always cancel the running turn.
                                                    // Ctrl+C: if input is non-empty → steering
                                                    // (cancel + redirect with typed text).
                                                    // If input is empty → plain cancel.
                                                    // Same 200 ms grace period as Esc to swallow
                                                    // stale events buffered just after a modal.
                                                    // Also suppressed for 500 ms post-modal close
                                                    // (same reason as Esc above).
                                                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                                                        let cc_now_ms = now_epoch_ms();
                                                        let cc_last_close = tick_modal_close_ms
                                                            .load(std::sync::atomic::Ordering::SeqCst);
                                                        let cc_post_modal = cc_last_close > 0
                                                            && cc_now_ms.saturating_sub(cc_last_close) < 500;
                                                        if !cc_post_modal && tick_start.elapsed().as_millis() >= 200 {
                                                            app.editor.expand_pastes();
                                                            let msg = app.editor.text().trim().to_string();
                                                            if !msg.is_empty() {
                                                                // Steering: cancel current turn and
                                                                // run this message immediately after.
                                                                *tick_queued_steering.lock() = Some(msg);
                                                                app.editor.clear();
                                                                app.editor.set_cursor_pos(0);
                                                                app.set_last_status(None);
                                                                let _ = app.draw();
                                                            } else {
                                                                app.editor.clear();
                                                                app.editor.set_cursor_pos(0);
                                                                app.set_last_status(None);
                                                                let _ = app.draw();
                                                            }
                                                            tick_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                                                        }
                                                    }
                                                    (KeyCode::Char(_), _) | (KeyCode::Backspace, _) | (KeyCode::Delete, _) | (KeyCode::Left, _) | (KeyCode::Right, _) | (KeyCode::Home, _) | (KeyCode::End, _) | (KeyCode::Up, _) | (KeyCode::Down, _) => {
                                                        let lw = app.last_input_width;
                                                        app.editor.handle_key_event(k, lw);
                                                        let _ = app.draw();
                                                    }
                                                    _ => {}
                                                }
                                            }
                                            break;
                                        }
                                        // R-02: Sleep briefly before retry.  yield_now() spun
                                        // at full CPU speed when the lock was held by a draw();
                                        // 1 ms is long enough to release contention pressure
                                        // without adding visible latency for key delivery.
                                        tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                                    }
                            }
                        } else if let Some(mut app) = tick_app.try_lock() {
                            // Mouse / resize — best-effort, fine to drop
                            if let Event::Mouse(m) = evt { match m.kind {
                                MouseEventKind::ScrollUp   => { app.follow = false; app.scroll = app.scroll.saturating_add(1); let _ = app.draw(); }
                                MouseEventKind::ScrollDown => { if app.scroll > 1 { app.scroll = app.scroll.saturating_sub(1); } else { app.scroll = 0; app.follow = true; } let _ = app.draw(); }
                                _ => {}
                            } }
                        }
                    }
                }
            }
        });

        let messages = self
            .stream_turn(
                stdout,
                &effective_input,
                false,
                "",
                "",
                false,
                None,
                Some(bar_text.clone()),
            )
            .await;

        let messages = messages?;

        let is_cancelled = self.cancel_turn.load(Ordering::SeqCst);
        // Clear cancel flag after turn completes
        self.cancel_turn.store(false, Ordering::SeqCst);

        let mut turn_stats = TurnStats::default();
        if is_cancelled {
            // Skip tool execution so we don't trigger auto-reprompts on empty responses
        } else {
            self.dispatch_tool_calls(
                stdout,
                messages,
                input,
                Some(bar_text),
                false,
                &mut turn_stats,
            )
            .await?;
        }

        // C3: Once per session, after enough write activity, check whether the
        // agent has been updating its working_set block.  If it's still empty
        // inject a single ephemeral reminder so the model fills it in — this
        // ensures the block survives context rotation during long coding sessions.
        const WORKING_SET_WRITE_THRESHOLD: u32 = 8;
        if self.write_tool_calls.load(Ordering::SeqCst) >= WORKING_SET_WRITE_THRESHOLD
            && !self.working_set_notified.load(Ordering::SeqCst)
        {
            self.working_set_notified.store(true, Ordering::SeqCst);
            if let Err(e) = self.inject_working_set_reminder(stdout).await {
                tracing::debug!("working_set reminder failed: {e}");
            }
        }

        // Blank line after every agent turn for visual block separation.
        let _ = self
            .app
            .lock()
            .push(RenderLine::Blank);

        // -- Stop thinking animation
        tick_handle.abort();
        let _ = tick_handle.await;
        let secs = self.app.lock().stop_thinking();
        // Accumulate agent-active time in session stats
        { let mut stats = self.session_stats.lock();
            stats.agent_active_ms += turn_start.elapsed().as_millis() as u64;
        }
        let time_str = if secs >= 60 {
            format!("{}m {}s", secs / 60, secs % 60)
        } else {
            format!("{}s", secs)
        };

        let summary = if is_cancelled {
            format!("⚠ Interrupted after {}", time_str)
        } else {
            let mut parts = vec![format!("✓ Finished in {}", time_str)];
            if turn_stats.reads > 0 {
                parts.push(format!(
                    "{} read{}",
                    turn_stats.reads,
                    if turn_stats.reads == 1 { "" } else { "s" }
                ));
            }
            if turn_stats.edits > 0 {
                parts.push(format!(
                    "{} edit{}",
                    turn_stats.edits,
                    if turn_stats.edits == 1 { "" } else { "s" }
                ));
            }
            if turn_stats.cmds > 0 {
                parts.push(format!(
                    "{} cmd{}",
                    turn_stats.cmds,
                    if turn_stats.cmds == 1 { "" } else { "s" }
                ));
            }
            parts.join("  ·  ")
        };
        self.app
            .lock()
            .set_last_status(Some(summary));
        let _ = self.app.lock().draw();

        self.turn_active.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Send a user message and drive the tool-call loop with live SSE streaming.
    /// Thin wrapper: start a turn, optionally attaching pasted images.
    pub(crate) async fn agent_turn_with_images(
        &mut self,
        stdout: &mut io::Stdout,
        input: &str,
        images: Vec<serde_json::Value>,
    ) -> Result<()> {
        // Store images on self so the inner agent_turn send path can pick them up.
        self.pending_turn_images = images;
        self.agent_turn(stdout, input).await
    }

    /// Commit any in-progress streaming/reasoning, push an error line, and
    /// return an empty message vec.  Shared cleanup path for stream errors.
    pub(crate) fn abort_stream_ui(&self, msg: impl Into<String>) -> Vec<CadeMessage> {
        let mut app = self.app.lock();
        let _ = app.commit_reasoning();
        let _ = app.commit_streaming();
        let _ = app.push(RenderLine::ErrorMsg(msg.into()));
        vec![]
    }

}
