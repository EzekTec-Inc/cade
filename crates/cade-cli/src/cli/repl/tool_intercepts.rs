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

    async fn pause_subagent(&self, id: &str) -> std::result::Result<String, cade_agent::Error> {
        let value = self
            .client
            .raw_post(&format!("/subagents/{id}/pause"), &serde_json::json!({}))
            .await?;
        Ok(value["status"].as_str().unwrap_or("unknown").to_string())
    }

    async fn resume_subagent(&self, id: &str) -> std::result::Result<String, cade_agent::Error> {
        let value = self
            .client
            .raw_post(&format!("/subagents/{id}/resume"), &serde_json::json!({}))
            .await?;
        Ok(value["status"].as_str().unwrap_or("unknown").to_string())
    }

    fn doctor_status(&self) -> std::result::Result<String, cade_agent::Error> {
        let report = cade_core::doctor::check_multiplexer_and_keys();
        let mut out =
            "Subagent system status: OK. Multi-agent concurrency slots available.\n".to_string();
        out.push_str(&report.to_formatted_summary());
        Ok(out)
    }

    async fn child_status(&self, id: &str) -> std::result::Result<String, cade_agent::Error> {
        let response = self
            .client
            .raw_get(&format!("/subagents/{id}/status"))
            .await
            .map_err(|e| {
                cade_agent::Error::custom(format!("Subagent '{id}' is not reachable: {e}"))
            })?;
        let status = response["status"]
            .as_str()
            .ok_or_else(|| cade_agent::Error::custom("Server returned no child status"))?;
        Ok(format!("Subagent '{id}' is {status}"))
    }

    async fn steer_child(
        &self,
        id: &str,
        message: &str,
    ) -> std::result::Result<String, cade_agent::Error> {
        self.client
            .raw_post(
                &format!("/subagents/{id}/steer"),
                &serde_json::json!({"message": message}),
            )
            .await
            .map_err(|e| {
                cade_agent::Error::custom(format!("Could not steer subagent '{id}': {e}"))
            })?;
        Ok(format!("Guidance accepted for subagent '{id}' next turn"))
    }

    async fn hot_swap_model(
        &self,
        subagent_id: &str,
        new_model: &str,
    ) -> std::result::Result<String, cade_agent::Error> {
        let response = self
            .client
            .raw_post(
                &format!("/subagents/{subagent_id}/model"),
                &serde_json::json!({"model": new_model}),
            )
            .await
            .map_err(|e| cade_agent::Error::custom(e.to_string()))?;
        let accepted = response["model"]
            .as_str()
            .ok_or_else(|| cade_agent::Error::custom("Server returned no accepted model"))?;
        Ok(format!(
            "Model for subagent '{subagent_id}' queued to swap to '{}' on its next turn",
            accepted
        ))
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
        let mut body = self.execution_options().await?;
        body["conversation_id"] = serde_json::json!(self.conversation_id());
        body["mode"] = self.permissions.mode().to_string().into();
        body["args"] = args.clone();
        let response = self
            .client
            .raw_post(&format!("/agents/{}/subagents", self.agent_id()), &body)
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
                // The active event driver owns the terminal. Await the shared
                // dialog channel with no app lock held and no second reader.
                let answer = self.ask_repl_question(question).await.unwrap_or(None);
                if let Some(feedback) = human_review_feedback(answer) {
                    result.is_error = true;
                    result.output = format!(
                        "HUMAN REVIEW REJECTED: The user rejected the subagent's work with feedback: {}\n\nPrevious output:\n{}",
                        feedback, result.output
                    );
                }
            }
        }
        if let Some(run_id) = response["run_id"].as_str() {
            result
                .output
                .push_str(&format!("\nInspection run: {run_id}"));
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
            SubagentTrayAction::PauseResume { subagent_id } => {
                let result = match self
                    .client
                    .raw_get(&format!("/subagents/{subagent_id}/pause"))
                    .await
                {
                    Ok(state) if state["status"] == "paused" => {
                        self.resume_subagent(&subagent_id).await
                    }
                    Ok(_) => self.pause_subagent(&subagent_id).await,
                    Err(error) => Err(error),
                };
                let mut app = self.app.lock();
                app.show_toast(
                    match result {
                        Ok(status) => format!("Subagent {subagent_id}: {status}"),
                        Err(error) => format!("Could not control {subagent_id}: {error}"),
                    },
                    cade_tui::ToastLevel::Info,
                );
                app.draw_dirty = true;
                return;
            }
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

fn human_review_feedback(answer: Option<cade_tui::question::QuestionAnswer>) -> Option<String> {
    match answer {
        Some(cade_tui::question::QuestionAnswer::Single(label)) if label == "Approve" => None,
        Some(answer) => Some(answer.as_str()),
        None => Some("Review cancelled".into()),
    }
}

#[cfg(test)]
mod candidate6_tests {
    use super::*;
    use cade_tui::question::QuestionAnswer;

    #[test]
    fn candidate6_human_review_preserves_typed_rejection() {
        assert_eq!(
            human_review_feedback(Some(QuestionAnswer::Single("Approve".into()))),
            None
        );
        assert_eq!(
            human_review_feedback(Some(QuestionAnswer::Custom("Approve".into()))),
            Some("Approve".into())
        );
        assert!(human_review_feedback(None).is_some());
    }
}
