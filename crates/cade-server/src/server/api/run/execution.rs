//! Tool execution dispatcher for the agentic loop.

use super::{SseTx, storage_impl, subagent};
use crate::server::state::AppState;
use cade_agent::tools::{manager::ToolResult, runtime::ToolRuntime};
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
    _state: AppState,
    _agent_id: String,
    tool_call_id: String,
    arguments: Value,
    runtime: Arc<ToolRuntime>,
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

        let runtime_result = runtime
            .execute(step_tool_call_id, tool_name, &step_args)
            .await
            .unwrap_or_else(|| cade_agent::tools::runtime::RuntimeToolResult {
                tool_call_id: format!("{}-step-{}", tool_call_id, i),
                tool_name: tool_name.to_string(),
                output: format!("Error: Tool '{}' not found in runtime.", tool_name),
                is_error: true,
                ui_resource_uri: None,
            });

        let result_to_store = ToolResult {
            tool_call_id: runtime_result.tool_call_id.clone(),
            tool_name: runtime_result.tool_name.clone(),
            output: runtime_result.output.clone(),
            is_error: runtime_result.is_error,
            ui_resource_uri: runtime_result.ui_resource_uri.clone(),
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
    let _ = tx
        .send(Ok(super::runtime::RunEventEnvelope {
            data: envelope.to_string(),
        }))
        .await;
}

fn parse_permission_mode(mode_str: &str) -> Option<cade_core::permissions::PermissionMode> {
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
    pub(super) tx: SseTx,
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
        let approval_id = format!("app-{}", uuid::Uuid::new_v4());
        let args_str = arguments.to_string();

        if let Err(e) = cade_store::sqlite::create_pending_approval(
            &self.db,
            &approval_id,
            &self.agent_id,
            None,
            tool_name,
            &args_str,
        ) {
            tracing::warn!("Failed to create pending approval in database: {e}");
            return Ok(false);
        }

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

        let event_payload = json!({
            "type": "approval_required",
            "id": approval_id,
            "agent_id": self.agent_id,
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "arguments": arguments,
            "reason": reason,
        });
        let _ = self
            .tx
            .send(Ok(super::runtime::RunEventEnvelope {
                data: event_payload.to_string(),
            }))
            .await;

        let timeout_secs = 600;
        let start_time = std::time::Instant::now();
        let mut poll_interval = std::time::Duration::from_millis(150);

        loop {
            if start_time.elapsed().as_secs() > timeout_secs {
                return Err(cade_agent::Error::custom(
                    "Approval request timed out after 10 minutes.",
                ));
            }

            if let Ok(Some(status)) =
                cade_store::sqlite::get_approval_status(&self.db, &approval_id)
            {
                if status == "approved" {
                    return Ok(true);
                } else if status == "denied" {
                    return Ok(false);
                } else if let Some(feedback) = status.strip_prefix("denied:") {
                    return Err(cade_agent::Error::custom(format!(
                        "Permission Denied: {feedback}"
                    )));
                }
            }

            tokio::time::sleep(poll_interval).await;
            poll_interval = (poll_interval * 2).min(std::time::Duration::from_secs(1));
        }
    }
}

async fn handle_ask_user_question(
    state: AppState,
    agent_id: String,
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

    if let Err(e) = cade_store::sqlite::create_pending_approval(
        &state.db,
        &question_id,
        &agent_id,
        None,
        "ask_user_question",
        &args_str,
    ) {
        tracing::warn!("Failed to create pending question in database: {e}");
        return ToolResult {
            tool_call_id,
            tool_name: "ask_user_question".to_string(),
            output: format!("Database error: {e}"),
            is_error: true,
            ui_resource_uri: None,
        };
    }

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

    let event_payload = json!({
        "type": "question_required",
        "id": question_id,
        "agent_id": agent_id,
        "tool_call_id": tool_call_id,
        "questions": arguments.get("questions").unwrap_or(&json!([])),
    });
    let _ = tx
        .send(Ok(super::runtime::RunEventEnvelope {
            data: event_payload.to_string(),
        }))
        .await;

    let timeout_secs = 600;
    let start_time = std::time::Instant::now();
    let mut poll_interval = std::time::Duration::from_millis(150);

    loop {
        if start_time.elapsed().as_secs() > timeout_secs {
            return ToolResult {
                tool_call_id,
                tool_name: "ask_user_question".to_string(),
                output: "User did not answer questions (timed out after 10 minutes).".to_string(),
                is_error: true,
                ui_resource_uri: None,
            };
        }

        if let Ok(Some(status)) = cade_store::sqlite::get_approval_status(&state.db, &question_id) {
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
        }

        tokio::time::sleep(poll_interval).await;
        poll_interval = (poll_interval * 2).min(std::time::Duration::from_secs(1));
    }
}

