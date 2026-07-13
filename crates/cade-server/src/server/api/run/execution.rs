//! Tool execution dispatcher for the agentic loop.

use super::{SseTx, storage_impl, subagent};
use crate::server::state::AppState;
use cade_agent::tools::{bash_agent::BashToolAgent, search_agent::SearchToolAgent};
use cade_agent::{
    moa::{Agent, AgentRequest},
    routing::Router,
    tools::{
        fs_agent::{ApplyPatchToolAgent, EditToolAgent, ReadToolAgent, WriteToolAgent},
        manager::ToolResult,
        runtime::ToolRuntime,
    },
};
use cade_ai::LlmToolCall;
use futures::future;
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

/// Recursively substitutes placeholders in serde_json::Value.
fn substitute_step_arguments(args: &mut Value, step_results: &[ToolResult]) {
    match args {
        Value::String(s) => {
            let re = regex::Regex::new(r#"\$steps\.(\d+)\.output"#).unwrap();
            if let Some(caps) = re.captures(s)
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
    _state: &AppState,
    _agent_id: &str,
    tool_call_id: &str,
    arguments: &Value,
    runtime: &ToolRuntime,
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

pub(super) async fn execute_turn_tools(
    state: &AppState,
    agent_id: &str,
    _conv_id: Option<&str>,
    input: &str,
    tool_calls: Vec<LlmToolCall>,
    tx: SseTx,
) -> Vec<(ToolResult, Value)> {
    let mut router = Router::new();
    let agents_to_register: Vec<Arc<dyn Agent>> = vec![
        Arc::new(ReadToolAgent),
        Arc::new(WriteToolAgent),
        Arc::new(EditToolAgent),
        Arc::new(ApplyPatchToolAgent),
        Arc::new(BashToolAgent),
        Arc::new(SearchToolAgent),
    ];
    let mut moa_tool_registry = HashSet::new();
    for agent in agents_to_register {
        for tool_name in agent.supported_tools() {
            moa_tool_registry.insert(tool_name.to_string());
        }
        router.add_agent(agent);
    }

    let router_req = AgentRequest {
        prompt: input.to_string(),
    };

    let mut turn_results: Vec<(ToolResult, Value)> = Vec::new();
    let mut moa_tool_calls = Vec::new();
    let mut legacy_tool_calls = Vec::new();

    for tc in tool_calls {
        if moa_tool_registry.contains(&tc.name) {
            moa_tool_calls.push(tc);
        } else {
            legacy_tool_calls.push(tc);
        }
    }

    let runtime = ToolRuntime::new(
        Arc::new(storage_impl::ServerStorageBackend {
            state: state.clone(),
        }),
        Arc::clone(&state.mcp),
        agent_id.to_string(),
        std::env::current_dir().unwrap_or_default(),
    );

    if !moa_tool_calls.is_empty() {
        let selected_agents = router.route(&router_req).unwrap_or_default();
        let mut execution_futures = Vec::new();

        for agent in selected_agents {
            for tc in &moa_tool_calls {
                if agent.supported_tools().contains(&tc.name.as_str()) {
                    let exec_req = AgentRequest {
                        prompt: format!("{} {}", tc.name, tc.arguments),
                    };
                    let tool_call_id = tc.id.clone();
                    let tool_name = tc.name.clone();
                    let arguments = tc.arguments.clone();
                    let agent_clone = agent.clone();
                    execution_futures.push(async move {
                        (
                            tool_call_id,
                            tool_name,
                            arguments,
                            agent_clone.execute(&exec_req).await,
                        )
                    });
                }
            }
        }

        let moa_results = future::join_all(execution_futures).await;

        for (tool_call_id, tool_name, arguments, result) in moa_results {
            let tool_result = match result {
                Ok(response) => ToolResult {
                    tool_call_id,
                    tool_name,
                    output: response.content,
                    is_error: false,
                    ui_resource_uri: None,
                },
                Err(e) => ToolResult {
                    tool_call_id,
                    tool_name,
                    output: e.to_string(),
                    is_error: true,
                    ui_resource_uri: None,
                },
            };
            turn_results.push((tool_result, arguments));
        }
    }

    for tc in legacy_tool_calls {
        let arguments = tc.arguments.clone();

        // Send tool-start progress notification
        let _ = tx.send(Ok(axum::response::sse::Event::default().data(json!({
            "message_type": "tool_progress_message",
            "tool_progress": {
                "id": tc.id,
                "name": tc.name,
                "status": "started",
                "message": format!("Executing tool '{}'...", tc.name)
            }
        }).to_string()))).await;

        let result = if tc.name == "run_sequential_tasks" {
            handle_sequential_workflow(state, agent_id, &tc.id, &arguments, &runtime).await
        } else if tc.name == "run_subagent" {
            subagent::handle_run_subagent_tool(state, agent_id, &tc.id, &arguments, tx.clone())
                .await
        } else if tc.name == "run_parallel_subagents" {
            subagent::handle_run_parallel_subagents_tool(
                state,
                agent_id,
                &tc.id,
                &arguments,
                tx.clone(),
            )
            .await
        } else if tc.name == "cancel_subagent" {
            subagent::handle_cancel_subagent_tool(state, &tc.id, &arguments).await
        } else {
            // ... other legacy tool handlers
            runtime
                .execute(tc.id.clone(), &tc.name, &arguments)
                .await
                .map(|r| ToolResult {
                    tool_call_id: r.tool_call_id,
                    tool_name: r.tool_name,
                    output: r.output,
                    is_error: r.is_error,
                    ui_resource_uri: r.ui_resource_uri,
                })
                .unwrap_or_else(|| ToolResult {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    output: "Tool execution failed".to_string(),
                    is_error: true,
                    ui_resource_uri: None,
                })
        };

        // Send tool-complete progress notification
        let _ = tx.send(Ok(axum::response::sse::Event::default().data(json!({
            "message_type": "tool_progress_message",
            "tool_progress": {
                "id": tc.id,
                "name": tc.name,
                "status": "completed",
                "message": ""
            }
        }).to_string()))).await;

        turn_results.push((result, arguments));
    }

    turn_results
}
