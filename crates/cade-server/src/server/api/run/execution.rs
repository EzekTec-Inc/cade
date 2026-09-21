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

pub(super) async fn execute_turn_tools(
    state: AppState,
    agent_id: String,
    _conv_id: Option<String>,
    run_id: String,
    _input: String,
    tool_calls: Vec<LlmToolCall>,
    tx: SseTx,
) -> Vec<(ToolResult, Value)> {
    let mut turn_results: Vec<(ToolResult, Value)> = Vec::new();

    let runtime = Arc::new(ToolRuntime::new(
        Arc::new(storage_impl::ServerStorageBackend {
            state: state.clone(),
        }),
        Arc::clone(&state.mcp),
        agent_id.clone(),
        std::env::current_dir().unwrap_or_default(),
    ));

    let cwd = std::env::current_dir().unwrap_or_default();
    let hooks = if let Ok(settings) = cade_core::settings::SettingsManager::new(&cwd) {
        Arc::new(cade_core::hooks::HookEngine::new(
            settings.merged_hooks(),
            cwd.clone(),
            agent_id.clone(),
        ))
    } else {
        Arc::new(cade_core::hooks::HookEngine::new(
            cade_core::settings::HooksConfig::default(),
            cwd.clone(),
            agent_id.clone(),
        ))
    };
    let permissions = cade_core::permissions::PermissionManager::new(
        cade_core::permissions::PermissionMode::Default,
    );
    let pipeline = Arc::new(cade_agent::tools::ToolPipeline::new(
        runtime.clone(),
        permissions,
        hooks,
        Arc::new(cade_agent::tools::AutoApprovalDelegate),
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
