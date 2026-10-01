//! Tool execution dispatcher for the agentic loop.

use super::{SseTx, subagent};
use crate::server::state::AppState;
use cade_agent::tools::{ToolPipeline, manager::ToolResult};
use cade_ai::LlmToolCall;
use serde_json::{Value, json};
use std::sync::Arc;

/// Recursively substitutes placeholders in serde_json::Value.
fn substitute_step_arguments(args: &mut Value, step_results: &[ToolResult]) {
    match args {
        Value::String(s) => {
            static RE: std::sync::LazyLock<Option<regex::Regex>> =
                std::sync::LazyLock::new(|| regex::Regex::new(r#"\$steps\.(\d+)\.output"#).ok());
            if let Some(re) = RE.as_ref()
                && let Some(caps) = re.captures(s)
                && let Some(index_match) = caps.get(1)
                && let Ok(index) = index_match.as_str().parse::<usize>()
                && let Some(prev_result) = step_results.get(index)
            {
                *s = prev_result.output.clone();
            }
        }
        Value::Array(arr) => {
            for val in arr {
                substitute_step_arguments(val, step_results);
            }
        }
        Value::Object(map) => {
            for (_, val) in map {
                substitute_step_arguments(val, step_results);
            }
        }
        _ => {}
    }
}

/// Executes a sequential workflow defined by the `run_sequential_tasks` tool.
async fn handle_sequential_workflow(
    tool_call_id: String,
    arguments: Value,
    pipeline: Arc<ToolPipeline>,
) -> ToolResult {
    let steps = match arguments.get("steps").and_then(|s| s.as_array()) {
        Some(s) => s,
        None => {
            return ToolResult {
                tool_call_id: tool_call_id.to_string(),
                tool_name: "run_sequential_tasks".to_string(),
                output: "Error: 'steps' array not found in arguments.".to_string(),
                is_error: true,
                ui_resource_uri: None,
            };
        }
    };

    let mut step_results = Vec::new();
    let mut aggregated_output = String::new();

    for (i, step) in steps.iter().enumerate() {
        let tool_name = match step.get("tool_name").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => {
                aggregated_output.push_str(&format!(
                    "\n--- Step {} Failed: 'tool_name' not found. ---",
                    i
                ));
                break;
            }
        };

        let mut step_args = match step.get("arguments") {
            Some(a) => a.clone(),
            None => json!({}),
        };

        substitute_step_arguments(&mut step_args, &step_results);

        let step_tool_call_id = format!("{}-step-{}", tool_call_id, i);

        let result_to_store = match pipeline
            .execute(&step_tool_call_id, tool_name, &step_args)
            .await
        {
            Ok(outcome) => ToolResult {
                tool_call_id: outcome.tool_call_id,
                tool_name: outcome.tool_name,
                output: outcome.output,
                is_error: outcome.is_error,
                ui_resource_uri: outcome.ui_resource_uri,
            },
            Err(error) => ToolResult {
                tool_call_id: step_tool_call_id,
                tool_name: tool_name.to_string(),
                output: format!("Tool execution error: {error}"),
                is_error: true,
                ui_resource_uri: None,
            },
        };

        if !aggregated_output.is_empty() {
            aggregated_output.push_str("\n---\n");
        }
        aggregated_output.push_str(&format!(
            "Step {}: {} ->\n{}",
            i, result_to_store.tool_name, result_to_store.output
        ));

        let is_error = result_to_store.is_error;
        step_results.push(result_to_store);

        if is_error {
            break;
        }
    }

    ToolResult {
        tool_call_id: tool_call_id.to_string(),
        tool_name: "run_sequential_tasks".to_string(),
        output: aggregated_output,
        is_error: step_results.last().is_some_and(|r| r.is_error),
        ui_resource_uri: None,
    }
}

async fn emit_tool_progress(
    database: &cade_store::sqlite::Db,
    run_id: &str,
    tx: &SseTx,
    payload: Value,
) {
    let serialized = payload.to_string();
    let mut envelope = payload;
    let sequence = match cade_store::sqlite::append_run_event(database, run_id, &serialized) {
        Ok(sequence) => sequence,
        Err(error) => {
            tracing::error!(%run_id, %error, "failed to persist tool progress event");
            return;
        }
    };
    if let Some(object) = envelope.as_object_mut() {
        object.insert("run_id".to_owned(), Value::String(run_id.to_owned()));
        object.insert("seq_id".to_owned(), Value::from(sequence));
    }
    // Progress is replayable; a stalled live receiver must not prevent the
    // guarded invocation from reaching (or completing) its approval check.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tx.send(Ok(super::runtime::RunEventEnvelope {
            data: envelope.to_string(),
        })),
    )
    .await;
}

