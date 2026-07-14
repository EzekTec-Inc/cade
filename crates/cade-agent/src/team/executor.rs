use super::config::TeamConfig;
use super::discovery::TeamDef;
use super::mode::TeamMode;
use serde::{Serialize, Deserialize};
use serde_json::Value;
use async_trait::async_trait;
use futures::StreamExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamResultItem {
    pub task_index: usize,
    pub output: String,
    pub is_error: bool,
}

#[async_trait]
pub trait SubagentRunner: Send + Sync {
    async fn run_subagent(
        &self,
        task_call_id: &str,
        args: &Value,
    ) -> Result<crate::tools::manager::ToolResult, String>;
}

#[async_trait]
pub trait LlmCompleter: Send + Sync {
    async fn complete(
        &self,
        model: &str,
        system_prompt: Option<&str>,
        prompt: &str,
    ) -> Result<String, String>;
}

pub struct TeamExecutor;

impl TeamExecutor {
    pub fn new() -> Self {
        Self
    }

    pub async fn run_team(
        &self,
        team_def: &TeamDef,
        config: &TeamConfig,
        parent_model: &str,
        tool_call_id: &str,
        runner: &dyn SubagentRunner,
        llm: &dyn LlmCompleter,
    ) -> Result<Vec<TeamResultItem>, String> {
        let active_mode = config.resolve_mode(Some(team_def));
        let mut members_to_run = team_def.members.clone();
        let prompt_val = config.task_with_test_command();

        // 🟢 1. TeamMode::Route (Specialist Routing Pass)
        if active_mode == TeamMode::Route {
            let mut roster = String::new();
            for m in &team_def.members {
                roster.push_str(&format!(
                    "- Member ID: {}\n  Role: {:?}\n  Description: {}\n\n",
                    m.id, m.role, m.description
                ));
            }

            let route_prompt = format!(
                "You are an AI router. Given the following user request and a roster of specialized team members, select the single best-suited member (or top 2 members if multiple specialists are required) to handle this request.\n\n\
                 USER REQUEST:\n{}\n\n\
                 ROSTER:\n{}\n\
                 Return a JSON array of the selected Member IDs, e.g. [\"id1\", \"id2\"]. Return ONLY the JSON array, no explanation.",
                prompt_val, roster
            );

            if let Ok(content) = llm.complete(parent_model, None, &route_prompt).await {
                let clean_json = content
                    .trim()
                    .trim_start_matches("```json")
                    .trim_end_matches("```")
                    .trim();
                if let Ok(Value::Array(arr)) = serde_json::from_str(clean_json) {
                    let selected_ids: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    if !selected_ids.is_empty() {
                        members_to_run.retain(|m| selected_ids.contains(&m.id));
                        tracing::info!(
                            "Router selected {} specialized members for execution: {:?}",
                            members_to_run.len(),
                            selected_ids
                        );
                    }
                }
            }
        }

        // 🟢 2. TeamMode::Tasks (Sequential Pipeline Execution)
        if active_mode == TeamMode::Tasks {
            let mut sequential_results = Vec::new();
            let mut previous_outputs = String::new();

            for (idx, member) in members_to_run.iter().enumerate() {
                let task_call_id = format!("{}_{}", tool_call_id, idx);
                let custom_prompt = if previous_outputs.is_empty() {
                    prompt_val.clone()
                } else {
                    format!(
                        "{}\n\n<previous_outputs>\n{}\n</previous_outputs>",
                        prompt_val, previous_outputs
                    )
                };

                let mut member_args = serde_json::Map::new();
                member_args.insert(
                    "prompt".to_string(),
                    Value::String(custom_prompt),
                );
                member_args.insert(
                    "description".to_string(),
                    Value::String(format!(
                        "Pipeline step [{}]: {} - {}",
                        idx + 1,
                        member.name,
                        member.description
                    )),
                );
                if !member.system_prompt.is_empty() {
                    member_args.insert(
                        "system_prompt".to_string(),
                        Value::String(member.system_prompt.clone()),
                    );
                }
                if let Some(model) = &member.model {
                    member_args.insert(
                        "model".to_string(),
                        Value::String(model.clone()),
                    );
                }

                let task_args_json = Value::Object(member_args);
                let tr = runner.run_subagent(&task_call_id, &task_args_json).await;

                match tr {
                    Ok(result) => {
                        if !result.is_error {
                            previous_outputs.push_str(&format!(
                                "\n--- Output of {} ---\n{}\n",
                                member.name, result.output
                            ));
                        }
                        sequential_results.push(TeamResultItem {
                            task_index: idx,
                            output: result.output,
                            is_error: result.is_error,
                        });
                    }
                    Err(err) => {
                        sequential_results.push(TeamResultItem {
                            task_index: idx,
                            output: format!("execution error: {err}"),
                            is_error: true,
                        });
                    }
                }
            }

            return Ok(sequential_results);
        }

        // For Coordinate or Broadcast, prepare and execute tasks concurrently
        let mut tasks_val = Vec::new();
        for member in &members_to_run {
            let mut member_args = serde_json::Map::new();
            member_args.insert(
                "prompt".to_string(),
                Value::String(prompt_val.clone()),
            );
            member_args.insert(
                "description".to_string(),
                Value::String(format!(
                    "Team member: {} - {}",
                    member.name, member.description
                )),
            );
            if !member.system_prompt.is_empty() {
                member_args.insert(
                    "system_prompt".to_string(),
                    Value::String(member.system_prompt.clone()),
                );
            }
            if let Some(model) = &member.model {
                member_args.insert(
                    "model".to_string(),
                    Value::String(model.clone()),
                );
            }
            tasks_val.push(Value::Object(member_args));
        }

        if tasks_val.is_empty() {
            return Err("error: task list cannot be empty (team may have no members)".to_string());
        }

        // Prepare futures for concurrent execution
        let mut futures = Vec::new();
        for (idx, task_args) in tasks_val.iter().enumerate() {
            let task_call_id = format!("{}_{}", tool_call_id, idx);
            let runner_ref = runner;
            let task_args_c = task_args.clone();

            futures.push(Box::pin(async move {
                runner_ref.run_subagent(&task_call_id, &task_args_c).await
            }));
        }

        let concurrency_cap = std::env::var("CADE_MAX_SUBAGENTS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(4);

        let results = futures::stream::iter(futures)
            .buffered(concurrency_cap)
            .collect::<Vec<_>>()
            .await;

        let mut aggregated = Vec::new();
        for (idx, tr) in results.into_iter().enumerate() {
            match tr {
                Ok(result) => {
                    aggregated.push(TeamResultItem {
                        task_index: idx,
                        output: result.output,
                        is_error: result.is_error,
                    });
                }
                Err(err) => {
                    aggregated.push(TeamResultItem {
                        task_index: idx,
                        output: format!("execution error: {err}"),
                        is_error: true,
                    });
                }
            }
        }

        // 🟢 3. TeamMode::Coordinate (Orchestrated Leader Synthesis)
        if active_mode == TeamMode::Coordinate {
            let mut reports = String::new();
            for (idx, tr) in aggregated.iter().enumerate() {
                let name = &members_to_run[idx].name;
                reports.push_str(&format!("--- Report from {} ---\n{}\n\n", name, tr.output));
            }

            let coord_prompt = format!(
                "You are a Team Coordinator. You have delegated a task to multiple specialized subagents. Below are their individual reports. Consolidate their findings into a single, coherent, unified master report. Resolve any conflicts or redundancies.\n\n\
                 ORIGINAL TASK:\n{}\n\n\
                 SUBAGENT REPORTS:\n{}\n\
                 Write a clear, structured, and comprehensive final report.",
                prompt_val, reports
            );

            if let Ok(content) = llm.complete(parent_model, None, &coord_prompt).await {
                return Ok(vec![TeamResultItem {
                    task_index: 0,
                    output: content.trim().to_string(),
                    is_error: false,
                }]);
            }
        }

        Ok(aggregated)
    }
}

impl Default for TeamExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::member::MemberDef;
    use crate::team::discovery::TeamDef;
    use crate::team::member::MemberTools;
    use crate::team::member::MemberScope;
    use crate::tools::manager::ToolResult;
    use std::sync::{Arc, Mutex};

