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

impl Repl {
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

    /// Run the Heuristic Evaluation Layer on user input before tool execution.
    /// This delegates a high-reasoning subagent to evaluate intent, safety, and pathfinding.
    pub(crate) async fn heuristic_evaluate(&self, input: &str) {
        if input.trim().is_empty() {
            return;
        }

        // Token reduction measure: We only invoke the full subagent evaluation
        // if the user input exceeds a certain character threshold, or contains
        // explicit tool/file keywords indicating a complex task.
        let is_complex = input.len() > 100
            || input.contains("file")
            || input.contains("code")
            || input.contains("test")
            || input.contains("implement");

        if !is_complex {
            self.tui_dim("  Input deemed simple: skipping full heuristic subagent evaluation to conserve tokens.");
            return;
        }

        self.tui_dim("  Evaluating heuristic constraints (Antivirus/Pathfinding) via subagent…");

        let prompt = format!(
            "You are the Heuristic Evaluation Layer. Perform an evaluation on the current task: The user has requested to '{}'.\nExecute the following logic and then call `update_memory` on the `working_set` block to persist the evolved context:\n1. Semantic Extraction: Parse Intent, Entities, Constraints.\n2. Antivirus Heuristic: Compare against Safety Protocol. If deviating, generate a corrective warning.\n3. Pathfinding Heuristic: Analyze distance to goal and recalculate Next Steps.\n4. CoT State Update: Update `working_set` with Vision, Progress, and Directives.",
            input
        );

        let args = serde_json::json!({
            "subagent_type": "heuristic_evaluator",
            "prompt": prompt,
            "background": false
        });

        // Run the subagent tool synchronously for this turn
        let _ = self.handle_run_subagent("heuristic_eval", &args).await;
        self.tui_ok("  Heuristic Evaluation completed. working_set updated.");
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

        // Run Heuristic Evaluator Layer
        self.heuristic_evaluate(input).await;

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
            let skills = self.skills.lock().expect("lock poisoned");
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
            .expect("lock poisoned")
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
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                        if tick_cancel.load(Ordering::SeqCst) { break; }
                        // Update assessing text once per second
                        let secs = tick_start.elapsed().as_secs();
                        let toks = tick_tokens.load(Ordering::SeqCst).saturating_sub(tick_base);
                        {
                            let cur = tick_bar.lock().expect("lock poisoned").clone();
                            if cur.starts_with("assessing") || cur.starts_with("CADE thinking") {
                                *tick_bar.lock().expect("lock poisoned") =
                                    format!("assessing… (esc to interrupt · {secs}s · {toks}↑)");
                            }
                        }
                        // R-01: Only draw if the app has pending state changes
                        // (draw_dirty) or the thinking animation needs refreshing.
                        // This avoids redundant full-screen redraws when nothing
                        // has changed since the last frame.
                        if let Ok(mut app) = tick_app.try_lock()
                            && (app.draw_dirty || app.thinking.is_some()) {
                                let _ = app.draw();
                            }
                    }
                    Some(Ok(evt)) = reader.next() => {
                        if tick_cancel.load(Ordering::SeqCst) { break; }
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
                                    if tick_cancel.load(Ordering::SeqCst) { break; }
                                    if let Ok(mut app) = tick_app.try_lock() {
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
                                                        let msg = app.editor.input.trim().to_string();
                                                        if !msg.is_empty() {
                                                            let now_ms = std::time::SystemTime::now()
                                                                .duration_since(std::time::UNIX_EPOCH)
                                                                .unwrap_or_default()
                                                                .as_millis() as u64;
                                                            let last_close = tick_modal_close_ms
                                                                .load(std::sync::atomic::Ordering::SeqCst);
                                                            let post_modal = last_close > 0
                                                                && now_ms.saturating_sub(last_close) < 300;
                                                            if !post_modal {
                                                                tick_queued_followup.lock().expect("lock poisoned").push_back(msg);
                                                                app.queued_count = tick_queued_followup.lock().expect("lock poisoned").len();
                                                                app.editor.input.clear();
                                                                app.editor.cursor_pos = 0;
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
                                                        let msg = app.editor.input.trim().to_string();
                                                        if !msg.is_empty() {
                                                            let now_ms = std::time::SystemTime::now()
                                                                .duration_since(std::time::UNIX_EPOCH)
                                                                .unwrap_or_default()
                                                                .as_millis() as u64;
                                                            let last_close = tick_modal_close_ms
                                                                .load(std::sync::atomic::Ordering::SeqCst);
                                                            let post_modal = last_close > 0
                                                                && now_ms.saturating_sub(last_close) < 300;
                                                            if !post_modal {
                                                                tick_queued_followup.lock().expect("lock poisoned").push_back(msg);
                                                                app.queued_count = tick_queued_followup.lock().expect("lock poisoned").len();
                                                                app.editor.input.clear();
                                                                app.editor.cursor_pos = 0;
                                                                let _ = app.draw();
                                                            }
                                                        }
                                                    }
                                                    // Shift+Enter: insert newline at cursor for
                                                    // multi-line input (mirrors idle-mode behaviour).
                                                    (KeyCode::Enter, m)
                                                        if m == KeyModifiers::SHIFT =>
                                                    {
                                                        let pos = app.editor.cursor_pos;
                                                        app.editor.input.insert(pos, '\n');
                                                        app.editor.cursor_pos = pos + 1;
                                                        let _ = app.draw();
                                                    }
                                                    // Alt+Enter: queue as follow-up without
                                                    // cancelling the current turn.
                                                    (KeyCode::Enter, m)
                                                        if m == KeyModifiers::ALT
                                                        || m == (KeyModifiers::SHIFT | KeyModifiers::ALT) =>
                                                    {
                                                        let msg = app.editor.input.trim().to_string();
                                                        if !msg.is_empty() {
                                                            tick_queued_followup.lock().expect("lock poisoned").push_back(msg);
                                                            app.queued_count = tick_queued_followup.lock().expect("lock poisoned").len();
                                                            app.editor.input.clear();
                                                            app.editor.cursor_pos = 0;
                                                            let _ = app.draw();
                                                        }
                                                    }
                                                    // Regular character input.
                                                    (KeyCode::Char(c), m)
                                                        if m == KeyModifiers::NONE
                                                        || m == KeyModifiers::SHIFT =>
                                                    {
                                                        let pos = app.editor.cursor_pos;
                                                        app.editor.input.insert(pos, c);
                                                        app.editor.cursor_pos = pos + c.len_utf8();
                                                        let _ = app.draw();
                                                    }
                                                    // Backspace — remove char before cursor.
                                                    (KeyCode::Backspace, _) => {
                                                        let cp = app.editor.cursor_pos;
                                                        if cp > 0 {
                                                            let new_pos = app.editor.input[..cp]
                                                                .char_indices()
                                                                .next_back()
                                                                .map(|(i, _)| i)
                                                                .unwrap_or(0);
                                                            app.editor.input.drain(new_pos..cp);
                                                            app.editor.cursor_pos = new_pos;
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
                                                        let esc_now_ms = std::time::SystemTime::now()
                                                            .duration_since(std::time::UNIX_EPOCH)
                                                            .unwrap_or_default()
                                                            .as_millis() as u64;
                                                        let esc_last_close = tick_modal_close_ms
                                                            .load(std::sync::atomic::Ordering::SeqCst);
                                                        let esc_post_modal = esc_last_close > 0
                                                            && esc_now_ms.saturating_sub(esc_last_close) < 500;
                                                        if !esc_post_modal && tick_start.elapsed().as_millis() >= 200
                                                            && !app.editor.input.is_empty() {
                                                                // Clear typed input rather than
                                                                // cancelling — lets user discard
                                                                // a queued message without stopping
                                                                // the agent.
                                                                app.editor.input.clear();
                                                                app.editor.cursor_pos = 0;
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
                                                        let cc_now_ms = std::time::SystemTime::now()
                                                            .duration_since(std::time::UNIX_EPOCH)
                                                            .unwrap_or_default()
                                                            .as_millis() as u64;
                                                        let cc_last_close = tick_modal_close_ms
                                                            .load(std::sync::atomic::Ordering::SeqCst);
                                                        let cc_post_modal = cc_last_close > 0
                                                            && cc_now_ms.saturating_sub(cc_last_close) < 500;
                                                        if !cc_post_modal && tick_start.elapsed().as_millis() >= 200 {
                                                            let msg = app.editor.input.trim().to_string();
                                                            if !msg.is_empty() {
                                                                // Steering: cancel current turn and
                                                                // run this message immediately after.
                                                                *tick_queued_steering.lock().expect("lock poisoned") = Some(msg);
                                                                app.editor.input.clear();
                                                                app.editor.cursor_pos = 0;
                                                                let _ = app.draw();
                                                            } else {
                                                                app.editor.input.clear();
                                                                app.editor.cursor_pos = 0;
                                                                let _ = app.draw();
                                                            }
                                                            tick_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                                                        }
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
                        } else if let Ok(mut app) = tick_app.try_lock() {
                            // Mouse / resize — best-effort, fine to drop
                            if let Event::Mouse(m) = evt { match m.kind {
                                MouseEventKind::ScrollUp   => { app.follow = false; app.scroll = app.scroll.saturating_add(3); let _ = app.draw(); }
                                MouseEventKind::ScrollDown => { if app.scroll > 3 { app.scroll = app.scroll.saturating_sub(3); } else { app.scroll = 0; app.follow = true; } let _ = app.draw(); }
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
            .expect("lock poisoned")
            .push(RenderLine::Blank);

        // -- Stop thinking animation
        tick_handle.abort();
        let _ = tick_handle.await;
        let secs = self.app.lock().expect("lock poisoned").stop_thinking();
        // Accumulate agent-active time in session stats
        if let Ok(mut stats) = self.session_stats.lock() {
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
            .expect("lock poisoned")
            .set_last_status(Some(summary));
        let _ = self.app.lock().expect("lock poisoned").draw();

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
        bar_text: Option<std::sync::Arc<std::sync::Mutex<String>>>,
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
        let run_id_cell: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
        let seq_id_cell: std::sync::Arc<std::sync::Mutex<Option<i64>>> = Default::default();
        let run_id_cell2 = run_id_cell.clone();
        let seq_id_cell2 = seq_id_cell.clone();
        let finish_reason_arc: std::sync::Arc<std::sync::Mutex<Option<String>>> =
            Default::default();
        let finish_reason_cb = finish_reason_arc.clone();

        // -- on_event: SSE callback — stats only, then forward to UI channel
        let on_event = move |msg: &CadeMessage| {
            match msg.msg_type() {
                "stream_start" => {
                    if let Some(cid) = msg.data["conversation_id"].as_str()
                        && !cid.is_empty()
                        && conv_arc.lock().expect("lock poisoned").as_deref() != Some(cid)
                    {
                        let cid: String = cid.to_string();
                        *conv_arc.lock().expect("lock poisoned") = Some(cid.clone());
                        if let Ok(mut s) = session_arc.lock() {
                            let _ = s.set_conversation(Some(cid));
                        }
                    }
                    if let Some(rid) = msg.run_id() {
                        *run_id_cell2.lock().expect("lock poisoned") = Some(rid.to_string());
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
                    if let Ok(mut stats) = sess_stats.lock() {
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
                        *finish_reason_cb.lock().expect("lock poisoned") = Some(reason.to_string());
                    }
                }
                _ => {}
            }
            if let Some(s) = msg.seq_id() {
                *seq_id_cell2.lock().expect("lock poisoned") = Some(s);
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
        reasoning_buf.lock().expect("lock poisoned").clear();
        assistant_buf.lock().expect("lock poisoned").clear();
        let ui_task = tokio::spawn(async move {
            let mut ui_rx = ui_rx;
            let mut in_reasoning = false;
            let mut in_assistant = false;
            while let Some(msg) = ui_rx.recv().await {
                match msg.msg_type() {
                    "reasoning_message" => {
                        if let Some(text) = msg.reasoning_text() {
                            in_reasoning = true;
                            reasoning_buf.lock().expect("lock poisoned").push_str(text);
                            app_arc
                                .lock()
                                .expect("lock poisoned")
                                .push_reasoning_chunk(text);
                        }
                    }
                    "assistant_message" => {
                        if let Some(text) = msg.assistant_text() {
                            assistant_buf.lock().expect("lock poisoned").push_str(text);
                            if !text.is_empty() {
                                in_reasoning = false;
                                in_assistant = true;
                                let line_count = {
                                    let mut app = app_arc.lock().expect("lock poisoned");
                                    app.commit_reasoning_inner();
                                    let _ = app.push_streaming_chunk(text);
                                    app.lines.len()
                                };
                                if let Some(bar) = &bar_text_arc {
                                    let cur = bar.lock().expect("lock poisoned").clone();
                                    if !cur.starts_with("●") {
                                        *bar.lock().expect("lock poisoned") =
                                            format!("generating… ({line_count} lines)");
                                    }
                                }
                            }
                        } else if in_reasoning {
                            let _ = app_arc.lock().expect("lock poisoned").commit_reasoning();
                            in_reasoning = false;
                        }
                    }
                    "tool_call_message" => {
                        in_reasoning = false;
                        {
                            let mut app = app_arc.lock().expect("lock poisoned");
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
                            *bar.lock().expect("lock poisoned") = format!("● {}…", display);
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
                            let stats = sess_stats_ui.lock().expect("lock poisoned");
                            let cache_r: u64 =
                                stats.per_model.values().map(|m| m.cache_read_tokens).sum();
                            let cache_w: u64 =
                                stats.per_model.values().map(|m| m.cache_write_tokens).sum();
                            let (total_cost, _) = stats.compute_cost();
                            (cache_r, cache_w, total_cost)
                        };

                        // Update TUI context_pct and footer_extra in one lock
                        let mut app = app_arc.lock().expect("lock poisoned");
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
            let reasoning_effort = self.reasoning_effort.lock().expect("lock poisoned").clone();
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
                    // Drop the sender so the UI task drains and exits.
                    ui_task.abort();
                    let mut app = self.app.lock().expect("lock poisoned");
                    let _ = app.commit_reasoning();
                    let _ = app.commit_streaming();
                    let _ = app.push(RenderLine::ErrorMsg("Turn interrupted".to_string()));
                    return Ok(vec![]);
                }
                Err(e) => {
                    ui_task.abort();
                    let mut app = self.app.lock().expect("lock poisoned");
                    let _ = app.commit_reasoning();
                    let _ = app.commit_streaming();
                    let _ = app.push(RenderLine::ErrorMsg(e.to_string()));
                    return Ok(vec![]);
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
                let reasoning_effort = self.reasoning_effort.lock().expect("lock poisoned").clone();
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
                        let mut app = self.app.lock().expect("lock poisoned");
                        let _ = app.commit_reasoning();
                        let _ = app.commit_streaming();
                        let _ = app.push(RenderLine::ErrorMsg("Turn interrupted".to_string()));
                        return Ok(vec![]);
                    }
                    Err(e) => {
                        ui_task.abort();
                        let mut app = self.app.lock().expect("lock poisoned");
                        let _ = app.commit_reasoning();
                        let _ = app.commit_streaming();
                        let _ = app.push(RenderLine::ErrorMsg(e.to_string()));
                        return Ok(vec![]);
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
                                    .expect("lock poisoned")
                                    .push_streaming_chunk(text);
                            }
                        }
                        let _ = self.app.lock().expect("lock poisoned").commit_streaming();
                        msgs
                    }
                    Err(e) => {
                        let _ = self
                            .app
                            .lock()
                            .expect("lock poisoned")
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

        let finish_reason_value = finish_reason_arc.lock().expect("lock poisoned").clone();

        // Safety-net commit: ensure reasoning/streaming are flushed even if the
        // UI task missed the final messages (e.g. channel race on success path).
        {
            let mut app = self.app.lock().expect("lock poisoned");
            let _ = app.commit_reasoning();
            let _ = app.commit_streaming();
        }

        // Post-stream diagnostics: finish reason, truncation heuristics, context usage.
        {
            let text = self
                .last_assistant_text
                .lock()
                .expect("lock poisoned")
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

            let context_pct_opt = { self.app.lock().expect("lock poisoned").context_pct };
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
        let saved_run_id = run_id_cell.lock().expect("lock poisoned").clone();
        let saved_seq_id = *seq_id_cell.lock().expect("lock poisoned");
        if (saved_run_id.is_some() || saved_seq_id.is_some())
            && let Ok(mut s) = self.session.lock()
        {
            let _ = s.set_run(saved_run_id, saved_seq_id);
        }

        Ok(messages)
    }

    /// Collect tool calls from messages and execute them one by one.
    ///
    /// `reprompt_done`: true when this call is itself the result of an auto-reprompt
    /// injection — prevents infinite reprompt loops if the LLM keeps returning empty.
    pub(crate) async fn dispatch_tool_calls(
        &mut self,
        stdout: &mut io::Stdout,
        messages: Vec<CadeMessage>,
        user_input: &str,
        bar_text: Option<std::sync::Arc<std::sync::Mutex<String>>>,
        reprompt_done: bool,
        turn_stats: &mut TurnStats,
    ) -> Result<()> {
        // If the user cancelled (Esc/Ctrl+C) during Phase 2 tool-result sending,
        // stream_turn may return vec![] due to the cancellation rather than an
        // actual empty LLM response.  Bail out immediately so the re-prompt
        // guard doesn't fire and override the user's intent.
        if self.cancel_turn.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(());
        }

        let tool_calls: Vec<(String, String, serde_json::Value)> =
            messages.iter().filter_map(|m| m.as_tool_call()).collect();

        // C3: Track file-write/edit/bash tool calls for the working_set reminder.
        const WRITE_TOOL_NAMES: &[&str] = &[
            "bash",
            "write_file",
            "edit_file",
            "apply_patch",
            "WriteFileGemini",
            "Replace",
            "RunShellCommand",
        ];
        let wc = tool_calls
            .iter()
            .filter(|(_, name, _)| WRITE_TOOL_NAMES.contains(&name.as_str()))
            .count() as u32;
        if wc > 0 {
            self.write_tool_calls
                .fetch_add(wc, std::sync::atomic::Ordering::SeqCst);
        }

        // Update turn statistics
        for (_, name, _) in &tool_calls {
            match name.as_str() {
                "bash" | "RunShellCommand" | "desktop-commander__start_process" => {
                    turn_stats.cmds += 1
                }
                "write_file"
                | "edit_file"
                | "apply_patch"
                | "WriteFileGemini"
                | "Replace"
                | "desktop-commander__write_file"
                | "desktop-commander__edit_block" => turn_stats.edits += 1,
                "read_file"
                | "ReadFileGemini"
                | "glob"
                | "GlobGemini"
                | "grep"
                | "SearchFileContent"
                | "desktop-commander__read_file"
                | "desktop-commander__read_multiple_files" => turn_stats.reads += 1,
                _ => {
                    // Fallback heuristics
                    if name.contains("read")
                        || name.contains("search")
                        || name.contains("find")
                        || name.contains("grep")
                        || name.contains("list")
                    {
                        turn_stats.reads += 1;
                    } else if name.contains("write")
                        || name.contains("edit")
                        || name.contains("patch")
                        || name.contains("update")
                        || name.contains("create")
                    {
                        turn_stats.edits += 1;
                    } else if name.contains("bash")
                        || name.contains("shell")
                        || name.contains("cmd")
                        || name.contains("run")
                    {
                        turn_stats.cmds += 1;
                    }
                }
            }
        }

        if tool_calls.is_empty() {
            // No tool calls → agent has stopped. Collect final assistant text.
            let assistant_msg: String = messages
                .iter()
                .filter_map(|m| m.assistant_text())
                .collect::<Vec<_>>()
                .join(" ");

            // Auto-reprompt: if the LLM produced nothing at all this entire turn,
            // inject a single follow-up user message so it knows it must respond.
            // `reprompt_done` guards against infinite loops — we only inject once.
            if assistant_msg.trim().is_empty() && !reprompt_done {
                tracing::warn!("Empty agent response after tool return — injecting re-prompt");
                let _ = self
                    .app
                    .lock()
                    .expect("lock poisoned")
                    .push(RenderLine::SystemMsg(
                        "  ⎿  (no response after tool — re-prompting)".to_string(),
                    ));
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                let follow = self
                    .stream_turn(
                        stdout,
                        EMPTY_YIELD_REPROMPT,
                        false,
                        "",
                        "",
                        true,
                        None,
                        bar_text.clone(),
                    )
                    .await?;
                Box::pin(
                    self.dispatch_tool_calls(
                        stdout, follow, user_input, bar_text, true, turn_stats,
                    ),
                )
                .await?;
                return Ok(());
            }

            // Stop hook — exit 2 feeds stderr back to agent as a continuation
            let last_reasoning = self.last_reasoning.lock().expect("lock poisoned").clone();
            let stop_outcome = self
                .hooks
                .stop(
                    "end_turn",
                    user_input,
                    &assistant_msg,
                    if last_reasoning.is_empty() {
                        None
                    } else {
                        Some(&last_reasoning)
                    },
                )
                .await;
            if let cade_core::hooks::HookOutcome::Block { reason } = stop_outcome {
                let _ = self
                    .app
                    .lock()
                    .expect("lock poisoned")
                    .push(RenderLine::SystemMsg(format!(
                        "  ⎿  Hook continuing: {reason}"
                    )));
                // Clear any stale cancel flag before the hook-continuation stream_turn.
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                // Feed the hook's stderr back to the agent as a new turn
                let follow_msgs = self
                    .stream_turn(
                        stdout,
                        &reason,
                        false,
                        "",
                        "",
                        false,
                        None,
                        bar_text.clone(),
                    )
                    .await?;
                Box::pin(self.dispatch_tool_calls(
                    stdout,
                    follow_msgs,
                    user_input,
                    bar_text,
                    false,
                    turn_stats,
                ))
                .await?;
            }
            return Ok(());
        }

        // Check if this response contained any assistant text alongside the tool calls.
        // Passed into each recursive dispatch so the re-prompt is suppressed when
        // the model spoke earlier in the chain (not just in prior tool-return rounds).
        // -- Execute all tools, then send results as a batch
        //
        // Tools execute sequentially (preserves approval prompts and the
        // &mut stdout requirement).  Results are collected first, then sent to
        // the server one-by-one.  The server's pending_tool_results guard holds
        // the LLM call until every expected result has arrived, so only ONE LLM
        // round-trip is needed regardless of how many tools the LLM called.
        // This replaces the old pattern that triggered a separate LLM call after
        // each individual tool, wasting N-1 context round-trips per response.

        // Update bar text with all tool names up-front.
        if let Some(bar) = &bar_text {
            let display = tool_calls
                .iter()
                .map(|(_, name, _)| name.rfind("__").map_or(name.as_str(), |p| &name[p + 2..]))
                .collect::<Vec<_>>()
                .join(", ");
            *bar.lock().expect("lock poisoned") = format!("● {}…", display);
        }

        // -- Phase 1: Sequential preflight (approval, blocking, hooks)
        // Each tool is checked for permissions, plan-mode blocking, and hook
        // denial. Tools that fail preflight get an immediate error result.
        // Tools that pass get queued for execution.
        let mut preflight: Vec<ToolPreflightResult> = Vec::with_capacity(tool_calls.len());
        for (call_id, tool_name, args) in &tool_calls {
            // Native tool intercepts that require &self must run sequentially
            // in Phase 1 because they access Repl state (client, skills, etc.).
            let native_result = self.try_native_intercept(call_id, tool_name, args).await;
            if let Some(result) = native_result {
                // Show tool call header for native intercepts
                let _ = self
                    .app
                    .lock()
                    .expect("lock poisoned")
                    .push(RenderLine::ToolCall {
                        name: tool_name.to_string(),
                        preview: String::new(),
                    });
                preflight.push(ToolPreflightResult::Blocked(result?));
                continue;
            }
            // Show tool call header
            {
                let preview = Self::tool_preview(tool_name, args);
                let _ = self
                    .app
                    .lock()
                    .expect("lock poisoned")
                    .push(RenderLine::ToolCall {
                        name: tool_name.to_string(),
                        preview,
                    });
            }
            let pf = self
                .preflight_tool(stdout, call_id, tool_name, args)
                .await?;
            preflight.push(pf);
        }

        // -- Phase 2: Parallel execution of approved tools
        // Read-only tools execute concurrently via tokio::spawn.
        // Write tools execute sequentially to prevent filesystem races.
        let mut results: Vec<cade_agent::tools::ToolResult> = Vec::with_capacity(tool_calls.len());

        // Separate into read and write buckets (preserving original indices).
        let mut read_indices: Vec<usize> = Vec::new();
        let mut write_indices: Vec<usize> = Vec::new();

        for (i, (_, tool_name, _)) in tool_calls.iter().enumerate() {
            if matches!(&preflight[i], ToolPreflightResult::Blocked(_)) {
                continue; // Already have a result
            }
            if cade_agent::tools::is_write_tool(tool_name, &self.mcp).await {
                write_indices.push(i);
            } else {
                read_indices.push(i);
            }
        }

        // Pre-allocate result slots.
        results.resize_with(tool_calls.len(), || cade_agent::tools::ToolResult {
            tool_call_id: String::new(),
            tool_name: String::new(),
            output: String::new(),
            is_error: false,
        });

        // Fill in blocked results first.
        for (i, pf) in preflight.iter().enumerate() {
            if let ToolPreflightResult::Blocked(r) = pf {
                results[i] = r.clone();
            }
        }

        // Auto-checkpoint (Phase 2): if there are pending write operations, take a checkpoint.
        if !write_indices.is_empty() && !self.turn_checkpoint_taken {
            let auto_enabled = self
                .settings
                .lock()
                .expect("lock poisoned")
                .project()
                .auto_checkpoint;
            if auto_enabled {
                self.tui_dim("  📦 Creating pre-edit auto-checkpoint...".to_string());

                // Attempt to create checkpoint
                let agent_id = self.agent_id();
                let conv_id = self.conversation_id();

                use cade_agent::tools::git_checkpoint;
                let git_cp = git_checkpoint::create_git_checkpoint("auto", &self.cwd).await;
                let stash = git_cp.as_ref().and_then(|g| g.stash_ref.as_deref());
                let commit = git_cp.as_ref().and_then(|g| g.commit_hash.as_deref());

                match self
                    .client
                    .create_checkpoint(
                        &agent_id,
                        Some("auto"),
                        Some("Created automatically prior to destructive tool execution"),
                        conv_id.as_deref(),
                        stash,
                        commit,
                    )
                    .await
                {
                    Ok(id) => {
                        let msg = if stash.is_some() {
                            format!(
                                "  ✓ Auto-checkpoint & stash saved (ID: {})",
                                &id[..8.min(id.len())]
                            )
                        } else {
                            format!("  ✓ Auto-checkpoint saved (ID: {})", &id[..8.min(id.len())])
                        };
                        self.tui_ok(msg);
                        self.turn_checkpoint_taken = true;
                    }
                    Err(e) => {
                        self.tui_err(format!("  ⚠ Auto-checkpoint failed: {e}"));
                    }
                }
            }
        }

        // Snapshot reasoning/assistant buffers for hook payloads.
        let pr = {
            let s = self.last_reasoning.lock().expect("lock poisoned").clone();
            if s.is_empty() { None } else { Some(s) }
        };
        let pa = {
            let s = self
                .last_assistant_text
                .lock()
                .expect("lock poisoned")
                .clone();
            if s.is_empty() { None } else { Some(s) }
        };

        // Refresh the grace period before execution so stale terminal events
        // (Esc, Ctrl+C) accumulated during the preflight approval loop do not
        // trigger a false cancellation during slow tool execution.
        self.cancel_turn
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.last_modal_close_ms.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            std::sync::atomic::Ordering::SeqCst,
        );

        // Execute read-only tools in parallel.
        let runtime = std::sync::Arc::new(
            cade_agent::tools::ToolRuntime::new(
                std::sync::Arc::new(self.client.clone()),
                std::sync::Arc::clone(&self.mcp),
                self.agent_id(),
                self.cwd.clone(),
            )
            .with_conversation(self.conversation_id())
            .with_backend(std::sync::Arc::clone(&self.exec_backend)),
        );

        if !read_indices.is_empty() {
            let mut handles = Vec::new();
            for &i in &read_indices {
                let (call_id, tool_name, args) = &tool_calls[i];
                let call_id = call_id.clone();
                let tool_name = tool_name.clone();
                let args = args.clone();
                let app_arc = self.app.clone();
                let mcp_arc = std::sync::Arc::clone(&self.mcp);
                let hooks = self.hooks.clone();
                let pr_c = pr.clone();
                let pa_c = pa.clone();
                let rt_c = std::sync::Arc::clone(&runtime);

                handles.push(tokio::spawn(async move {
                    let r = Self::run_tool_inner(
                        &call_id,
                        &tool_name,
                        &args,
                        &mcp_arc,
                        &hooks,
                        &app_arc,
                        &rt_c,
                        pr_c.as_deref(),
                        pa_c.as_deref(),
                    )
                    .await;
                    (i, r)
                }));
            }
            let join_results = futures::future::join_all(handles).await;
            for (i, r) in join_results.into_iter().flatten() {
                results[i] = r;
            }
            // Refresh grace period after parallel batch completes.
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.last_modal_close_ms.store(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                std::sync::atomic::Ordering::SeqCst,
            );
        }

        // Execute write tools sequentially.
        for &i in &write_indices {
            let (call_id, tool_name, args) = &tool_calls[i];
            let r = Self::run_tool_inner(
                call_id,
                tool_name,
                args,
                &self.mcp,
                &self.hooks,
                &self.app,
                &runtime,
                pr.as_deref(),
                pa.as_deref(),
            )
            .await;
            results[i] = r;
            // Refresh grace period after each write tool so the next tool (or
            // Phase 3 streaming) is protected from stale terminal events.
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.last_modal_close_ms.store(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                std::sync::atomic::Ordering::SeqCst,
            );
        }

        // Update stats.
        for r in &results {
            if let Ok(mut stats) = self.session_stats.lock() {
                stats.tool_calls_total += 1;
                if r.is_error {
                    stats.tool_calls_err += 1;
                } else {
                    stats.tool_calls_ok += 1;
                }
            }
        }

        // Clear any cancel flags accumulated during tool execution and
        // refresh the modal-close grace period so the tick task does not
        // re-set cancel_turn from a stale terminal event while the HTTP
        // connection for Phase 2 streaming is being established.
        self.cancel_turn
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.last_modal_close_ms.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            std::sync::atomic::Ordering::SeqCst,
        );

        // Phase 2: deposit all results to the server.  The first N-1 sends
        // return [] (server is still buffering); the Nth triggers the LLM and
        // streams back the assistant response with full context of all results.
        let mut follow = Vec::new();
        for result in &results {
            follow = self
                .stream_turn(
                    stdout,
                    "",
                    true,
                    &result.tool_call_id,
                    &result.output,
                    false,
                    None,
                    bar_text.clone(),
                )
                .await?;
        }

        Box::pin(self.dispatch_tool_calls(stdout, follow, user_input, bar_text, false, turn_stats))
            .await?;

        Ok(())
    }

    /// Check if a tool is a native intercept (requires &self). If so, execute
    /// it immediately and return the result. Returns None for generic tools.
    pub(crate) async fn try_native_intercept(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<Result<cade_agent::tools::ToolResult>> {
        match tool_name {
            "EnterPlanMode" => {
                self.permissions
                    .set_mode(cade_core::permissions::PermissionMode::Plan);
                let mut app = self.app.lock().expect("lock poisoned");
                app.update_mode(cade_core::permissions::PermissionMode::Plan);
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: "Plan mode entered. File modifications are now blocked.".to_string(),
                    is_error: false,
                }))
            }
            "ExitPlanMode" => {
                self.permissions
                    .set_mode(cade_core::permissions::PermissionMode::Default);
                let mut app = self.app.lock().expect("lock poisoned");
                app.update_mode(cade_core::permissions::PermissionMode::Default);
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: "Plan mode exited. Normal operation resumed.".to_string(),
                    is_error: false,
                }))
            }
            "run_subagent" => Some(self.handle_run_subagent(call_id, args).await),
            "ask_user_question" => Some(self.handle_ask_user_question(call_id, args).await),
            "message_agent" => Some(self.handle_message_agent(call_id, args).await),
            // Plan panel — require TuiApp access, intercepted before generic dispatch.
            "set_plan" => {
                let steps: Vec<String> = args["steps"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let n = steps.len();
                self.app.lock().expect("lock poisoned").set_plan(steps);
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: format!("Plan set with {n} step(s)."),
                    is_error: false,
                }))
            }
            "UpdatePlan" => {
                let step_id = args["step_id"].as_u64().unwrap_or(0) as usize;
                let done = args["done"].as_bool().unwrap_or(true);
                let found = self
                    .app
                    .lock()
                    .expect("lock poisoned")
                    .update_plan_step(step_id, done);
                Some(Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: if found {
                        format!(
                            "Step {step_id} marked {}.",
                            if done { "done" } else { "not done" }
                        )
                    } else {
                        format!("Step {step_id} not found in active plan.")
                    },
                    is_error: !found,
                }))
            }
            _ => None,
        }
    }

    /// Build a compact argument preview for a tool call header.
    pub(crate) fn tool_preview(_tool_name: &str, args: &serde_json::Value) -> String {
        fn short(s: &str, n: usize) -> String {
            let s = s.trim();
            if s.chars().count() <= n {
                s.to_string()
            } else {
                format!("{}…", s.chars().take(n).collect::<String>())
            }
        }
        let a = args;
        if let Some(cmd) = a["command"].as_str() {
            short(cmd, 80)
        } else if let Some(fp) = a["file_path"].as_str().or(a["path"].as_str()) {
            let extra = if let Some(old) = a["old_string"].as_str() {
                format!("  \"{}\"", short(old, 40))
            } else if let Some(content) = a["content"].as_str() {
                format!("  ({} chars)", content.len())
            } else {
                String::new()
            };
            format!("{fp}{extra}")
        } else if let Some(pat) = a["pattern"].as_str() {
            let in_path = a["path"].as_str().unwrap_or("");
            if in_path.is_empty() {
                format!("\"{}\"", short(pat, 60))
            } else {
                format!("\"{}\" in {in_path}", short(pat, 40))
            }
        } else if let Some(label) = a["label"].as_str() {
            let op = a["operation"].as_str().unwrap_or("set");
            format!("[{label}] ({op})")
        } else if let Some(patch) = a["patch"].as_str() {
            short(patch, 60)
        } else {
            a.as_object()
                .and_then(|m| m.values().find_map(|v| v.as_str()).map(|s| short(s, 60)))
                .unwrap_or_default()
        }
    }

    /// Phase 1: Sequential preflight — checks permissions, plan-mode blocking,
    /// hooks, and prompts the user for approval if needed.
    /// Returns `Approved` if the tool should proceed, or `Blocked(result)` if it
    /// was denied (with a pre-built error ToolResult).
    pub(crate) async fn preflight_tool(
        &self,
        stdout: &mut io::Stdout,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<ToolPreflightResult> {
        // Permission check — plan mode / deny rules
        if self.permissions.is_blocked(tool_name, args) {
            let msg = self.permissions.block_reason(tool_name, args);
            let _ = self
                .app
                .lock()
                .expect("lock poisoned")
                .push(RenderLine::ToolResult {
                    is_error: true,
                    content: msg.clone(),
                });
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            return Ok(ToolPreflightResult::Blocked(
                cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: msg,
                    is_error: true,
                },
            ));
        }

        if !self.permissions.auto_approve(tool_name, args) {
            // PermissionRequest hook — can block before showing prompt
            if let cade_core::hooks::HookOutcome::Block { reason } =
                self.hooks.permission_request(tool_name, args).await
            {
                let _ = self
                    .app
                    .lock()
                    .expect("lock poisoned")
                    .push(RenderLine::ToolResult {
                        is_error: true,
                        content: format!("Hook denied: {reason}"),
                    });
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                return Ok(ToolPreflightResult::Blocked(
                    cade_agent::tools::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: format!("Hook denied: {reason}"),
                        is_error: true,
                    },
                ));
            }

            // Prompt for approval
            if !self.prompt_approval(stdout, tool_name, args).await? {
                if let Ok(mut stats) = self.session_stats.lock() {
                    stats.reviewed += 1;
                }
                let msg = format!("Tool '{tool_name}' denied by user");
                let _ = self
                    .app
                    .lock()
                    .expect("lock poisoned")
                    .push(RenderLine::ToolResult {
                        is_error: true,
                        content: msg.clone(),
                    });
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                return Ok(ToolPreflightResult::Blocked(
                    cade_agent::tools::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: msg,
                        is_error: true,
                    },
                ));
            }
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            if let Ok(mut stats) = self.session_stats.lock() {
                stats.reviewed += 1;
                stats.approved += 1;
            }
        } else {
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.last_modal_close_ms.store(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                std::sync::atomic::Ordering::SeqCst,
            );
        }

        // PreToolUse hook — can block execution
        if let cade_core::hooks::HookOutcome::Block { reason } =
            self.hooks.pre_tool_use(tool_name, args).await
        {
            let _ = self
                .app
                .lock()
                .expect("lock poisoned")
                .push(RenderLine::ToolResult {
                    is_error: true,
                    content: format!("Hook blocked: {reason}"),
                });
            self.cancel_turn
                .store(false, std::sync::atomic::Ordering::SeqCst);
            return Ok(ToolPreflightResult::Blocked(
                cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    output: format!("Blocked by hook: {reason}"),
                    is_error: true,
                },
            ));
        }

        Ok(ToolPreflightResult::Approved)
    }