pub(super) fn parse_permission_mode(
    mode_str: &str,
) -> Option<cade_core::permissions::PermissionMode> {
    match mode_str.to_ascii_lowercase().replace('-', "_").as_str() {
        "default" => Some(cade_core::permissions::PermissionMode::Default),
        "accept_edits" | "acceptedits" => Some(cade_core::permissions::PermissionMode::AcceptEdits),
        "plan" => Some(cade_core::permissions::PermissionMode::Plan),
        "bypass_permissions" | "bypasspermissions" | "bypass" => {
            Some(cade_core::permissions::PermissionMode::BypassPermissions)
        }
        _ => None,
    }
}

pub(super) struct SseApprovalDelegate {
    pub(super) db: cade_store::sqlite::Db,
    pub(super) agent_id: String,
    pub(super) run_id: String,
    pub(super) conversation_id: Option<String>,
    pub(super) permission_sessions: Arc<crate::server::permission_sessions::PermissionSessions>,
    pub(super) permissions: cade_core::permissions::PermissionManager,
    pub(super) tx: SseTx,
}

// A cancelled turn must not leave a request in the actionable queue.
struct PendingRunApproval {
    db: cade_store::sqlite::Db,
    id: String,
    run_id: String,
    tx: SseTx,
    abandonment_reason: &'static str,
    permission_sessions: Option<Arc<crate::server::permission_sessions::PermissionSessions>>,
}

impl PendingRunApproval {
    /// One cancellation-aware wait protocol for both approvals and questions.
    async fn wait(&mut self, timeout: std::time::Duration) -> cade_agent::Result<String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(sessions) = &self.permission_sessions {
                sessions
                    .resolve_remembered(&self.db, &self.id)
                    .map_err(cade_agent::Error::custom)?;
            }
            let status =
                cade_store::sqlite::get_approval_status(&self.db, &self.id).map_err(|error| {
                    cade_agent::Error::custom(format!("Failed to read decision: {error}"))
                })?;
            match status.as_deref() {
                Some("pending") => {}
                Some(status) if status.starts_with("approved") || status.starts_with("denied") => {
                    let event = self.journal_resolution(status);
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(1),
                        self.tx.send(Ok(event)),
                    )
                    .await;
                    return Ok(status.to_owned());
                }
                _ => {
                    return Err(cade_agent::Error::custom(
                        "Request is missing or has an invalid status",
                    ));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                self.abandonment_reason = "denied:Approval request timed out";
                self.finish().await;
                return Err(cade_agent::Error::custom("Approval request timed out"));
            }
            if super::runtime::until_cancelled(
                &self.db,
                &self.run_id,
                tokio::time::sleep_until(
                    deadline
                        .min(tokio::time::Instant::now() + std::time::Duration::from_millis(150)),
                ),
            )
            .await
            .is_none()
            {
                self.finish().await;
                return Err(cade_agent::Error::custom("Request cancelled with run"));
            }
        }
    }

    fn journal_resolution(&self, status: &str) -> super::runtime::RunEventEnvelope {
        let mut event = json!({
            "message_type": "approval_resolved", "id": self.id, "status": status,
            "approved": status.starts_with("approved"),
        });
        match cade_store::sqlite::append_run_event(&self.db, &self.run_id, &event.to_string()) {
            Ok(seq) => {
                event["run_id"] = self.run_id.clone().into();
                event["seq_id"] = seq.into();
            }
            Err(error) => {
                tracing::error!(%error, approval_id = %self.id, "failed to persist approval resolution")
            }
        }
        super::runtime::RunEventEnvelope {
            data: event.to_string(),
        }
    }

    fn resolve(&self) -> Option<super::runtime::RunEventEnvelope> {
        let changed = match cade_store::sqlite::resolve_pending_approval(
            &self.db,
            &self.id,
            self.abandonment_reason,
        ) {
            Ok(changed) => changed,
            Err(error) => {
                tracing::error!(%error, approval_id = %self.id, "failed to cancel pending approval");
                return None;
            }
        };
        if changed {
            let event = self.journal_resolution(self.abandonment_reason);
            crate::server::api::agents::publish_global_event(
                Some(&self.db),
                "approval_resolved",
                json!({"id": self.id, "status": self.abandonment_reason}),
            );
            return Some(event);
        }
        None
    }

    async fn finish(&self) {
        if let Some(event) = self.resolve() {
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(1), self.tx.send(Ok(event)))
                    .await;
        }
    }
}

