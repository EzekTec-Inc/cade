use async_trait::async_trait;
use serde_json::Value;
use crate::Result;
use crate::tools::ToolResult;

/// A trait that defines the interface for running a single subagent.
/// Both the CLI client and the server implement this to execute single runs
/// using their respective isolated runtime environments.
#[async_trait]
pub trait SubagentSingleRunner: Send + Sync {
    /// Executes a single subagent run.
    async fn run_single(
        &self,
        call_id: &str,
        args: &Value,
        force_sync: bool,
    ) -> Result<ToolResult>;

    /// Lists all available subagents.
    fn list_subagents(&self) -> Result<String>;

    /// Cancels or interrupts an active subagent task.
    async fn cancel_subagent(&self, subagent_id: &str) -> Result<String>;

    /// Inspects the subagent system status.
    fn doctor_status(&self) -> Result<String>;
}

/// The unified subagent coordinator module.
/// It acts as a deep module that encapsulates all multi-agent orchestration logic,
/// such as action routing, chain loops (sequential dependencies), and concurrent
/// parallel joins, completely removing duplicate copies from client and server layers.
pub struct SubagentCoordinator;

impl SubagentCoordinator {
    /// Coordinates the orchestration of a subagent tool call based on its arguments.
    /// Delegates single runs back to the supplied `SubagentSingleRunner` adapter.
    pub async fn coordinate<R: SubagentSingleRunner>(
        runner: &R,
        call_id: &str,
        args: &Value,
    ) -> Result<ToolResult> {
        let cfg = crate::subagents::config::SubagentConfig::from_args(args);

        if let Err(reason) = cfg.validate() {
            return Ok(ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "subagent".to_string(),
                output: reason,
                is_error: true,
                ui_resource_uri: None,
            });
        }

        // 1. Action mode
        if let Some(action) = &cfg.action {
            match action.as_str() {
                "list" => {
                    let out = runner.list_subagents()?;
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: out,
                        is_error: false,
                        ui_resource_uri: None,
                    });
                }
                "cancel" | "cancel_subagent" | "interrupt" => {
                    let subagent_id = cfg
                        .id
                        .clone()
                        .or_else(|| cfg.agent_id.clone())
                        .unwrap_or_default();
                    if subagent_id.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'id' is required for cancel/interrupt".to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                    let out = runner.cancel_subagent(&subagent_id).await?;
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: out,
                        is_error: false,
                        ui_resource_uri: None,
                    });
                }
                "doctor" => {
                    let out = runner.doctor_status()?;
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: out,
                        is_error: false,
                        ui_resource_uri: None,
                    });
                }
                other => {
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: format!("error: unsupported action: {other}"),
                        is_error: true,
                        ui_resource_uri: None,
                    });
                }
            }
        }

        // 2. Chain mode (Sequential execution)
        if let Some(chain_val) = &cfg.chain {
            if chain_val.is_empty() {
                return Ok(ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "subagent".to_string(),
                    output: "error: 'chain' array cannot be empty".to_string(),
                    is_error: true,
                    ui_resource_uri: None,
                });
            }

            let mut previous_output = String::new();
            for (idx, step_args) in chain_val.iter().enumerate() {
                let step_call_id = format!("{}_{}", call_id, idx);
                let mut step_args_c = step_args.clone();
                let task_val = if step_args_c.get("task").is_some() {
                    step_args_c.get_mut("task")
                } else {
                    step_args_c.get_mut("prompt")
                };
                if let Some(task_val) = task_val
                    && let Some(task_str) = task_val.as_str()
                {
                    let replaced = task_str.replace("{previous}", &previous_output);
                    *task_val = serde_json::Value::String(replaced);
                }

                let step_res = runner.run_single(&step_call_id, &step_args_c, true).await?;
                if step_res.is_error {
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: format!(
                            "Chain stopped at step {} because of error: {}",
                            idx + 1,
                            step_res.output
                        ),
                        is_error: true,
                        ui_resource_uri: None,
                    });
                }
                previous_output = step_res.output;
            }

            return Ok(ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "subagent".to_string(),
                output: previous_output,
                is_error: false,
                ui_resource_uri: None,
            });
        }

        // 3. Parallel mode (Concurrent execution)
        if let Some(tasks_val) = &cfg.tasks {
            if tasks_val.is_empty() {
                return Ok(ToolResult {
                    tool_call_id: call_id.to_string(),
                    tool_name: "subagent".to_string(),
                    output: "error: 'tasks' array cannot be empty".to_string(),
                    is_error: true,
                    ui_resource_uri: None,
                });
            }

            let mut futures = Vec::new();
            for (idx, task_args) in tasks_val.iter().enumerate() {
                let task_call_id = format!("{}_{}", call_id, idx);
                let task_args_c = task_args.clone();
                futures.push(async move {
                    runner.run_single(&task_call_id, &task_args_c, true).await
                });
            }

            let results = futures::future::join_all(futures).await;

            let mut aggregated = Vec::new();
            for (idx, res) in results.into_iter().enumerate() {
                match res {
                    Ok(tr) => {
                        aggregated.push(serde_json::json!({
                            "task_index": idx,
                            "output": tr.output,
                            "is_error": tr.is_error,
                        }));
                    }
                    Err(e) => {
                        aggregated.push(serde_json::json!({
                            "task_index": idx,
                            "output": format!("task failed: {e}"),
                            "is_error": true,
                        }));
                    }
                }
            }

            return Ok(ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "subagent".to_string(),
                output: serde_json::to_string_pretty(&aggregated)
                    .unwrap_or_else(|e| format!("error serializing results: {e}")),
                is_error: false,
                ui_resource_uri: None,
            });
        }

        // 4. Default: single mode
        runner.run_single(call_id, args, false).await
    }
}