    /// Phase 2: Execute a single tool (no stdout, no approval — already preflighted).
    /// This is safe to call from `tokio::spawn` for parallel execution.
    pub(crate) async fn run_tool_inner(
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
        mcp: &std::sync::Arc<cade_agent::mcp::McpManager>,
        hooks: &cade_core::hooks::HookEngine,
        app: &std::sync::Arc<std::sync::Mutex<crate::ui::TuiApp>>,
        runtime: &std::sync::Arc<cade_agent::tools::ToolRuntime>,
        preceding_reasoning: Option<&str>,
        preceding_assistant_message: Option<&str>,
    ) -> cade_agent::tools::ToolResult {
        use cade_agent::tools::dispatch;

        // Bash tools — live-streaming path (buffered per-tool)
        if matches!(tool_name, "bash" | "run_command" | "execute_command") {
            let live_idx = app.lock().expect("lock poisoned").begin_live_output(8);
            let app_arc = app.clone();
            let run_result = cade_agent::tools::bash::BashTool::run_streaming(args, move |line| {
                let _ = app_arc
                    .lock()
                    .expect("lock poisoned")
                    .append_live_output_line(live_idx, line);
            })
            .await;
            let _ = app
                .lock()
                .expect("lock poisoned")
                .finish_live_output(live_idx);

            let (output, is_error) = match run_result {
                Ok(out) => (out, false),
                Err(e) => (format!("Error: {e}"), true),
            };

            let mut result = cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output,
                is_error,
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

        // Try ToolRuntime first (handles memory, skills, checkpoints, web, codeintel, etc.).
        // Fall back to native dispatch / MCP for tools ToolRuntime does not handle.
        const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
        let mut result = match tokio::time::timeout(
            TOOL_TIMEOUT,
            runtime.execute(call_id.to_string(), tool_name, args),
        )
        .await
        {
            Ok(Some(rt)) => cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: rt.output,
                is_error: rt.is_error,
            },
            Ok(None) => {
                // ToolRuntime returned None — interactive-only tool not handled there;
                // fall through to native dispatch / MCP.
                match tokio::time::timeout(
                    TOOL_TIMEOUT,
                    dispatch(call_id.to_string(), tool_name, args, mcp),
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
                            TOOL_TIMEOUT.as_secs()
                        ),
                        is_error: true,
                    },
                }
            }
            Err(_) => cade_agent::tools::ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: tool_name.to_string(),
                output: format!(
                    "Tool '{}' timed out after {}s",
                    tool_name,
                    TOOL_TIMEOUT.as_secs()
                ),
                is_error: true,
            },
        };

        if !result.is_error {
            match tool_name {
                "write_file" | "edit_file" | "apply_patch" | "Replace" | "WriteFileGemini" => {
                    let path = args["file_path"]
                        .as_str()
                        .or(args["path"].as_str())
                        .unwrap_or("unknown");
                    let msg = format!("Recently edited: {path}\n");
                    let c = runtime.client.clone();
                    let a = runtime.agent_id.clone();
                    tokio::spawn(async move {
                        let _ = c
                            .append_memory_with_limit(&a, "working_set", &msg, None, Some(3000))
                            .await;
                    });
                }
                _ => {}
            }
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
        let _ = app
            .lock()
            .expect("lock poisoned")
            .push(RenderLine::ToolResult {
                is_error: is_err,
                content,
            });
        result
    }

    /// Prompt the user to approve/deny a tool call.
    /// Returns true = approved, false = denied.
    ///
    /// Shows a ratatui inline menu with three options:
    ///   1. Yes — run once
    ///   2. Yes, don't ask again — session-allow + run
    ///   3. No — deny
    ///      Generate a diff preview for file-mutation tools shown before the approval prompt.
    pub(crate) fn build_diff_preview(
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<Vec<RenderLine>> {
        match tool_name {
            "edit_file" => {
                let path = args["path"].as_str()?;
                let old_string = args["old_string"].as_str()?;
                let new_string = args["new_string"].as_str()?;
                let existing = std::fs::read_to_string(path).ok()?;
                let offset = existing
                    .find(old_string)
                    .map(|byte| existing[..byte].lines().count())
                    .unwrap_or(0);
                let mut out: Vec<RenderLine> = vec![RenderLine::DimMsg(format!("--- {path}"))];
                for (i, ln) in old_string.lines().enumerate() {
                    out.push(RenderLine::ErrorMsg(format!(
                        "- {ln}  (L{})",
                        offset + i + 1
                    )));
                }
                for ln in new_string.lines() {
                    out.push(RenderLine::SuccessMsg(format!("+ {ln}")));
                }
                Some(out)
            }
            "write_file" | "create_file" => {
                let path = args["path"].as_str()?;
                let content = args["content"].as_str()?;
                let is_new = !std::path::Path::new(path).exists();
                let lines: Vec<&str> = content.lines().collect();
                let show = lines.len().min(12);
                let mut out: Vec<RenderLine> = vec![RenderLine::DimMsg(format!(
                    "{} {path}",
                    if is_new { "new file:" } else { "overwrite:" }
                ))];
                for ln in &lines[..show] {
                    out.push(RenderLine::SuccessMsg(format!("+ {ln}")));
                }
                if lines.len() > show {
                    out.push(RenderLine::DimMsg(format!(
                        "  … ({} more lines)",
                        lines.len() - show
                    )));
                }
                Some(out)
            }
            "apply_patch" => {
                let patch = args["patch"].as_str()?;
                let mut out: Vec<RenderLine> = vec![RenderLine::DimMsg("(patch)".to_string())];
                for ln in patch.lines().take(20) {
                    if ln.starts_with('-') && !ln.starts_with("---") {
                        out.push(RenderLine::ErrorMsg(ln.to_string()));
                    } else if ln.starts_with('+') && !ln.starts_with("+++") {
                        out.push(RenderLine::SuccessMsg(ln.to_string()));
                    } else {
                        out.push(RenderLine::DimMsg(ln.to_string()));
                    }
                }
                if patch.lines().count() > 20 {
                    out.push(RenderLine::DimMsg(format!(
                        "… ({} more lines)",
                        patch.lines().count() - 20
                    )));
                }
                Some(out)
            }
            _ => None,
        }
    }

    pub(crate) async fn prompt_approval(
        &self,
        _stdout: &mut io::Stdout,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<bool> {
        use crate::ui::question::{Question, QuestionOption};

        // Show diff preview for file-mutation tools before the approval prompt.
        if let Some(diff_lines) = Self::build_diff_preview(tool_name, args) {
            let mut app = self.app.lock().expect("lock poisoned");
            for line in diff_lines {
                let _ = app.push(line);
            }
            let _ = app.draw();
        }

        // One-line preview of what is being requested
        let preview: String = if let Some(cmd) = args["command"].as_str() {
            truncate(cmd, 100).to_string()
        } else if let Some(fp) = args["file_path"].as_str().or(args["path"].as_str()) {
            fp.to_string()
        } else if let Some(pat) = args["pattern"].as_str() {
            format!("\"{}\"", truncate(pat, 60))
        } else {
            String::new()
        };

        // Header chip — tool name, max 12 chars
        let header_raw = tool_name.replace('_', " ");
        let header: String = header_raw.chars().take(12).collect();

        let mut warning_text = String::new();
        if tool_name == "bash"
            && let Some(cmd) = args["command"].as_str()
            && cade_core::permissions::bash_command_is_suspicious(cmd)
        {
            warning_text = "\n⚠️  WARNING: Suspicious command detected (nested shell, network, or obfuscation)".to_string();
        }

        let question_text = if preview.is_empty() {
            format!("Run {tool_name}?{warning_text}")
        } else {
            format!("{preview}{warning_text}")
        };

        let opts = vec![
            QuestionOption {
                label: "Yes".to_string(),
                description: "Run this tool once".to_string(),
            },
            QuestionOption {
                label: "Yes, don't ask again".to_string(),
                description: "Allow this tool for the rest of the session".to_string(),
            },
            QuestionOption {
                label: "No".to_string(),
                description: "Deny this tool call".to_string(),
            },
        ];

        let q = Question {
            header: header.clone(),
            text: question_text.clone(),
            options: opts.clone(),
            multi_select: false,
            allow_other: false,
            progress: None,
        };

        #[allow(deprecated)]
        let rx = {
            let mut app = self.app.lock().expect("lock poisoned");
            app.ask_question_async(q)?
        };

        let qa = rx
            .await
            .map_err(|e| crate::Error::custom(format!("approval channel dropped: {e}")))?;
        // Record close time so the tick task's I-01 Enter handler can apply
        // a 300 ms grace period (mirrors the 200 ms Esc grace period).
        self.last_modal_close_ms.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            std::sync::atomic::Ordering::SeqCst,
        );

        match qa {
            None => {
                // Esc / Ctrl+C = deny. Clear any cancel flag set while the
                // blocking question was active — an Esc inside the modal must
                // not abort the subsequent stream_turn.
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                Ok(false)
            }
            Some(answer) => {
                let label = answer.as_str();
                // Clear any stale SIGINT cancel flag set while the blocking
                // event loop ran (terminal may have converted Ctrl+Enter or
                // a buffered Esc into an OS-level interrupt during the modal).
                // Without this reset the next stream_turn would see
                // cancel_turn == true and immediately abort with "Turn interrupted".
                self.cancel_turn
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                if label.starts_with("Yes, don't") {
                    // Store allow rule BEFORE returning so that any immediately
                    // following tool call of the same type is auto-approved (B3).
                    self.permissions.add_session_allow(tool_name);
                    Ok(true)
                } else if label.starts_with("Yes") {
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// C3: Inject a one-time ephemeral reminder prompting the agent to fill its
    /// `working_set` memory block after significant file-write activity.
    ///
    /// Only fires when the block is actually empty so the model is not nagged
    /// when it has already been diligently updating its own memory.
    pub(crate) async fn inject_working_set_reminder(
        &mut self,
        stdout: &mut io::Stdout,
    ) -> Result<()> {
        let agent_id = self.agent_id();

        // Fetch the current working_set value — one async call, performed once
        // per session at most.
        let is_empty = self
            .client
            .get_memory(&agent_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|b| b.label == "working_set")
            .map(|b| b.value.trim().is_empty())
            .unwrap_or(true);

        if !is_empty {
            // Already populated — no reminder needed.
            return Ok(());
        }

        let reminder = "[System: You have made several file changes this session. \
            Your `working_set` memory block is currently empty. \
            Please call update_memory now with label='working_set' and a value that records: \
            (1) the current task / goal, \
            (2) files you have modified, \
            (3) your immediate next steps. \
            Keep it under 200 words. This block persists when older context is dropped.]";

        tracing::debug!(
            "Injecting working_set reminder (write_tool_calls={})",
            self.write_tool_calls
                .load(std::sync::atomic::Ordering::SeqCst)
        );

        // Send as an ephemeral user message so it is not stored in the
        // conversation history but the agent still sees it and can respond
        // with an update_memory call.
        let msgs = self
            .stream_turn(stdout, reminder, false, "", "", true, None, None)
            .await?;

        // Dispatch any tool calls the model makes in response (usually update_memory).
        // reprompt_done=true prevents re-entry loops.
        let mut turn_stats = TurnStats::default();
        Box::pin(self.dispatch_tool_calls(stdout, msgs, "", None, true, &mut turn_stats)).await
    }

    /// Interactive `ask_user_question` tool intercept.
    ///
    /// Parses the LLM's structured questions, shows the `QuestionWidget` for
    /// each one sequentially, then returns a formatted result string to the agent.
    pub(crate) async fn handle_ask_user_question(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<cade_agent::tools::ToolResult> {
        use crate::ui::question::{Question, QuestionOption};
        use cade_agent::tools::AskUserQuestionTool;
        use std::collections::HashMap;

        // Parse and validate
        let ask_questions = match AskUserQuestionTool::parse_questions(args) {
            Ok(q) => q,
            Err(e) => {
                let msg = format!("Invalid ask_user_question args: {e}");
                let _ = self
                    .app
                    .lock()
                    .expect("lock poisoned")
                    .push(RenderLine::ToolResult {
                        is_error: true,
                        content: msg.clone(),
                    });
                return Ok(cade_agent::tools::ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "ask_user_question".to_string(),
                    output: msg,
                    is_error: true,
                });
            }
        };

        let total = ask_questions.len();
        let _ = self.app.lock().expect("lock poisoned").commit_streaming();

        let mut answers: HashMap<String, String> = HashMap::new();
        let mut answers_display: Vec<(String, String)> = Vec::new();

        for (i, aq) in ask_questions.iter().enumerate() {
            let opts: Vec<QuestionOption> = aq
                .options
                .iter()
                .map(|o| QuestionOption {
                    label: o.label.clone(),
                    description: o.description.clone(),
                })
                .collect();

            let q = Question {
                header: aq.header.clone(),
                text: aq.question.clone(),
                options: opts.clone(),
                multi_select: aq.multi_select,
                allow_other: true,
                progress: if total > 1 {
                    Some((i + 1, total))
                } else {
                    None
                },
            };

            // Use ask_question_async to avoid blocking the main event loop
            // while awaiting user input. The app mutex is released during await.
            #[allow(deprecated)]
            let rx = {
                let mut app = self.app.lock().expect("lock poisoned");
                app.ask_question_async(q)?
            };

            let qa = rx.await.map_err(|e| {
                crate::Error::custom(format!("ask_user_question channel dropped: {e}"))
            })?;

            self.last_modal_close_ms.store(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                std::sync::atomic::Ordering::SeqCst,
            );

            match qa {
                None => {
                    // User cancelled — clear any stale cancel flag so subsequent
                    // stream_turn calls are not aborted immediately.
                    self.cancel_turn
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    let msg = "User cancelled the question prompt.".to_string();
                    let _ = self
                        .app
                        .lock()
                        .expect("lock poisoned")
                        .push(RenderLine::ToolResult {
                            is_error: true,
                            content: msg.clone(),
                        });
                    return Ok(cade_agent::tools::ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "ask_user_question".to_string(),
                        output: msg,
                        is_error: true,
                    });
                }
                Some(answer) => {
                    answers_display.push((aq.header.clone(), answer.as_str()));
                    answers.insert(aq.question.clone(), answer.as_str());
                }
            }
        }

        // Show answers inline under the tool call header (⎿ answer / ⎿ h: a\n  h: b)
        let result_content = if total == 1 {
            answers_display[0].1.clone()
        } else {
            answers_display
                .iter()
                .map(|(h, a)| format!("{h}: {a}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        // Clear any stale cancel flag accumulated during the question loop so
        // the following stream_turn is not aborted prematurely.
        self.cancel_turn
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Removed internal ToolResult push since dispatch_tool_calls pushes it unconditionally.
        {
            let mut app = self.app.lock().expect("lock poisoned");
            // Force a redraw to ensure the viewport updates immediately after the
            // question modal is dismissed, fixing a race condition where the
            // result of the next tool call would not be displayed.
            let _ = app.draw();
        }

        Ok(cade_agent::tools::ToolResult {
            tool_call_id: call_id.to_string(),
            tool_name: "ask_user_question".to_string(),
            output: result_content,
            is_error: false,
        })
    }
}