impl Drop for PendingRunApproval {
    fn drop(&mut self) {
        // Dropping a run task cannot await, but must still withdraw its queue
        // entry. Normal timeout/cancellation paths use finish() in order.
        if let Some(event) = self.resolve() {
            // Never schedule a late live resolution behind run_done. Replay
            // retains the durable resolution if the live receiver is saturated.
            let _ = self.tx.try_send(Ok(event));
        }
    }
}

impl SseApprovalDelegate {
    pub(super) async fn request_with_timeout(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &Value,
        reason: &str,
        timeout: std::time::Duration,
    ) -> cade_agent::Result<bool> {
        let approval_id = format!("app-{}", uuid::Uuid::new_v4());
        let _registration =
            self.permission_sessions
                .register(&approval_id, tool_name, &self.permissions);
        let event_payload = json!({
            "message_type": "approval_required",
            "id": approval_id,
            "agent_id": self.agent_id,
            "run_id": self.run_id,
            "conversation_id": self.conversation_id,
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "arguments": arguments,
            "reason": reason,
        });
        let sequence = cade_store::sqlite::create_run_approval(
            &self.db,
            &cade_store::sqlite::RunApproval {
                id: &approval_id,
                agent_id: &self.agent_id,
                run_id: &self.run_id,
                tool_name,
                arguments: &arguments.to_string(),
                reason,
                event: &event_payload.to_string(),
            },
        )
        .map_err(|error| {
            cade_agent::Error::custom(format!("Failed to persist approval request: {error}"))
        })?;
        let mut pending = PendingRunApproval {
            db: self.db.clone(),
            id: approval_id.clone(),
            run_id: self.run_id.clone(),
            tx: self.tx.clone(),
            abandonment_reason: "denied:Approval request cancelled",
            permission_sessions: Some(self.permission_sessions.clone()),
        };

        crate::server::api::agents::publish_global_event(
            Some(&self.db),
            "approval_required",
            json!({
                "id": approval_id,
                "agent_id": self.agent_id,
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "arguments": arguments,
                "reason": reason,
            }),
        );

        let mut live_payload = event_payload;
        live_payload["seq_id"] = sequence.into();
        // A closed receiver can reconnect via the durable log. A *full* live
        // channel must not silently lose the prompt and wait for ten minutes.
        let delivery = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            self.tx.send(Ok(super::runtime::RunEventEnvelope {
                data: live_payload.to_string(),
            })),
        )
        .await;
        if delivery.is_err() {
            pending.abandonment_reason = "denied:Approval prompt delivery timed out";
            pending.finish().await;
            return Err(cade_agent::Error::custom(
                "Approval prompt delivery timed out",
            ));
        }