pub(super) async fn execute_turn_tools(
    state: AppState,
    turn_input: super::runtime::TurnExecutionInput,
    tool_calls: Vec<LlmToolCall>,
    tx: SseTx,
) -> Vec<(ToolResult, Value)> {
    let mut turn_results: Vec<(ToolResult, Value)> = Vec::new();
    let agent_id = turn_input.agent_id;
    let run_id = turn_input.run_id;
    let permission_mode_override = turn_input.permission_mode;
    let _ = (&turn_input.conversation_id, &turn_input.input);

    let runtime = Arc::new(ToolRuntime::new(
        Arc::new(storage_impl::ServerStorageBackend {
            state: state.clone(),
        }),
        Arc::clone(&state.mcp),
        agent_id.clone(),
        std::env::current_dir().unwrap_or_default(),
    ));

    let cwd = std::env::current_dir().unwrap_or_default();
    let (hooks, permissions) = if let Ok(settings) = cade_core::settings::SettingsManager::new(&cwd)
    {
        let h = Arc::new(cade_core::hooks::HookEngine::new(
            settings.merged_hooks(),
            cwd.clone(),
            agent_id.clone(),
        ));
        let perm_settings = settings.permission_settings();
        let p = cade_core::permissions::PermissionManager::new_with_strict_bash(
            cade_core::permissions::PermissionMode::Default,
            perm_settings.strict_bash,
        );
        for rule_str in &perm_settings.allow {
            if let Some(rule) = cade_core::permissions::PermissionRule::parse(rule_str) {
                p.add_allow_rule(rule);
            }
        }
        for rule_str in &perm_settings.deny {
            if let Some(rule) = cade_core::permissions::PermissionRule::parse(rule_str) {
                p.add_deny_rule(rule);
            }
        }
        (h, p)
    } else {
        let h = Arc::new(cade_core::hooks::HookEngine::new(
            cade_core::settings::HooksConfig::default(),
            cwd.clone(),
            agent_id.clone(),
        ));
        let p = cade_core::permissions::PermissionManager::new(
            cade_core::permissions::PermissionMode::Default,
        );
        (h, p)
    };

    if let Some(mode_str) = permission_mode_override
        && let Some(mode) = parse_permission_mode(&mode_str)
    {
        permissions.set_mode(mode);
    }

    let approval_delegate: Arc<dyn cade_agent::tools::ApprovalDelegate> =
        if permissions.mode() == cade_core::permissions::PermissionMode::BypassPermissions {
            Arc::new(cade_agent::tools::AutoApprovalDelegate)
        } else {
            Arc::new(SseApprovalDelegate {
                db: state.db.clone(),
                agent_id: agent_id.clone(),
                tx: tx.clone(),
            })
        };

    let pipeline = Arc::new(cade_agent::tools::ToolPipeline::new(
        runtime.clone(),
        permissions,
        hooks,
        approval_delegate,
    ));

    for tc in tool_calls {
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
            let state_c = state.clone();
            let agent_id_c = agent_id.clone();
            let tool_call_id_c = tool_call_id.clone();
            let arguments_c = arguments.clone();
            let runtime_c = Arc::clone(&runtime);
            let handle = tokio::spawn(async move {
                handle_sequential_workflow(
                    state_c,
                    agent_id_c,
                    tool_call_id_c,
                    arguments_c,
                    runtime_c,
                )
                .await
            });
            handle.await.unwrap_or_else(|e| ToolResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: "run_sequential_tasks".to_string(),
                output: format!("Task join error: {e}"),
                is_error: true,
                ui_resource_uri: None,
            })
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
            let tool_name_c = tool_name.clone();
            let tool_call_id_c = tool_call_id.clone();
            let arguments_c = arguments.clone();
            let tx_c = tx.clone();
            let handle = tokio::spawn(async move {
                subagent::handle_subagent_tool(
                    state_c,
                    agent_id_c,
                    tool_name_c,
                    tool_call_id_c,
                    arguments_c,
                    tx_c,
                )
                .await
            });
            handle.await.unwrap_or_else(|e| ToolResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: tool_name.clone(),
                output: format!("Task join error: {e}"),
                is_error: true,
                ui_resource_uri: None,
            })
        } else if tool_name == "run_team" {
            let state_c = state.clone();
            let agent_id_c = agent_id.clone();
            let tool_call_id_c = tool_call_id.clone();
            let arguments_c = arguments.clone();
            let tx_c = tx.clone();
            let handle = tokio::spawn(async move {
                subagent::handle_run_team_tool(
                    state_c,
                    agent_id_c,
                    tool_call_id_c,
                    arguments_c,
                    tx_c,
                )
                .await
            });
            handle.await.unwrap_or_else(|e| ToolResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: "run_team".to_string(),
                output: format!("Task join error: {e}"),
                is_error: true,
                ui_resource_uri: None,
            })
        } else if tool_name == "ask_user_question" {
            let state_c = state.clone();
            let agent_id_c = agent_id.clone();
            let tool_call_id_c = tool_call_id.clone();
            let arguments_c = arguments.clone();
            let tx_c = tx.clone();
            let handle = tokio::spawn(async move {
                handle_ask_user_question(state_c, agent_id_c, tool_call_id_c, arguments_c, tx_c)
                    .await
            });
            handle.await.unwrap_or_else(|e| ToolResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: "ask_user_question".to_string(),
                output: format!("Task join error: {e}"),
                is_error: true,
                ui_resource_uri: None,
            })
        } else {
            // Unified execution via deep ToolPipeline seam
            let pipeline_c = Arc::clone(&pipeline);
            let tool_call_id_c = tool_call_id.clone();
            let tool_name_c = tool_name.clone();
            let arguments_c = arguments.clone();
            let handle = tokio::spawn(async move {
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
            handle.await.unwrap_or_else(|e| ToolResult {
                tool_call_id: tool_call_id.clone(),
                tool_name: tool_name.clone(),
                output: format!("Task join error: {e}"),
                is_error: true,
                ui_resource_uri: None,
            })
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
