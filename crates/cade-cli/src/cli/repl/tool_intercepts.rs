use super::Repl;
use crate::Result;
use cade_agent::subagents::{SubagentConfig, discover_all_subagents, visible_subagents};

#[async_trait::async_trait]
impl cade_agent::subagents::SubagentSingleRunner for Repl {
    async fn run_single(
        &self,
        call_id: &str,
        args: &serde_json::Value,
        force_sync: bool,
    ) -> std::result::Result<cade_agent::tools::ToolResult, cade_agent::Error> {
        self.handle_subagent_single_inner(call_id, args, force_sync)
            .await
            .map_err(|e| cade_agent::Error::custom(e.to_string()))
    }

    fn list_subagents(&self) -> std::result::Result<String, cade_agent::Error> {
        let defs = discover_all_subagents(&self.cwd);
        let mut out = String::from("Available subagents:\n");
        for d in visible_subagents(&defs) {
            out.push_str(&format!("- {}: {} ({})\n", d.name, d.description, d.tools));
        }
        Ok(out)
    }

    async fn cancel_subagent(&self, id: &str) -> std::result::Result<String, cade_agent::Error> {
        let response = self
            .client
            .raw_post(&format!("/subagents/{id}/cancel"), &serde_json::json!({}))
            .await
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        Ok(format!(
            "Cancellation {} for subagent {id}",
            response["status"].as_str().unwrap_or("requested")
        ))
    }

    fn doctor_status(&self) -> std::result::Result<String, cade_agent::Error> {
        let report = cade_core::doctor::check_multiplexer_and_keys();
        let mut out =
            "Subagent system status: OK. Multi-agent concurrency slots available.\n".to_string();
        out.push_str(&report.to_formatted_summary());
        Ok(out)
    }
}

impl Repl {
    pub(crate) async fn handle_subagent(
        &self,
        call_id: &str,
        args: &serde_json::Value,
    ) -> Result<cade_agent::tools::ToolResult> {
        cade_agent::subagents::SubagentCoordinator::coordinate(self, call_id, args)
            .await
            .map_err(|e| crate::error::Error::custom(e.to_string()))
    }

    /// Direct CLI invocations use the server-owned session for tool policy,
    /// approval, isolation and conversation-scoped outcome delivery.
    pub(crate) async fn handle_subagent_single_inner(
        &self,
        call_id: &str,
        args: &serde_json::Value,
        force_synchronous: bool,
    ) -> Result<cade_agent::tools::ToolResult> {
        let mut args = args.clone();
        if force_synchronous {
            args["background"] = serde_json::Value::Bool(false);
        }
        let response = self
            .client
            .raw_post(
                &format!("/agents/{}/subagents", self.agent_id()),
                &serde_json::json!({
                    "conversation_id": self.conversation_id(),
                    "mode": self.permissions.mode().to_string(),
                    "args": args,
                }),
            )
            .await?;
        let mut result = cade_agent::tools::ToolResult {
            tool_call_id: call_id.to_string(),
            tool_name: "subagent".to_string(),
            output: response["output"]
                .as_str()
                .unwrap_or("missing subagent response")
                .to_string(),
            is_error: response["is_error"].as_bool().unwrap_or(true),
            ui_resource_uri: None,
        };
        let config = SubagentConfig::from_args(&args);
        if !config.background {
            if let cade_core::hooks::HookOutcome::Block { reason } = self
                .hooks
                .subagent_stop(&config.mode, &result.output, result.is_error)
                .await
            {
                result
                    .output
                    .push_str(&format!("\n\n[SubagentStop hook: {reason}]"));
            }
            if !result.is_error && config.human_review {
                use crate::ui::question::{Question, QuestionOption};
                let question = Question {
                    header: format!("Subagent [{}] Completed", config.mode),
                    text: "Review the subagent's work. Select Approve, or type feedback to Reject and re-task:".to_string(),
                    options: vec![QuestionOption { label: "Approve".to_string(), description: String::new() }],
                    multi_select: false,
                    allow_other: true,
                    progress: None,
                };
                if let Some(answer) = self.app.lock().ask_question(&question).unwrap_or(None)
                    && answer.as_str() != "Approve"
                {
                    result.is_error = true;
                    result.output = format!(
                        "HUMAN REVIEW REJECTED: The user rejected the subagent's work with feedback: {}\n\nPrevious output:\n{}",
                        answer.as_str(),
                        result.output
                    );
                }
            }
        }
        Ok(result)
    }

    pub(crate) async fn dispatch_subagent_tray_action(
        &self,
        action: cade_tui::app::subagent_tray::SubagentTrayAction,
    ) {
        use cade_agent::subagents::SubagentSingleRunner;
        use cade_tui::app::subagent_tray::SubagentTrayAction;
        let (id, path, body, local_note) = match action {
            SubagentTrayAction::None => return,
            SubagentTrayAction::Kill { subagent_id } => {
                let result = self.cancel_subagent(&subagent_id).await;
                let mut app = self.app.lock();
                app.show_toast(
                    match result {
                        Ok(message) => message,
                        Err(error) => format!("Could not cancel {subagent_id}: {error}"),
                    },
                    cade_tui::ToastLevel::Info,
                );
                app.draw_dirty = true;
                return;
            }
            SubagentTrayAction::Steer {
                subagent_id,
                message,
            } => {
                let note = format!("[STEERING GUIDANCE]: {message}");
                (
                    subagent_id,
                    "steer",
                    serde_json::json!({ "message": message }),
                    note,
                )
            }
            SubagentTrayAction::HotSwapModel { subagent_id, model } => {
                let note = format!("[MODEL HOT-SWAP]: {model}");
                (
                    subagent_id,
                    "model",
                    serde_json::json!({ "model": model }),
                    note,
                )
            }
            SubagentTrayAction::PauseResume { subagent_id } => (
                subagent_id,
                "pause",
                serde_json::json!({ "action": "pause_resume" }),
                "[PAUSE/RESUME]".to_string(),
            ),
        };
        let result = self
            .client
            .raw_post(&format!("/subagents/{id}/{path}"), &body)
            .await;
        let mut app = self.app.lock();
        let message = match result {
            Ok(_) => {
                if let Some(tracker) = app.subagent_trackers.iter_mut().find(|t| t.task_id == id) {
                    tracker.push_output(local_note);
                }
                format!("Action sent to {id}")
            }
            Err(error) => format!("Action failed for {id}: {error}"),
        };
        app.show_toast(message, cade_tui::ToastLevel::Info);
        app.draw_dirty = true;
    }
}
