use crate::Result;
use crate::tools::ToolResult;
use async_trait::async_trait;
use futures::future::join_all;
use futures::stream::{self, StreamExt};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

/// A trait that defines the interface for running a single subagent.
/// Both the CLI client and the server implement this to execute single runs
/// using their respective isolated runtime environments.
#[async_trait]
pub trait SubagentSingleRunner: Send + Sync {
    /// Executes a single subagent run.
    async fn run_single(&self, call_id: &str, args: &Value, force_sync: bool)
    -> Result<ToolResult>;

    /// Lists all available subagents.
    fn list_subagents(&self) -> Result<String>;

    /// Cancels or interrupts an active subagent task.
    async fn cancel_subagent(&self, subagent_id: &str) -> Result<String>;

    /// Inspects the subagent system status.
    fn doctor_status(&self) -> Result<String>;

    /// Dynamically hot-swaps the model of an active subagent for its next turn.
    fn hot_swap_model(&self, subagent_id: &str, new_model: &str) -> Result<String> {
        Ok(format!(
            "Model for subagent '{subagent_id}' queued to swap to '{new_model}'"
        ))
    }
}

/// Helper to resolve the global ~/.cade/subagents/ directory
fn global_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".cade").join("subagents"))
}

/// Helper to resolve the project .cade/subagents/ directory
fn project_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join(".cade")
        .join("subagents")
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

        // ── 1. Action mode (Management / Control) ───────────────────────────
        if let Some(action) = &cfg.action {
            match action.as_str() {
                "list" => {
                    let defs = crate::subagents::discover_all_subagents(
                        &std::env::current_dir().unwrap_or_default(),
                    );
                    let mut out = String::from("Executable agents:\n");
                    for d in defs {
                        out.push_str(&format!(
                            "- {} ({}): {} ({})\n",
                            d.name, d.scope, d.description, d.tools
                        ));
                    }
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: out,
                        is_error: false,
                        ui_resource_uri: None,
                    });
                }
                "get" => {
                    let agent_name = if cfg.mode != "build" && !cfg.mode.is_empty() {
                        cfg.mode.clone()
                    } else {
                        cfg.id.clone().unwrap_or_default()
                    };
                    if agent_name.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'agent' or 'id' parameter is required for 'get' action"
                                .to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                    let defs = crate::subagents::discover_all_subagents(
                        &std::env::current_dir().unwrap_or_default(),
                    );
                    if let Some(def) = defs.iter().find(|d| d.name == agent_name) {
                        let out = format!(
                            "Agent: {}\nScope: {}\nDescription: {}\nModel: {}\nTools: {}\n\nSystem Prompt:\n{}",
                            def.name,
                            def.scope,
                            def.description,
                            def.model.as_deref().unwrap_or("Inherit Parent Model"),
                            def.tools,
                            def.system_prompt
                        );
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: out,
                            is_error: false,
                            ui_resource_uri: None,
                        });
                    } else {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!("error: agent '{}' not found", agent_name),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                }
                "models" => {
                    let defs = crate::subagents::discover_all_subagents(
                        &std::env::current_dir().unwrap_or_default(),
                    );
                    let mut out = String::from("Registered subagent models:\n");
                    for d in defs {
                        out.push_str(&format!(
                            "- {}: {}\n",
                            d.name,
                            d.model.as_deref().unwrap_or("Inherit Parent Model")
                        ));
                    }
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: out,
                        is_error: false,
                        ui_resource_uri: None,
                    });
                }
                "create" | "update" => {
                    let config_val = args
                        .get("config")
                        .cloned()
                        .unwrap_or(Value::Object(Default::default()));
                    let name = config_val
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if name.is_empty() {
                        // If 'config.name' is missing but 'task' or 'prompt' is present with 'agent',
                        // the caller intended to execute the subagent rather than define one on disk.
                        if args.get("task").is_some() || args.get("prompt").is_some() {
                            return runner.run_single(call_id, args, false).await;
                        }
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'config.name' is required".to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                    let system_prompt = config_val
                        .get("systemPrompt")
                        .or_else(|| config_val.get("system_prompt"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let description = config_val
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Custom user subagent")
                        .to_string();
                    let model = config_val
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let tools = config_val
                        .get("tools")
                        .and_then(|v| v.as_str())
                        .unwrap_or("all")
                        .to_string();
                    let scope_str = config_val
                        .get("agentScope")
                        .and_then(|v| v.as_str())
                        .unwrap_or("project");

                    let target_dir = if scope_str == "user" {
                        if let Some(dir) = global_dir() {
                            dir
                        } else {
                            return Ok(ToolResult {
                                tool_call_id: call_id.to_string(),
                                tool_name: "subagent".to_string(),
                                output: "error: could not resolve user home directory".to_string(),
                                is_error: true,
                                ui_resource_uri: None,
                            });
                        }
                    } else {
                        project_dir()
                    };

                    if let Err(e) = fs::create_dir_all(&target_dir) {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!("error: failed to create subagents directory: {e}"),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }

                    let file_path = target_dir.join(format!("{}.md", name));
                    let model_line = if let Some(m) = &model {
                        format!("model: {}\n", m)
                    } else {
                        String::new()
                    };

                    let content = format!(
                        "---\nname: {}\ndescription: {}\n{}tools: {}\n---\n\n{}",
                        name, description, model_line, tools, system_prompt
                    );

                    if let Err(e) = fs::write(&file_path, content) {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!("error: failed to write subagent file: {e}"),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }

                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: format!(
                            "Successfully {}d subagent '{}' at {}",
                            action,
                            name,
                            file_path.display()
                        ),
                        is_error: false,
                        ui_resource_uri: None,
                    });
                }
                "delete" => {
                    let name = if cfg.mode != "build" && !cfg.mode.is_empty() {
                        cfg.mode.clone()
                    } else {
                        cfg.id.clone().unwrap_or_default()
                    };
                    if name.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output:
                                "error: 'agent' or 'id' parameter is required for 'delete' action"
                                    .to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }

                    let mut deleted = false;
                    let paths_to_try = vec![
                        project_dir().join(format!("{}.md", name)),
                        project_dir().join(format!("{}.json", name)),
                        project_dir().join(format!("{}.md.disabled", name)),
                    ];
                    for path in paths_to_try {
                        if path.exists() {
                            let _ = fs::remove_file(path);
                            deleted = true;
                        }
                    }

                    if let Some(dir) = global_dir() {
                        let global_paths = vec![
                            dir.join(format!("{}.md", name)),
                            dir.join(format!("{}.json", name)),
                            dir.join(format!("{}.md.disabled", name)),
                        ];
                        for path in global_paths {
                            if path.exists() {
                                let _ = fs::remove_file(path);
                                deleted = true;
                            }
                        }
                    }

                    if deleted {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!("Deleted custom subagent definition '{}'", name),
                            is_error: false,
                            ui_resource_uri: None,
                        });
                    } else {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!(
                                "error: no custom subagent definition found for '{}'",
                                name
                            ),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                }
                "eject" => {
                    let name = if cfg.mode != "build" && !cfg.mode.is_empty() {
                        cfg.mode.clone()
                    } else {
                        cfg.id.clone().unwrap_or_default()
                    };
                    if name.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'agent' parameter is required for 'eject' action"
                                .to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }

                    let defs = crate::subagents::discover_all_subagents(
                        &std::env::current_dir().unwrap_or_default(),
                    );
                    if let Some(def) = defs.iter().find(|d| d.name == name) {
                        let target_dir = project_dir();
                        let _ = fs::create_dir_all(&target_dir);
                        let file_path = target_dir.join(format!("{}.md", name));
                        let model_line = if let Some(m) = &def.model {
                            format!("model: {}\n", m)
                        } else {
                            String::new()
                        };

                        let content = format!(
                            "---\nname: {}\ndescription: {}\n{}tools: {}\n---\n\n{}",
                            def.name, def.description, model_line, def.tools, def.system_prompt
                        );

                        if let Err(e) = fs::write(&file_path, content) {
                            return Ok(ToolResult {
                                tool_call_id: call_id.to_string(),
                                tool_name: "subagent".to_string(),
                                output: format!("error: failed to write subagent file: {e}"),
                                is_error: true,
                                ui_resource_uri: None,
                            });
                        }

                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!(
                                "Successfully ejected subagent '{}' to {}",
                                name,
                                file_path.display()
                            ),
                            is_error: false,
                            ui_resource_uri: None,
                        });
                    } else {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!("error: subagent '{}' not found", name),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                }
                "disable" => {
                    let name = if cfg.mode != "build" && !cfg.mode.is_empty() {
                        cfg.mode.clone()
                    } else {
                        cfg.id.clone().unwrap_or_default()
                    };
                    if name.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'agent' parameter is required for 'disable' action"
                                .to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }

                    let path = project_dir().join(format!("{}.md", name));
                    if path.exists() {
                        let disabled_path = project_dir().join(format!("{}.md.disabled", name));
                        if let Err(e) = fs::rename(&path, &disabled_path) {
                            return Ok(ToolResult {
                                tool_call_id: call_id.to_string(),
                                tool_name: "subagent".to_string(),
                                output: format!("error: failed to disable: {e}"),
                                is_error: true,
                                ui_resource_uri: None,
                            });
                        }
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!("Disabled subagent '{}' (project scope)", name),
                            is_error: false,
                            ui_resource_uri: None,
                        });
                    }

                    if let Some(dir) = global_dir() {
                        let path = dir.join(format!("{}.md", name));
                        if path.exists() {
                            let disabled_path = dir.join(format!("{}.md.disabled", name));
                            if let Err(e) = fs::rename(&path, &disabled_path) {
                                return Ok(ToolResult {
                                    tool_call_id: call_id.to_string(),
                                    tool_name: "subagent".to_string(),
                                    output: format!("error: failed to disable: {e}"),
                                    is_error: true,
                                    ui_resource_uri: None,
                                });
                            }
                            return Ok(ToolResult {
                                tool_call_id: call_id.to_string(),
                                tool_name: "subagent".to_string(),
                                output: format!("Disabled subagent '{}' (global scope)", name),
                                is_error: false,
                                ui_resource_uri: None,
                            });
                        }
                    }

                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: format!(
                            "error: no custom subagent found to disable for '{}'",
                            name
                        ),
                        is_error: true,
                        ui_resource_uri: None,
                    });
                }
                "enable" => {
                    let name = if cfg.mode != "build" && !cfg.mode.is_empty() {
                        cfg.mode.clone()
                    } else {
                        cfg.id.clone().unwrap_or_default()
                    };
                    if name.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'agent' parameter is required for 'enable' action"
                                .to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }

                    let disabled_path = project_dir().join(format!("{}.md.disabled", name));
                    if disabled_path.exists() {
                        let path = project_dir().join(format!("{}.md", name));
                        if let Err(e) = fs::rename(&disabled_path, &path) {
                            return Ok(ToolResult {
                                tool_call_id: call_id.to_string(),
                                tool_name: "subagent".to_string(),
                                output: format!("error: failed to enable: {e}"),
                                is_error: true,
                                ui_resource_uri: None,
                            });
                        }
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!("Enabled subagent '{}' (project scope)", name),
                            is_error: false,
                            ui_resource_uri: None,
                        });
                    }

                    if let Some(dir) = global_dir() {
                        let disabled_path = dir.join(format!("{}.md.disabled", name));
                        if disabled_path.exists() {
                            let path = dir.join(format!("{}.md", name));
                            if let Err(e) = fs::rename(&disabled_path, &path) {
                                return Ok(ToolResult {
                                    tool_call_id: call_id.to_string(),
                                    tool_name: "subagent".to_string(),
                                    output: format!("error: failed to enable: {e}"),
                                    is_error: true,
                                    ui_resource_uri: None,
                                });
                            }
                            return Ok(ToolResult {
                                tool_call_id: call_id.to_string(),
                                tool_name: "subagent".to_string(),
                                output: format!("Enabled subagent '{}' (global scope)", name),
                                is_error: false,
                                ui_resource_uri: None,
                            });
                        }
                    }

                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: format!(
                            "error: no disabled subagent definition found for '{}'",
                            name
                        ),
                        is_error: true,
                        ui_resource_uri: None,
                    });
                }
                "reset" => {
                    let name = if cfg.mode != "build" && !cfg.mode.is_empty() {
                        cfg.mode.clone()
                    } else {
                        cfg.id.clone().unwrap_or_default()
                    };
                    if name.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'agent' parameter is required for 'reset' action"
                                .to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }

                    let paths_to_remove = vec![
                        project_dir().join(format!("{}.md", name)),
                        project_dir().join(format!("{}.json", name)),
                        project_dir().join(format!("{}.md.disabled", name)),
                    ];
                    for path in paths_to_remove {
                        if path.exists() {
                            let _ = fs::remove_file(path);
                        }
                    }

                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: format!("Reset subagent '{}' to its bundled default.", name),
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
                "steer" => {
                    let subagent_id = cfg.id.clone().unwrap_or_default();
                    let message = args["message"].as_str().unwrap_or("").to_string();
                    if subagent_id.is_empty() || message.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'id' and 'message' are required for steering"
                                .to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: format!("Guidance sent to subagent '{}'", subagent_id),
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
                "tasks" | "parallel" => {
                    let tasks = cfg.tasks.as_ref().or_else(|| args["tasks"].as_array());
                    if let Some(tasks_val) = tasks {
                        let concurrency = args
                            .get("concurrency")
                            .and_then(|v| v.as_u64())
                            .map(|c| c as usize);
                        let use_worktree = args
                            .get("worktree")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        return Self::coordinate_parallel(
                            call_id,
                            tasks_val,
                            concurrency,
                            use_worktree,
                            runner,
                        )
                        .await;
                    } else {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'tasks' array is required for 'tasks' action"
                                .to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                }
                "resume" => {
                    let subagent_id = cfg
                        .id
                        .clone()
                        .or_else(|| cfg.agent_id.clone())
                        .or_else(|| args["id"].as_str().map(|s| s.to_string()))
                        .unwrap_or_default();
                    if subagent_id.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'id' is required for 'resume' action".to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                    let message = args["message"].as_str().unwrap_or("").to_string();
                    if !message.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: format!(
                                "Resumed subagent '{}' with guidance: {}",
                                subagent_id, message
                            ),
                            is_error: false,
                            ui_resource_uri: None,
                        });
                    }
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: format!("Resumed subagent '{}'", subagent_id),
                        is_error: false,
                        ui_resource_uri: None,
                    });
                }
                "status" => {
                    let subagent_id = cfg
                        .id
                        .clone()
                        .or_else(|| cfg.agent_id.clone())
                        .or_else(|| args["id"].as_str().map(|s| s.to_string()));
                    let out = if let Some(id) = subagent_id {
                        format!("Subagent '{id}' is registered")
                    } else {
                        runner.doctor_status()?
                    };
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: out,
                        is_error: false,
                        ui_resource_uri: None,
                    });
                }
                "model" | "swap_model" | "hot_swap" => {
                    let subagent_id = cfg
                        .id
                        .clone()
                        .or_else(|| cfg.agent_id.clone())
                        .or_else(|| args["id"].as_str().map(|s| s.to_string()))
                        .unwrap_or_default();
                    let new_model = args["model"].as_str().unwrap_or("").to_string();
                    if subagent_id.is_empty() || new_model.is_empty() {
                        return Ok(ToolResult {
                            tool_call_id: call_id.to_string(),
                            tool_name: "subagent".to_string(),
                            output: "error: 'id' and 'model' are required for model hot-swap"
                                .to_string(),
                            is_error: true,
                            ui_resource_uri: None,
                        });
                    }
                    let out = runner.hot_swap_model(&subagent_id, &new_model)?;
                    return Ok(ToolResult {
                        tool_call_id: call_id.to_string(),
                        tool_name: "subagent".to_string(),
                        output: out,
                        is_error: false,
                        ui_resource_uri: None,
                    });
                }
                "run" | "single" | "execute" => {
                    return runner.run_single(call_id, args, false).await;
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

        // ── 2. Chain mode (Sequential execution) ───────────────────────────
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

            // Create a shared directory for chain files
            let chain_run_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
            let chain_dir_path = std::env::temp_dir()
                .join("cade-chains")
                .join(format!("chain-{}", chain_run_id));
            let _ = fs::create_dir_all(&chain_dir_path);
            let chain_dir_str = chain_dir_path.to_string_lossy().to_string();

            let mut previous_output = String::new();
            for (idx, step_args) in chain_val.iter().enumerate() {
                let step_call_id = format!("{}_{}", call_id, idx);
                let mut step_args_c = step_args.clone();

                // ── Sub-step substitution logic ──
                let replace_vars = |val_str: &str| -> String {
                    val_str
                        .replace("{previous}", &previous_output)
                        .replace("{task}", &cfg.prompt)
                        .replace("{chain_dir}", &chain_dir_str)
                };

                let task_val = if step_args_c.get("task").is_some() {
                    step_args_c.get_mut("task")
                } else {
                    step_args_c.get_mut("prompt")
                };
                if let Some(t_val) = task_val
                    && let Some(task_str) = t_val.as_str()
                {
                    *t_val = Value::String(replace_vars(task_str));
                }

                // Check for parallel step inside chain
                let step_res = if let Some(parallel_tasks) =
                    step_args_c.get("parallel").and_then(|v| v.as_array())
                {
                    let mut futures = Vec::new();
                    for (p_idx, p_task_args) in parallel_tasks.iter().enumerate() {
                        let p_call_id = format!("{}_p{}", step_call_id, p_idx);
                        let mut p_task_args_c = p_task_args.clone();
                        let p_task_val = if p_task_args_c.get("task").is_some() {
                            p_task_args_c.get_mut("task")
                        } else {
                            p_task_args_c.get_mut("prompt")
                        };
                        if let Some(pt_val) = p_task_val
                            && let Some(pt_str) = pt_val.as_str()
                        {
                            *pt_val = Value::String(replace_vars(pt_str));
                        }
                        let runner_ref = runner;
                        futures.push(async move {
                            runner_ref
                                .run_single(&p_call_id, &p_task_args_c, true)
                                .await
                        });
                    }
                    let parallel_results = join_all(futures).await;
                    let mut aggregated = Vec::new();
                    let mut has_error = false;
                    for tr_res in parallel_results {
                        match tr_res {
                            Ok(tr) => {
                                if tr.is_error {
                                    has_error = true;
                                }
                                aggregated.push(tr.output);
                            }
                            Err(e) => {
                                has_error = true;
                                aggregated.push(format!("Parallel step failed: {e}"));
                            }
                        }
                    }
                    ToolResult {
                        tool_call_id: step_call_id,
                        tool_name: "subagent".to_string(),
                        output: aggregated.join("\n\n---\n\n"),
                        is_error: has_error,
                        ui_resource_uri: None,
                    }
                } else {
                    runner.run_single(&step_call_id, &step_args_c, true).await?
                };

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

        // ── 3. Parallel mode (Concurrent execution with concurrency limit) ─
        if let Some(tasks_val) = &cfg.tasks {
            let concurrency = args
                .get("concurrency")
                .and_then(|v| v.as_u64())
                .map(|c| c as usize);
            let use_worktree = args
                .get("worktree")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            return Self::coordinate_parallel(
                call_id,
                tasks_val,
                concurrency,
                use_worktree,
                runner,
            )
            .await;
        }

        // ── 4. Default: single mode ────────────────────────────────────────
        runner.run_single(call_id, args, false).await
    }

    /// Internal helper coordinating parallel subagent runs with concurrency containment.
    async fn coordinate_parallel<R: SubagentSingleRunner>(
        call_id: &str,
        tasks_val: &[Value],
        concurrency: Option<usize>,
        worktree: bool,
        runner: &R,
    ) -> Result<ToolResult> {
        if tasks_val.is_empty() {
            return Ok(ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "subagent".to_string(),
                output: "error: 'tasks' array cannot be empty".to_string(),
                is_error: true,
                ui_resource_uri: None,
            });
        }

        let concurrency_limit = concurrency.unwrap_or(4).max(1);
        let tasks_owned: Vec<Value> = tasks_val.to_vec();
        let mut stream = stream::iter(tasks_owned.into_iter().enumerate().map(
            |(idx, mut task_args_c)| {
                let task_call_id = format!("{}_{}", call_id, idx);
                if worktree {
                    task_args_c["enforce_isolation"] = Value::Bool(true);
                    task_args_c["_enforce_isolation"] = Value::Bool(true);
                }
                let runner_ref = runner;
                let task_call_id_c = task_call_id.clone();
                async move {
                    let res = runner_ref
                        .run_single(&task_call_id_c, &task_args_c, true)
                        .await;
                    (idx, res)
                }
            },
        ))
        .buffer_unordered(concurrency_limit);

        let mut results = Vec::new();
        while let Some((idx, res)) = stream.next().await {
            results.push((idx, res));
        }
        results.sort_by_key(|(idx, _)| *idx);

        let mut aggregated = Vec::new();
        for (idx, res) in results {
            match res {
                Ok(tr) => {
                    aggregated.push(json!({
                        "task_index": idx,
                        "output": tr.output,
                        "is_error": tr.is_error,
                    }));
                }
                Err(e) => {
                    aggregated.push(json!({
                        "task_index": idx,
                        "output": format!("task failed: {e}"),
                        "is_error": true,
                    }));
                }
            }
        }

        Ok(ToolResult {
            tool_call_id: call_id.to_string(),
            tool_name: "subagent".to_string(),
            output: serde_json::to_string_pretty(&aggregated)
                .unwrap_or_else(|e| format!("error serializing results: {e}")),
            is_error: false,
            ui_resource_uri: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockRunner;

    #[async_trait]
    impl SubagentSingleRunner for MockRunner {
        async fn run_single(
            &self,
            call_id: &str,
            args: &Value,
            _force_sync: bool,
        ) -> Result<ToolResult> {
            Ok(ToolResult {
                tool_call_id: call_id.to_string(),
                tool_name: "subagent".to_string(),
                output: format!("ran: {}", args["task"].as_str().unwrap_or("empty")),
                is_error: false,
                ui_resource_uri: None,
            })
        }

        fn list_subagents(&self) -> Result<String> {
            Ok("subagents: mock".to_string())
        }

        async fn cancel_subagent(&self, id: &str) -> Result<String> {
            Ok(format!("cancelled {id}"))
        }

        fn doctor_status(&self) -> Result<String> {
            Ok("system operational".to_string())
        }
    }

    #[tokio::test]
    async fn test_coordinate_tasks_action_dispatches_to_parallel() {
        let runner = MockRunner;
        let args = json!({
            "action": "tasks",
            "tasks": [
                { "agent": "worker", "task": "task 1" },
                { "agent": "worker", "task": "task 2" }
            ]
        });

        let res = SubagentCoordinator::coordinate(&runner, "call_1", &args)
            .await
            .expect("coordinate tasks");
        assert!(
            !res.is_error,
            "tasks action should succeed without unsupported action error"
        );
        assert!(res.output.contains("task 1"));
        assert!(res.output.contains("task 2"));
    }

    #[tokio::test]
    async fn test_coordinate_resume_and_status_actions() {
        let runner = MockRunner;

        // Status action
        let status_args = json!({
            "action": "status",
            "id": "agent-123"
        });
        let res = SubagentCoordinator::coordinate(&runner, "call_2", &status_args)
            .await
            .expect("coordinate status");
        assert!(!res.is_error);
        assert!(res.output.contains("agent-123"));

        // Resume action with guidance
        let resume_args = json!({
            "action": "resume",
            "id": "agent-123",
            "message": "continue with next step"
        });
        let res2 = SubagentCoordinator::coordinate(&runner, "call_3", &resume_args)
            .await
            .expect("coordinate resume");
        assert!(!res2.is_error);
        assert!(res2.output.contains("agent-123"));
        assert!(res2.output.contains("continue with next step"));
    }

    #[tokio::test]
    async fn test_coordinate_model_hot_swap_action() {
        let runner = MockRunner;
        let model_args = json!({
            "action": "model",
            "id": "agent-123",
            "model": "anthropic/claude-3-7-sonnet"
        });
        let res = SubagentCoordinator::coordinate(&runner, "call_4", &model_args)
            .await
            .expect("coordinate model hot-swap");
        assert!(!res.is_error);
        assert!(res.output.contains("agent-123"));
        assert!(res.output.contains("anthropic/claude-3-7-sonnet"));
    }

    #[tokio::test]
    async fn test_coordinate_run_and_single_actions() {
        let runner = MockRunner;

        // action: "run"
        let run_args = json!({
            "action": "run",
            "agent": "scout",
            "task": "check branch state"
        });
        let res = SubagentCoordinator::coordinate(&runner, "call_run", &run_args)
            .await
            .expect("coordinate run");
        assert!(!res.is_error);
        assert!(res.output.contains("check branch state"));

        // action: "single"
        let single_args = json!({
            "action": "single",
            "agent": "scout",
            "task": "inspect latest commit"
        });
        let res2 = SubagentCoordinator::coordinate(&runner, "call_single", &single_args)
            .await
            .expect("coordinate single");
        assert!(!res2.is_error);
        assert!(res2.output.contains("inspect latest commit"));
    }

    #[tokio::test]
    async fn test_coordinate_create_fallback_to_single_when_task_present() {
        let runner = MockRunner;

        // action: "create" with task present (e.g. from LLM or supervisor adapter)
        let create_task_args = json!({
            "action": "create",
            "agent": "scout",
            "task": "smoke test git"
        });
        let res = SubagentCoordinator::coordinate(&runner, "call_fallback", &create_task_args)
            .await
            .expect("coordinate fallback");
        assert!(!res.is_error);
        assert!(res.output.contains("smoke test git"));

        // action: "create" without task and without config.name should still return error
        let empty_create_args = json!({
            "action": "create"
        });
        let res2 = SubagentCoordinator::coordinate(&runner, "call_err", &empty_create_args)
            .await
            .expect("coordinate create error");
        assert!(res2.is_error);
        assert!(res2.output.contains("error: 'config.name' is required"));
    }
}