        let status = pending.wait(timeout).await?;
        crate::server::permission_sessions::approval_outcome(&status, tool_name, &self.permissions)
            .map_err(cade_agent::Error::custom)
    }
}

#[async_trait::async_trait]
impl cade_agent::tools::ApprovalDelegate for SseApprovalDelegate {
    async fn request_approval(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &Value,
        reason: &str,
    ) -> cade_agent::Result<bool> {
        self.request_with_timeout(
            tool_call_id,
            tool_name,
            arguments,
            reason,
            std::time::Duration::from_secs(600),
        )
        .await
    }
}

async fn handle_ask_user_question(
    state: AppState,
    agent_id: String,
    run_id: String,
    tool_call_id: String,
    arguments: Value,
    tx: SseTx,
) -> ToolResult {
    use cade_agent::tools::InteractionDelegate;

    let questions = match cade_agent::tools::AskUserQuestionTool::parse_questions(&arguments) {
        Ok(q) => q,
        Err(e) => {
            return ToolResult {
                tool_call_id,
                tool_name: "ask_user_question".to_string(),
                output: format!("Invalid question parameters: {e}"),
                is_error: true,
                ui_resource_uri: None,
            };
        }
    };

    let question_id = format!("q-{}", uuid::Uuid::new_v4());
    let args_str = arguments.to_string();

    let event_payload = json!({
        "message_type": "question_required", "type": "question_required",
        "id": question_id, "agent_id": agent_id, "run_id": run_id,
        "tool_call_id": tool_call_id, "questions": arguments["questions"],
    });
    let sequence = match cade_store::sqlite::create_run_approval(
        &state.db,
        &cade_store::sqlite::RunApproval {
            id: &question_id,
            agent_id: &agent_id,
            run_id: &run_id,
            tool_name: "ask_user_question",
            arguments: &args_str,
            reason: "User input required",
            event: &event_payload.to_string(),
        },
    ) {
        Ok(sequence) => sequence,
        Err(e) => {
            tracing::warn!("Failed to create pending question in database: {e}");
            return ToolResult {
                tool_call_id,
                tool_name: "ask_user_question".to_string(),
                output: format!("Database error: {e}"),
                is_error: true,
                ui_resource_uri: None,
            };
        }
    };
    let mut pending = PendingRunApproval {
        db: state.db.clone(),
        id: question_id.clone(),
        run_id,
        tx: tx.clone(),
        abandonment_reason: "denied:Approval request cancelled",
        permission_sessions: None,
    };

    crate::server::api::agents::publish_global_event(
        Some(&state.db),
        "question_required",
        json!({
            "id": question_id,
            "agent_id": agent_id,
            "tool_call_id": tool_call_id,
            "questions": arguments.get("questions").unwrap_or(&json!([])),
        }),
    );

    let mut event_payload = event_payload;
    event_payload["seq_id"] = sequence.into();
    if tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tx.send(Ok(super::runtime::RunEventEnvelope {
            data: event_payload.to_string(),
        })),
    )
    .await
    .is_err()
    {
        pending.abandonment_reason = "denied:Approval prompt delivery timed out";
        pending.finish().await;
        return tool_error(
            tool_call_id,
            "ask_user_question",
            "Question prompt delivery timed out".into(),
        );
    }
    match pending.wait(std::time::Duration::from_secs(600)).await {
        Ok(status) => {
            if status == "approved" {
                let default_answer = cade_agent::tools::NonInteractiveDelegate;
                let answers = default_answer
                    .ask_question(&questions)
                    .await
                    .unwrap_or_default();
                let output = cade_agent::tools::AskUserQuestionTool::format_result(&answers);
                return ToolResult {
                    tool_call_id,
                    tool_name: "ask_user_question".to_string(),
                    output,
                    is_error: false,
                    ui_resource_uri: None,
                };
            } else if let Some(feedback) = status.strip_prefix("approved:") {
                let output = if let Ok(parsed) =
                    serde_json::from_str::<std::collections::HashMap<String, String>>(feedback)
                {
                    cade_agent::tools::AskUserQuestionTool::format_result(&parsed)
                } else {
                    format!(
                        "User has answered your questions: {feedback}. You can now continue with the user's answers in mind."
                    )
                };
                return ToolResult {
                    tool_call_id,
                    tool_name: "ask_user_question".to_string(),
                    output,
                    is_error: false,
                    ui_resource_uri: None,
                };
            } else if status == "denied" || status.starts_with("denied:") {
                return ToolResult {
                    tool_call_id,
                    tool_name: "ask_user_question".to_string(),
                    output: "User cancelled the question prompt without answering.".to_string(),
                    is_error: true,
                    ui_resource_uri: None,
                };
            }
            tool_error(
                tool_call_id,
                "ask_user_question",
                "Invalid question decision".into(),
            )
        }
        Err(error) => tool_error(tool_call_id, "ask_user_question", error.to_string()),
    }
}

