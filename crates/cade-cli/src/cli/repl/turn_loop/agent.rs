use super::Repl;
use crate::Result;
use crate::ui::RenderLine;
use cade_agent::agent::client::CadeMessage;
use std::io;

// ── First-turn `active_goal` reminder (pure) ─────────────────────────────────
//
// On the first user turn of every REPL session, if the agent's `active_goal`
// memory block is non-empty we prepend a short system-style reminder to the
// effective input.  This nudges the agent to *verify* the stored plan is
// still current before resuming work — a long-idle session can leave a
// `active_goal` from a task the user has long since moved on from.

/// Build the staleness-check reminder appended to the first turn's effective
/// input.  Returns `None` when the value is empty/whitespace — no plan to
/// verify, so no reminder is emitted.
///
/// Returns a `<system>...</system>` snippet for embedding inside the larger
/// `effective_input` string built by `agent_turn`.
pub(crate) fn build_active_goal_first_turn_reminder(active_goal_value: &str) -> Option<String> {
    if active_goal_value.trim().is_empty() {
        return None;
    }
    Some(
        "<system>You have an `active_goal` memory block from a previous session. \
Before acting on it, briefly verify it is still the user's current task. \
If the user's first message implies a different task, call \
update_memory(label='active_goal', value=...) with the new plan before \
running any write tools.</system>"
            .to_string(),
    )
}

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
            // Look up active_goal once; if it is non-empty, prepend a staleness
            // verification reminder so the agent does not silently resume a
            // long-idle plan from a previous session.
            let active_goal_val = self
                .client
                .get_memory(&self.agent_id())
                .await
                .unwrap_or_default()
                .into_iter()
                .find(|b| b.label == "active_goal")
                .map(|b| b.value)
                .unwrap_or_default();
            let staleness = build_active_goal_first_turn_reminder(&active_goal_val)
                .map(|s| format!("\n\n{s}"))
                .unwrap_or_default();
            format!(
                "{env}\n\n<system>Do not introduce yourself. Answer the user's message directly.</system>{staleness}\n\n{input}"
            )
        } else {
            input.to_string()
        };

        // -- Skill trigger auto-detection
        // If the input matches any skill trigger, notify the server to load the
        // skill (server-side injection handles the actual context injection).
        {
            let skills = self.skills.lock();
            let triggered_ids: Vec<String> = skills
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
                    s.id.clone()
                })
                .collect();
            drop(skills);

            // Load triggered skills server-side (fire-and-forget)
            for skill_id in &triggered_ids {
                let agent_id = self.agent_id();
                let _ = self.client.load_skill_on_server(&agent_id, skill_id).await;
                tracing::info!("Auto-loaded skill '{}' server-side", skill_id);
            }
        }

        let mut director = super::TurnDirector::new(self);
        let outcome = director
            .execute_turn(stdout, &effective_input, turn_start, out_tok_before)
            .await?;

        match outcome {
            super::TurnOutcome::Cancelled => {
                self.turn_active.store(false, Ordering::SeqCst);
                return Ok(());
            }
            super::TurnOutcome::Error(err) => {
                self.turn_active.store(false, Ordering::SeqCst);
                self.app
                    .lock()
                    .set_last_status(Some(format!("Error: {err}")));
                return Ok(());
            }
            super::TurnOutcome::Completed { .. } => {}
        }

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

    /// Commit any in-progress streaming/reasoning, push an error line, reset
    /// the status indicator bar, and return an empty message vec.
    /// Shared cleanup path for stream errors to prevent frozen turn states.
    pub(crate) fn abort_stream_ui(&self, msg: impl Into<String>) -> Vec<CadeMessage> {
        let err_text = msg.into();
        let mut app = self.app.lock();
        let _ = app.commit_reasoning();
        let _ = app.commit_streaming();
        app.show_toast(err_text.clone(), cade_tui::app::ToastLevel::Error);
        let _ = app.push(RenderLine::ErrorMsg(err_text.clone()));
        app.set_last_status(Some(format!("✗ Error: {err_text}")));
        app.notify_if_unfocused(
            cade_tui::app::notifier::AttentionCue::TaskError,
            "Turn Error",
            &err_text,
        );
        app.draw_dirty = true;
        let _ = app.draw();
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_reminder_when_active_goal_empty() {
        assert!(build_active_goal_first_turn_reminder("").is_none());
    }

    #[test]
    fn no_reminder_when_active_goal_whitespace() {
        assert!(build_active_goal_first_turn_reminder("   \n\t  ").is_none());
    }

    #[test]
    fn reminder_present_when_active_goal_has_content() {
        let s = build_active_goal_first_turn_reminder("Working on M1.")
            .expect("non-empty active_goal must produce a reminder");
        assert!(s.contains("<system>"));
        assert!(s.contains("</system>"));
        assert!(s.contains("active_goal"));
        assert!(s.contains("verify"));
    }

    #[test]
    fn reminder_does_not_leak_active_goal_value_into_string() {
        // The reminder is generic instructions; it must not embed the stored
        // plan verbatim (the agent already sees it via the memory block).
        let s = build_active_goal_first_turn_reminder("super-secret-plan-xyz").expect("present");
        assert!(!s.contains("super-secret-plan-xyz"));
    }

    #[test]
    #[ignore = "requires tty"]
    fn abort_stream_ui_resets_spinner_and_adds_error_line() {
        let app = std::sync::Arc::new(parking_lot::Mutex::new(cade_tui::app::TuiApp::new(
            cade_core::permissions::PermissionMode::Default,
            "test_agent".into(),
            "test_model".into(),
            None,
        )));
        let repl_app = app.clone();
        let err_msg = "Upstream model rejected with HTTP 404";
        {
            let mut a = repl_app.lock();
            a.set_last_status(Some("generating...".into()));
        }
        // Emulate abort_stream_ui logic directly on TuiApp
        {
            let mut a = repl_app.lock();
            let _ = a.commit_reasoning();
            let _ = a.commit_streaming();
            a.show_toast(err_msg.to_string(), cade_tui::app::ToastLevel::Error);
            let _ = a.push(RenderLine::ErrorMsg(err_msg.to_string()));
            a.set_last_status(Some(format!("✗ Error: {err_msg}")));
        }
        let a = repl_app.lock();
        assert_eq!(a.last_status, Some(format!("✗ Error: {err_msg}")));
        assert!(a.lines.iter().any(|line| match line {
            RenderLine::ErrorMsg(s) => s.contains("Upstream model rejected"),
            _ => false,
        }));
    }

    #[test]
    #[ignore = "requires tty"]
    fn test_tui_adapter_renders_canonical_event_stream() {
        let app = std::sync::Arc::new(parking_lot::Mutex::new(cade_tui::app::TuiApp::new(
            cade_core::permissions::PermissionMode::Default,
            "test_agent".into(),
            "test_model".into(),
            None,
        )));

        // User typed something into editor before stream arrived
        {
            let mut a = app.lock();
            a.editor.set_text("local draft prompt".to_string());
            assert_eq!(a.editor.text(), "local draft prompt");
        }

        // Simulate recorded canonical event stream delivery
        let events = vec![
            serde_json::json!({
                "message_type": "stream_start",
                "conversation_id": "conv-1",
                "run_id": "run-42",
                "seq_id": 0
            }),
            serde_json::json!({
                "message_type": "reasoning_message",
                "content": "Thinking deeply...",
                "run_id": "run-42",
                "seq_id": 1
            }),
            serde_json::json!({
                "message_type": "assistant_message",
                "content": "Here is the plan.",
                "run_id": "run-42",
                "seq_id": 2
            }),
            serde_json::json!({
                "message_type": "tool_call_message",
                "tool_call": {
                    "id": "call_1",
                    "name": "bash",
                    "arguments": {}
                },
                "run_id": "run-42",
                "seq_id": 3
            }),
            serde_json::json!({
                "message_type": "run_done",
                "status": "done",
                "run_id": "run-42",
                "seq_id": 4
            }),
        ];

        // Process canonical events through TuiApp
        for event in events {
            let msg: cade_agent::agent::client::CadeMessage =
                serde_json::from_value(event).expect("valid test json");
            let mut a = app.lock();
            match msg.msg_type() {
                "reasoning_message" => {
                    if let Some(text) = msg.reasoning_text() {
                        a.push_reasoning_chunk(text);
                    }
                }
                "assistant_message" => {
                    if let Some(text) = msg.assistant_text() {
                        let _ = a.push_streaming_chunk(text);
                    }
                }
                "tool_call_message" => {
                    a.commit_reasoning_inner();
                    let _ = a.commit_streaming();
                    if let Some((_, tool_name, _)) = msg.as_tool_call() {
                        a.set_last_status(Some(format!("● {tool_name}…")));
                    }
                }
                "run_done" => {
                    let _ = a.commit_reasoning();
                    let _ = a.commit_streaming();
                    a.set_last_status(Some("✓ Finished".to_string()));
                }
                _ => {}
            }
        }

        // Verify TuiApp state
        let a = app.lock();
        // 1. Editor local draft state was NOT corrupted by incoming stream
        assert_eq!(a.editor.text(), "local draft prompt");
        // 2. Status was updated
        assert_eq!(a.last_status, Some("✓ Finished".to_string()));
        // 3. Lines contain the committed streaming content
        assert!(a.lines.iter().any(|line| match line {
            RenderLine::AssistantText(s) => s.contains("Here is the plan."),
            _ => false,
        }));
        // 4. Reasoning streamed through the viewport is committed as a
        //    Reasoning block (never hidden in the bottom bar).
        assert!(a.lines.iter().any(|line| match line {
            RenderLine::Reasoning { content, .. } => content.contains("Thinking deeply..."),
            _ => false,
        }));
    }
}
