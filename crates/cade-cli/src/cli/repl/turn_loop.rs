use super::{EMPTY_YIELD_REPROMPT, Repl, ToolPreflightResult};
use super::{fmt_tok_short, fmt_window_tokens_short, short_mode_label};
use crate::Result;
use crate::support::text::{FinishReasonCategory, finish_reason_hint, truncate};
use crate::ui::RenderLine;
use cade_agent::agent::client::CadeMessage;
use std::io;

#[derive(Default, Debug)]
pub(crate) struct TurnStats {
    pub reads: u32,
    pub edits: u32,
    pub cmds: u32,
}

/// Current wall-clock time as milliseconds since the Unix epoch.
pub(crate) fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Build a `ToolPreflightResult::Blocked` with the given error message.
pub(crate) fn blocked_result(
    call_id: &str,
    tool_name: &str,
    output: impl Into<String>,
) -> ToolPreflightResult {
    ToolPreflightResult::Blocked(cade_agent::tools::ToolResult {
        tool_call_id: call_id.to_string(),
        tool_name: tool_name.to_string(),
        output: output.into(),
        is_error: true,
    })
}

impl Repl {
    /// Commit any in-progress streaming/reasoning, push an error line, and
    /// return an empty message vec.  Shared cleanup path for stream errors.
    fn abort_stream_ui(&self, msg: impl Into<String>) -> Vec<CadeMessage> {
        let mut app = self.app.lock();
        let _ = app.commit_reasoning();
        let _ = app.commit_streaming();
        let _ = app.push(RenderLine::ErrorMsg(msg.into()));
        vec![]
    }
    pub(crate) fn build_env_context(&self) -> String {
        use std::process::Command;

        let now = chrono::Local::now().format("%Y-%m-%d %H:%M %Z");

        // OS / kernel
        let os_info = {
            let uname = {
                let mut cmd = Command::new("uname");
                cade_core::agent_env::apply_agent_env(&mut cmd);
                cmd.arg("-sr").output()
            }
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
            // Try /etc/os-release for distro name
            let distro = std::fs::read_to_string("/etc/os-release")
                .unwrap_or_default()
                .lines()
                .find(|l| l.starts_with("PRETTY_NAME="))
                .map(|l| {
                    l.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string()
                })
                .unwrap_or_default();
            if distro.is_empty() {
                uname.trim().to_string()
            } else {
                format!("{} ({})", uname.trim(), distro)
            }
        };

        // CWD
        let cwd = self.cwd.display().to_string();

        // Git info
        let git_info = {
            let branch = {
                let mut cmd = Command::new("git");
                cade_core::agent_env::apply_agent_env(&mut cmd);
                cmd.args(["-C", &cwd, "rev-parse", "--abbrev-ref", "HEAD"])
                    .output()
            }
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout).ok()
                } else {
                    None
                }
            })
            .map(|s| s.trim().to_string());

            let status = {
                let mut cmd = Command::new("git");
                cade_core::agent_env::apply_agent_env(&mut cmd);
                cmd.args(["-C", &cwd, "status", "--porcelain"]).output()
            }
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout).ok()
                } else {
                    None
                }
            });

            match (branch, status) {
                (Some(b), Some(s)) if !b.is_empty() => {
                    let lines: Vec<&str> = s.lines().collect();
                    if lines.is_empty() {
                        format!("branch={b}, clean")
                    } else {
                        format!(
                            "branch={b}, {} uncommitted change{}",
                            lines.len(),
                            if lines.len() == 1 { "" } else { "s" }
                        )
                    }
                }
                _ => String::new(),
            }
        };

        let mut parts = vec![
            format!("Date:   {now}"),
            format!("OS:     {os_info}"),
            format!("CWD:    {cwd}"),
        ];
        if !git_info.is_empty() {
            parts.push(format!("Git:    {git_info}"));
        }
        format!("<environment>\n{}\n</environment>", parts.join("\n"))
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
                                                    // Shift+Enter: insert newline at cursor for
                                                    // multi-line input (mirrors idle-mode behaviour).
                                                    (KeyCode::Enter, m)
                                                        if m == KeyModifiers::SHIFT =>
                                                    {
                                                        app.editor.insert_newline();
                                                        let _ = app.draw();
                                                    }
                                                    // Alt+Enter: queue as follow-up without
                                                    // cancelling the current turn.
                                                    (KeyCode::Enter, m)
                                                        if m == KeyModifiers::ALT
                                                        || m == (KeyModifiers::SHIFT | KeyModifiers::ALT) =>
                                                    {
                                                        app.editor.expand_pastes();
                                                        let msg = app.editor.text().trim().to_string();
                                                        if !msg.is_empty() {
                                                            tick_queued_followup.lock().push_back(msg);
                                                            app.queued_count = tick_queued_followup.lock().len();
                                                            app.editor.clear();
                                                            app.editor.set_cursor_pos(0);
                                                            let _ = app.draw();
                                                        }
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

    /// Stream one turn (user message or tool return) and render live.
    /// Returns the complete collected message list.
    ///
    /// `bar_text`: optional shared string updated by tool_call_message events
    /// to keep the ThinkingBar status current.
    pub(crate) async fn stream_turn(
        &mut self,
        _stdout: &mut io::Stdout,
        input: &str,
        is_tool_return: bool,
        tool_call_id: &str,
        tool_output: &str,
        // When true, user message is sent to LLM but NOT persisted to DB.
        // Used for system-injected re-prompts (EMPTY_YIELD_REPROMPT) so they
        // don't pollute conversation history or consume future context window.
        ephemeral: bool,
        _spinner: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        bar_text: Option<std::sync::Arc<parking_lot::Mutex<String>>>,
    ) -> Result<Vec<CadeMessage>> {
        // -- R-04: Async event buffering
        // Decouples network I/O from TUI rendering.  The SSE callback (`on_event`)
        // performs only lightweight session/stats bookkeeping and forwards each
        // message to an unbounded channel.  A dedicated UI consumer task reads
        // from the channel and applies all TuiApp mutations + draws.  This means
        // the SSE event loop is never blocked by draw() or lock contention.

        // -- Per-turn channel
        let (ui_tx, ui_rx) = tokio::sync::mpsc::unbounded_channel::<CadeMessage>();

        // -- Session / stats state (used by on_event — NO TuiApp access)
        let conv_arc = self.conversation_id.clone();
        let session_arc = self.session.clone();
        let sess_in_tok = self.session_input_tokens.clone();
        let sess_out_tok = self.session_output_tokens.clone();
        let sess_stats = self.session_stats.clone();
        let run_id_cell: std::sync::Arc<parking_lot::Mutex<Option<String>>> = Default::default();
        let seq_id_cell: std::sync::Arc<parking_lot::Mutex<Option<i64>>> = Default::default();
        let run_id_cell2 = run_id_cell.clone();
        let seq_id_cell2 = seq_id_cell.clone();
        let finish_reason_arc: std::sync::Arc<parking_lot::Mutex<Option<String>>> =
            Default::default();
        let finish_reason_cb = finish_reason_arc.clone();

        // -- on_event: SSE callback — stats only, then forward to UI channel
        let on_event = move |msg: &CadeMessage| {
            match msg.msg_type() {
                "stream_start" => {
                    if let Some(cid) = msg.data["conversation_id"].as_str()
                        && !cid.is_empty()
                        && conv_arc.lock().as_deref() != Some(cid)
                    {
                        let cid: String = cid.to_string();
                        *conv_arc.lock() = Some(cid.clone());
                        { let mut s = session_arc.lock();
                            let _ = s.set_conversation(Some(cid));
                        }
                    }
                    if let Some(rid) = msg.run_id() {
                        *run_id_cell2.lock() = Some(rid.to_string());
                    }
                }
                "usage_statistics" => {
                    use std::sync::atomic::Ordering;
                    if let Some(n) = msg.data["input_tokens"].as_u64() {
                        sess_in_tok.fetch_add(n, Ordering::SeqCst);
                    }
                    if let Some(n) = msg.data["output_tokens"].as_u64() {
                        sess_out_tok.fetch_add(n, Ordering::SeqCst);
                    }
                    { let mut stats = sess_stats.lock();
                        let model = msg.data["model"].as_str().unwrap_or("").to_string();
                        let input = msg.data["input_tokens"].as_u64().unwrap_or(0);
                        let cache_read = msg.data["cache_read_tokens"].as_u64().unwrap_or(0);
                        let cache_write = msg.data["cache_write_tokens"].as_u64().unwrap_or(0);
                        let output = msg.data["output_tokens"].as_u64().unwrap_or(0);
                        stats.record_usage(&model, input, cache_read, cache_write, output);
                    }
                }
                "finish_reason" => {
                    if let Some(reason) = msg.data["reason"].as_str() {
                        *finish_reason_cb.lock() = Some(reason.to_string());
                    }
                }
                _ => {}
            }
            if let Some(s) = msg.seq_id() {
                *seq_id_cell2.lock() = Some(s);
            }
            // Forward to UI consumer (non-blocking, never stalls the SSE loop).
            let _ = ui_tx.send(msg.clone());
        };

        // -- UI consumer task — all TuiApp mutations happen here
        let app_arc = self.app.clone();
        let bar_text_arc = bar_text;
        let reasoning_buf = self.last_reasoning.clone();
        let assistant_buf = self.last_assistant_text.clone();
        // Session-level stats for footer metrics (tokens, cost, cache usage)
        let sess_in_tok_ui = self.session_input_tokens.clone();
        let sess_out_tok_ui = self.session_output_tokens.clone();
        let sess_stats_ui = self.session_stats.clone();
        // Full model ID (provider/name) for accurate context window lookup.
        // The usage event's `model` field carries only the bare name (after
        // the LlmRouter strips the provider prefix), which causes
        // context_window_for_model to fall through to a wrong default.
        let full_model_id = self.model();
        // Clear buffers at the start of each turn.
        reasoning_buf.lock().clear();
        assistant_buf.lock().clear();
        let ui_task = tokio::spawn(async move {
            let mut ui_rx = ui_rx;
            let mut in_reasoning = false;
            let mut in_assistant = false;
            while let Some(msg) = ui_rx.recv().await {
                match msg.msg_type() {
                    "reasoning_message" => {
                        if let Some(text) = msg.reasoning_text() {
                            in_reasoning = true;
                            reasoning_buf.lock().push_str(text);
                            app_arc
                                .lock()
                                .push_reasoning_chunk(text);
                        }
                    }
                    "assistant_message" => {
                        if let Some(text) = msg.assistant_text() {
                            assistant_buf.lock().push_str(text);
                            if !text.is_empty() {
                                in_reasoning = false;
                                in_assistant = true;
                                let line_count = {
                                    let mut app = app_arc.lock();
                                    app.commit_reasoning_inner();
                                    let _ = app.push_streaming_chunk(text);
                                    app.lines.len()
                                };
                                if let Some(bar) = &bar_text_arc {
                                    let cur = bar.lock().clone();
                                    if !cur.starts_with("●") {
                                        *bar.lock() =
                                            format!("generating… ({line_count} lines)");
                                    }
                                }
                            }
                        } else if in_reasoning {
                            let _ = app_arc.lock().commit_reasoning();
                            in_reasoning = false;
                        }
                    }
                    "tool_call_message" => {
                        in_reasoning = false;
                        {
                            let mut app = app_arc.lock();
                            app.commit_reasoning_inner();
                            let _ = app.commit_streaming();
                        }
                        in_assistant = false;
                        if let Some(bar) = &bar_text_arc {
                            let tool_name = msg.data["tool_calls"][0]["function"]["name"]
                                .as_str()
                                .unwrap_or("tool");
                            let display = if let Some(pos) = tool_name.rfind("__") {
                                &tool_name[pos + 2..]
                            } else {
                                tool_name
                            };
                            *bar.lock() = format!("● {}…", display);
                        }
                    }
                    "usage_statistics" => {
                        use std::sync::atomic::Ordering;

                        // Stats already updated in on_event; here we derive UI metrics:
                        // - session tokens (↑ input, ↓ output)
                        // - cache tokens (R read, W write)
                        // - total cost (USD)
                        // - context usage % and window size
                        // - current permission mode (auto/edits/plan/yolo)
                        //
                        // Use the full model ID (provider/name) for the context
                        // window lookup.  The usage event's `model` field carries
                        // only the bare name (router strips the prefix), which
                        // causes context_window_for_model to fall through to a
                        // wrong 32k default for dynamic/uncatalogued models.
                        let _model = msg.data["model"].as_str().unwrap_or("");
                        let input = msg.data["input_tokens"].as_u64().unwrap_or(0);
                        let cache_read = msg.data["cache_read_tokens"].as_u64().unwrap_or(0);
                        let window = cade_ai::catalogue::context_window_for_model(&full_model_id);

                        // Per-turn context usage for this model
                        let (pct_f_opt, pct_int_opt) = if window > 0 {
                            let used = input + cache_read;
                            let pct_f = (used as f64 / window as f64) * 100.0;
                            let pct_int = pct_f.round().min(99.0) as u8;
                            (Some(pct_f), Some(pct_int))
                        } else {
                            (None, None)
                        };

                        // Session-level aggregates
                        let in_tok = sess_in_tok_ui.load(Ordering::SeqCst);
                        let out_tok = sess_out_tok_ui.load(Ordering::SeqCst);
                        let (cache_r, cache_w, total_cost) = {
                            let stats = sess_stats_ui.lock();
                            let cache_r: u64 =
                                stats.per_model.values().map(|m| m.cache_read_tokens).sum();
                            let cache_w: u64 =
                                stats.per_model.values().map(|m| m.cache_write_tokens).sum();
                            let (total_cost, _) = stats.compute_cost();
                            (cache_r, cache_w, total_cost)
                        };

                        // Update TUI context_pct and footer_extra in one lock
                        let mut app = app_arc.lock();
                        if let Some(pct_int) = pct_int_opt {
                            app.set_context_pct(pct_int);
                        }
                        let ctx_pct_f = pct_f_opt
                            .unwrap_or_else(|| app.context_pct.map(|p| p as f64).unwrap_or(0.0));
                        let window_str = fmt_window_tokens_short(window);
                        let mode_label = short_mode_label(app.mode);

                        let metrics = format!(
                            "↑{} ↓{} R{} W{} ${:.3} {:.1}%/{} ({})",
                            fmt_tok_short(in_tok),
                            fmt_tok_short(out_tok),
                            fmt_tok_short(cache_r),
                            fmt_tok_short(cache_w),
                            total_cost,
                            ctx_pct_f,
                            window_str,
                            mode_label,
                        );
                        app.footer_extra = Some(metrics);
                    }
                    _ => {}
                }
            }
            // Channel closed — suppress unused-variable warnings.
            let _ = (in_reasoning, in_assistant);
        });

        // -- Streaming call (network I/O — on_event never touches TuiApp)
        let agent_id = self.agent_id();
        let cancel = &self.cancel_turn;

        fn is_cancel(e: &cade_agent::Error) -> bool {
            matches!(e, cade_agent::Error::Custom(s) if s == "__cancelled__")
        }

        let conv_id = self.conversation_id();
        let conv_ref = conv_id.as_deref();

        let messages = if is_tool_return {
            let reasoning_effort = self.reasoning_effort.lock().clone();
            match self
                .client
                .stream_tool_return_cancellable(
                    &agent_id,
                    tool_call_id,
                    tool_output,
                    false,
                    conv_ref,
                    reasoning_effort.as_deref(),
                    on_event,
                    Some(cancel),
                )
                .await
            {
                Ok(m) => m,
                Err(e) if is_cancel(&e) => {
                    ui_task.abort();
                    return Ok(self.abort_stream_ui("Turn interrupted"));
                }
                Err(e) => {
                    ui_task.abort();
                    return Ok(self.abort_stream_ui(e.to_string()));
                }
            }
        } else {
            use std::sync::atomic::Ordering;
            let streaming = self.streaming_enabled.load(Ordering::SeqCst);
            if streaming {
                // Consume any pasted images on the first (non-tool-return) turn.
                // Subsequent turns (tool returns, follow-ups) carry no images.
                let turn_images = if !is_tool_return {
                    std::mem::take(&mut self.pending_turn_images)
                } else {
                    vec![]
                };
                let reasoning_effort = self.reasoning_effort.lock().clone();
                match self
                    .client
                    .stream_message_cancellable_with_images(
                        &agent_id,
                        input,
                        conv_ref,
                        ephemeral,
                        turn_images,
                        reasoning_effort.as_deref(),
                        on_event,
                        Some(cancel),
                    )
                    .await
                {
                    Ok(m) => m,
                    Err(e) if is_cancel(&e) => {
                        ui_task.abort();
                        return Ok(self.abort_stream_ui("Turn interrupted"));
                    }
                    Err(e) => {
                        ui_task.abort();
                        return Ok(self.abort_stream_ui(e.to_string()));
                    }
                }
            } else {
                // Non-streaming path — single HTTP request, print result at end.
                // UI task is unused; abort it immediately.
                ui_task.abort();
                let turn_images_ns = if !is_tool_return {
                    std::mem::take(&mut self.pending_turn_images)
                } else {
                    vec![]
                };
                match self
                    .client
                    .send_message_with_images(&agent_id, input, turn_images_ns, ephemeral)
                    .await
                {
                    Ok(msgs) => {
                        for msg in &msgs {
                            if let Some(text) = msg.assistant_text()
                                && !text.is_empty()
                            {
                                let _ = self
                                    .app
                                    .lock()
                                    .push_streaming_chunk(text);
                            }
                        }
                        let _ = self.app.lock().commit_streaming();
                        msgs
                    }
                    Err(e) => {
                        let _ = self
                            .app
                            .lock()
                            .push(RenderLine::ErrorMsg(e.to_string()));
                        return Ok(vec![]);
                    }
                }
            }
        };

        // -- Drain UI consumer — let it process any remaining queued messages
        // on_event held the sender; the streaming call above consumed it (closure
        // dropped when stream_message_cancellable returned).  The channel is now
        // closed, so ui_rx.recv() will return None after draining.
        let _ = ui_task.await;

        let finish_reason_value = finish_reason_arc.lock().clone();

        // Safety-net commit: ensure reasoning/streaming are flushed even if the
        // UI task missed the final messages (e.g. channel race on success path).
        {
            let mut app = self.app.lock();
            let _ = app.commit_reasoning();
            let _ = app.commit_streaming();
        }

        // Post-stream diagnostics: finish reason, truncation heuristics, context usage.
        {
            let text = self
                .last_assistant_text
                .lock()
                .clone();
            let trimmed = text.trim_end();
            let looks_truncated = !trimmed.is_empty()
                && (trimmed.ends_with(':')
                || trimmed.ends_with("—")
                || trimmed.ends_with("...")
                || trimmed.ends_with('-')
                // Ends with a list-item prefix that was never followed by content
                || trimmed.ends_with("1.")
                || trimmed.ends_with("2.")
                || trimmed.ends_with("3."));

            let mut hints: Vec<String> = Vec::new();
            let mut suppress_truncation_hint = false;

            if let Some(reason) = finish_reason_value.as_deref()
                && let Some((msg, category)) = finish_reason_hint(reason)
            {
                if matches!(category, FinishReasonCategory::OutputLimit) {
                    suppress_truncation_hint = true;
                }
                hints.push(msg);
            }

            if looks_truncated && !suppress_truncation_hint {
                hints.push(
                    "⚠ Response may be incomplete — the model stopped generating. Try: /new for a fresh conversation, or rephrase your question.".to_string()
                );
            }

            let context_pct_opt = { self.app.lock().context_pct };
            if let Some(pct) = context_pct_opt
                && pct >= 95
            {
                hints.push(format!(
                        "⚠ Context window is {pct}% full — CADE summarized or trimmed older turns. Consider /new or ask for a shorter reply."
                    ));
            }
            for msg in hints {
                self.tui_dim(msg);
            }
        }

        // Save run_id + last seq_id for crash recovery / reconnect
        let saved_run_id = run_id_cell.lock().clone();
        let saved_seq_id = *seq_id_cell.lock();
        if saved_run_id.is_some() || saved_seq_id.is_some()
        {
            let mut s = self.session.lock();
            let _ = s.set_run(saved_run_id, saved_seq_id);
        }

        Ok(messages)
    }
}