#[cfg(test)]
pub(super) async fn execute_turn_tools(
    state: AppState,
    turn_input: super::runtime::TurnExecutionInput,
    tool_calls: Vec<LlmToolCall>,
    tx: SseTx,
) -> Vec<(ToolResult, Value)> {
    let request = super::runtime::RunRequest {
        agent_id: turn_input.agent_id.clone(),
        conversation_id: turn_input.conversation_id.clone(),
        input: turn_input.input.clone(),
        permission_mode: turn_input.permission_mode.clone(),
    };
    let options = super::runtime::RunExecutionOptions::default()
        .resolve(&state, &request)
        .expect("test run options");
    execute_turn_tools_with_options(state, turn_input, tool_calls, tx, options).await
}

pub(super) async fn execute_turn_tools_with_options(
    state: AppState,
    turn_input: super::runtime::TurnExecutionInput,
    tool_calls: Vec<LlmToolCall>,
    tx: SseTx,
    options: Arc<super::runtime::ResolvedRunExecutionOptions>,
) -> Vec<(ToolResult, Value)> {
    super::runtime::in_execution_scope(
        Some(options.clone()),
        execute_turn_tools_scoped(state, turn_input, tool_calls, tx, options),
    )
    .await
}

async fn execute_turn_tools_scoped(
    state: AppState,
    turn_input: super::runtime::TurnExecutionInput,
    tool_calls: Vec<LlmToolCall>,
    tx: SseTx,
    options: Arc<super::runtime::ResolvedRunExecutionOptions>,
) -> Vec<(ToolResult, Value)> {
    let mut turn_results: Vec<(ToolResult, Value)> = Vec::new();
    // Retain transport metadata for compatibility; execution policy comes only
    // from the accepted snapshot, never a per-turn re-resolution.
    let _ = (&turn_input.input, &turn_input.permission_mode);
    let agent_id = turn_input.agent_id;
    let run_id = turn_input.run_id;
    let conversation_id = turn_input.conversation_id;
    let runtime = options.runtime.clone();
    let hooks = options.hooks.clone();
    let permissions = options.permissions.clone();

    // Bypass verdicts never call the delegate. Strict-bash prompts still must
    // reach a real decision queue even when the selected mode is bypass.
    let approval_delegate: Arc<dyn cade_agent::tools::ApprovalDelegate> =
        Arc::new(SseApprovalDelegate {
            db: state.db.clone(),
            agent_id: agent_id.clone(),
            run_id: run_id.clone(),
            conversation_id: conversation_id.clone(),
            permission_sessions: Arc::clone(&state.permission_sessions),
            permissions: permissions.clone(),
            tx: tx.clone(),
        });

    let pipeline = Arc::new(cade_agent::tools::ToolPipeline::new(
        runtime.clone(),
        permissions,
        hooks,
        approval_delegate,
    ));

    for tc in tool_calls {
        if cade_store::sqlite::is_run_cancellation_requested(&state.db, &run_id).unwrap_or(true) {
            break;
        }
        let tool_name = tc.name;
        let tool_call_id = tc.id;
        let arguments = tc.arguments;

        // Send tool-start progress notification
        crate::server::api::agents::publish_global_event(
            Some(&state.db),
            "tool_progress",
            json!({
                "agent_id": agent_id,
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "status": "started",
            }),
        );
        emit_tool_progress(
            &state.db,
            &run_id,
            &tx,
            json!({
                "message_type": "tool_progress_message",
                "tool_progress": {
                    "id": tool_call_id,
                    "name": tool_name,
                    "status": "started",
                    "message": format!("Executing tool '{tool_name}'..."),
                }
            }),
        )
        .await;

        let result = if tool_name == "run_sequential_tasks" {
            let tool_call_id_c = tool_call_id.clone();
            let arguments_c = arguments.clone();
            let pipeline_c = Arc::clone(&pipeline);
            let handle = super::runtime::spawn_in_execution_scope(async move {
                handle_sequential_workflow(tool_call_id_c, arguments_c, pipeline_c).await
            });
            await_tool_task(&state.db, &run_id, handle, &tool_call_id, &tool_name).await
        } else if tool_name == "subagent"
            || tool_name == "run_subagent"
            || tool_name == "run_parallel_subagents"
            || tool_name == "cancel_subagent"
            || tool_name == "wait"
            || tool_name == "intercom"
            || tool_name == "subagent_supervisor"
        {
            let state_c = state.clone();
            let agent_id_c = agent_id.clone();
            let conversation_id_c = conversation_id.clone();
            let tool_name_c = tool_name.clone();
            let tool_call_id_c = tool_call_id.clone();
            let arguments_c = arguments.clone();
            let parent_mode = pipeline.permissions().mode();
            let tx_c = tx.clone();
            let run_id_c = run_id.clone();
            let handle = super::runtime::spawn_in_execution_scope(async move {
                subagent::handle_subagent_tool(
                    state_c,
                    agent_id_c,
                    conversation_id_c,
                    tool_name_c,
                    tool_call_id_c,
                    arguments_c,
                    tx_c,
                    parent_mode,
                    run_id_c,
                )
                .await
            });
            await_tool_task(&state.db, &run_id, handle, &tool_call_id, &tool_name).await
        } else if tool_name == "run_team" {
            let state_c = state.clone();
            let agent_id_c = agent_id.clone();
            let conversation_id_c = conversation_id.clone();
            let tool_call_id_c = tool_call_id.clone();
            let arguments_c = arguments.clone();
            let tx_c = tx.clone();
            let handle = super::runtime::spawn_in_execution_scope(async move {
                subagent::handle_run_team_tool(
                    state_c,
                    agent_id_c,
                    conversation_id_c,
                    tool_call_id_c,
                    arguments_c,
                    tx_c,
                )
                .await
            });
            await_tool_task(&state.db, &run_id, handle, &tool_call_id, &tool_name).await
        } else if tool_name == "ask_user_question" {
            let state_c = state.clone();
            let agent_id_c = agent_id.clone();
            let tool_call_id_c = tool_call_id.clone();
            let arguments_c = arguments.clone();
            let tx_c = tx.clone();
            let run_id_c = run_id.clone();
            let handle = super::runtime::spawn_in_execution_scope(async move {
                handle_ask_user_question(
                    state_c,
                    agent_id_c,
                    run_id_c,
                    tool_call_id_c,
                    arguments_c,
                    tx_c,
                )
                .await
            });
            await_tool_task(&state.db, &run_id, handle, &tool_call_id, &tool_name).await
        } else {
            // Unified execution via deep ToolPipeline seam
            let pipeline_c = Arc::clone(&pipeline);
            let tool_call_id_c = tool_call_id.clone();
            let tool_name_c = tool_name.clone();
            let arguments_c = arguments.clone();
            let handle = super::runtime::spawn_in_execution_scope(async move {
                match pipeline_c
                    .execute(&tool_call_id_c, &tool_name_c, &arguments_c)
                    .await
                {
                    Ok(outcome) => ToolResult {
                        tool_call_id: outcome.tool_call_id,
                        tool_name: outcome.tool_name,
                        output: outcome.output,
                        is_error: outcome.is_error,
                        ui_resource_uri: outcome.ui_resource_uri,
                    },
                    Err(e) => ToolResult {
                        tool_call_id: tool_call_id_c,
                        tool_name: tool_name_c,
                        output: format!("Tool execution error: {e}"),
                        is_error: true,
                        ui_resource_uri: None,
                    },
                }
            });
            await_tool_task(&state.db, &run_id, handle, &tool_call_id, &tool_name).await
        };

        // Send tool-complete progress notification
        crate::server::api::agents::publish_global_event(
            Some(&state.db),
            "tool_progress",
            json!({
                "agent_id": agent_id,
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "status": "completed",
            }),
        );
        emit_tool_progress(
            &state.db,
            &run_id,
            &tx,
            json!({
                "message_type": "tool_progress_message",
                "tool_progress": {
                    "id": tool_call_id,
                    "name": tool_name,
                    "status": "completed",
                    "message": "",
                }
            }),
        )
        .await;

        // Bridge plan execution seam: publish plan_update SSE event upon set_plan or UpdatePlan
        if !result.is_error {
            if tool_name == "set_plan" {
                if let Some(steps_arr) = arguments.get("steps").and_then(|v| v.as_array()) {
                    let steps_payload: Vec<Value> = steps_arr
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            json!({
                                "id": i + 1,
                                "description": s.as_str().unwrap_or(""),
                                "is_done": false
                            })
                        })
                        .collect();

                    let title = arguments
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Tasks");

                    emit_tool_progress(
                        &state.db,
                        &run_id,
                        &tx,
                        json!({
                            "message_type": "plan_update",
                            "plan": {
                                "title": title,
                                "steps": steps_payload
                            }
                        }),
                    )
                    .await;
                }
            } else if tool_name == "UpdatePlan" {
                let step_id = arguments
                    .get("step_id")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let done = arguments
                    .get("done")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                emit_tool_progress(
                    &state.db,
                    &run_id,
                    &tx,
                    json!({
                        "message_type": "plan_update",
                        "plan": {
                            "steps": [
                                {
                                    "id": step_id,
                                    "is_done": done
                                }
                            ]
                        }
                    }),
                )
                .await;
            }
        }

        turn_results.push((result, arguments));
    }

    turn_results
}

fn tool_error(id: String, name: &str, output: String) -> ToolResult {
    ToolResult {
        tool_call_id: id,
        tool_name: name.into(),
        output,
        is_error: true,
        ui_resource_uri: None,
    }
}

/// Abort only the awaited invocation. Independently spawned background children
/// retain their own ownership and cancellation tokens.
async fn await_tool_task(
    db: &cade_store::sqlite::Db,
    run_id: &str,
    mut handle: tokio::task::JoinHandle<ToolResult>,
    id: &str,
    name: &str,
) -> ToolResult {
    match super::runtime::until_cancelled(db, run_id, &mut handle).await {
        Some(Ok(result)) => result,
        Some(Err(error)) => tool_error(id.into(), name, format!("Task join error: {error}")),
        None => {
            handle.abort();
            // Wait for RAII cleanup (withdraw pending approvals/questions) before terminal outcome.
            let _ = handle.await;
            tool_error(id.into(), name, "Tool invocation cancelled with run".into())
        }
    }
}