    struct MockRunner {
        calls: Arc<Mutex<Vec<(String, Value)>>>,
    }

    #[async_trait]
    impl SubagentRunner for MockRunner {
        async fn run_subagent(
            &self,
            task_call_id: &str,
            args: &Value,
        ) -> Result<ToolResult, String> {
            self.calls.lock().unwrap().push((task_call_id.to_string(), args.clone()));
            Ok(ToolResult {
                tool_call_id: task_call_id.to_string(),
                tool_name: "run_subagent".to_string(),
                output: format!("output from {}", task_call_id),
                is_error: false,
                ui_resource_uri: None,
            })
        }
    }

    struct MockLlm {
        completions: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl LlmCompleter for MockLlm {
        async fn complete(
            &self,
            _model: &str,
            _system_prompt: Option<&str>,
            prompt: &str,
        ) -> Result<String, String> {
            self.completions.lock().unwrap().push(prompt.to_string());
            Ok("[\"m1\"]".to_string())
        }
    }

    #[tokio::test]
    async fn test_tasks_sequential_pipeline() {
        let m1 = MemberDef {
            id: "m1".to_string(),
            name: "Member 1".to_string(),
            role: None,
            description: "First member".to_string(),
            model: None,
            tools: MemberTools::All,
            system_prompt: "".to_string(),
            skills: vec![],
            scope: MemberScope::Builtin,
            path: None,
        };
        let m2 = MemberDef {
            id: "m2".to_string(),
            name: "Member 2".to_string(),
            role: None,
            description: "Second member".to_string(),
            model: None,
            tools: MemberTools::All,
            system_prompt: "".to_string(),
            skills: vec![],
            scope: MemberScope::Builtin,
            path: None,
        };

        let team_def = TeamDef {
            id: "test_team".to_string(),
            name: "Test Team".to_string(),
            description: "".to_string(),
            mode: TeamMode::Tasks,
            max_iterations: 10,
            leader_model: None,
            members: vec![m1, m2],
            scope: MemberScope::Builtin,
            path: None,
        };

        let args = serde_json::json!({
            "prompt": "solve world peace",
            "team_id": "test_team",
            "mode": "tasks",
        });
        let config = TeamConfig::from_args(&args);

        let calls = Arc::new(Mutex::new(vec![]));
        let runner = MockRunner { calls: calls.clone() };

        let completions = Arc::new(Mutex::new(vec![]));
        let llm = MockLlm { completions: completions.clone() };

        let executor = TeamExecutor::new();
        let results = executor
            .run_team(&team_def, &config, "gpt-4o", "call_123", &runner, &llm)
            .await
            .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].task_index, 0);
        assert_eq!(results[0].output, "output from call_123_0");
        assert_eq!(results[1].task_index, 1);
        assert_eq!(results[1].output, "output from call_123_1");

        let call_list = calls.lock().unwrap();
        assert_eq!(call_list.len(), 2);
        assert_eq!(call_list[0].0, "call_123_0");
        assert_eq!(call_list[1].0, "call_123_1");

        // Verify that output from step 1 propagated to step 2's prompt
        let second_prompt = call_list[1].1["prompt"].as_str().unwrap();
        assert!(second_prompt.contains("output from call_123_0"));
    }
}
