//! Autonomous SubagentSession Execution Harness (ADR-0021).
//!
//! Encapsulates the execution loop, canonical finish tool injection,
//! dual budget enforcement (max_iters & max_tokens_budget), and structured outcome models.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::config::SubagentConfig;

/// Canonical finish tool name
pub const FINISH_TOOL_NAME: &str = "finish";

/// Returns the standard OpenAI/JSON-compatible tool schema for the canonical `finish` tool.
pub fn canonical_finish_tool_schema() -> Value {
    json!({
        "name": FINISH_TOOL_NAME,
        "description": "Signal task completion or a definitive block. Must be called to end the subagent session. Use status='done' when complete, 'blocked' when stuck, 'error' on failure.",
        "parameters": {
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["done", "blocked", "error"],
                    "description": "The completion status of the subagent task."
                },
                "summary": {
                    "type": "string",
                    "description": "Concise summary of what was accomplished, or the reason why execution is blocked/failed."
                },
                "questions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional clarifying questions when status='blocked'."
                }
            },
            "required": ["status", "summary"]
        }
    })
}

/// Structured outcome produced when a subagent session terminates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum SubagentOutcome {
    Done {
        summary: String,
        iterations: usize,
        tool_calls_count: usize,
        token_usage: usize,
    },
    Blocked {
        reason: String,
        questions: Vec<String>,
    },
    Failed {
        error: String,
    },
    Exhausted {
        reason: String,
        iterations: usize,
        tokens_used: usize,
    },
}

impl SubagentOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Done { .. })
    }

    pub fn summary_text(&self) -> &str {
        match self {
            Self::Done { summary, .. } => summary.as_str(),
            Self::Blocked { reason, .. } => reason.as_str(),
            Self::Failed { error } => error.as_str(),
            Self::Exhausted { reason, .. } => reason.as_str(),
        }
    }
}

/// Autonomous session harness for running subagents.
#[derive(Debug, Clone)]
pub struct SubagentSession {
    pub session_id: String,
    pub parent_agent_id: String,
    pub config: SubagentConfig,
    pub max_iters: usize,
    pub max_tokens_budget: Option<u64>,
    pub current_iteration: usize,
    pub cumulative_tokens: u64,
    pub total_tool_calls: usize,
}

impl SubagentSession {
    /// Create a new subagent session instance.
    pub fn new(config: SubagentConfig, parent_agent_id: impl Into<String>) -> Self {
        let max_tokens = config.max_tokens_budget;
        Self {
            session_id: format!("subagent-sess-{}", uuid::Uuid::new_v4()),
            parent_agent_id: parent_agent_id.into(),
            config,
            max_iters: 20,
            max_tokens_budget: max_tokens,
            current_iteration: 0,
            cumulative_tokens: 0,
            total_tool_calls: 0,
        }
    }

    pub fn with_max_iters(mut self, max_iters: usize) -> Self {
        self.max_iters = max_iters;
        self
    }

    pub fn with_max_tokens_budget(mut self, budget: Option<u64>) -> Self {
        self.max_tokens_budget = budget;
        self
    }

    /// Check if either iteration or token budget limits have been reached.
    pub fn is_budget_exhausted(&self) -> Option<String> {
        if self.current_iteration >= self.max_iters {
            return Some(format!(
                "Iteration limit of {} reached without explicit task completion",
                self.max_iters
            ));
        }
        if let Some(budget) = self.max_tokens_budget
            && self.cumulative_tokens >= budget
        {
            return Some(format!(
                "Token budget limit of {} tokens exceeded (used: {})",
                budget, self.cumulative_tokens
            ));
        }
        None
    }

    /// Record turn step progress and token usage.
    pub fn record_turn(&mut self, tokens_used: u64, tool_calls_count: usize) {
        self.current_iteration += 1;
        self.cumulative_tokens += tokens_used;
        self.total_tool_calls += tool_calls_count;
    }

    /// Inspect a tool call to determine if it is the canonical `finish` tool.
    pub fn check_finish_tool_call(tool_name: &str, arguments: &Value) -> Option<SubagentOutcome> {
        if tool_name != FINISH_TOOL_NAME {
            return None;
        }

        let status = arguments["status"].as_str().unwrap_or("done");
        let summary = arguments["summary"]
            .as_str()
            .unwrap_or("Task completed.")
            .to_string();

        match status {
            "done" => Some(SubagentOutcome::Done {
                summary,
                iterations: 0,
                tool_calls_count: 0,
                token_usage: 0,
            }),
            "blocked" => {
                let questions = arguments["questions"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(SubagentOutcome::Blocked {
                    reason: summary,
                    questions,
                })
            }
            "error" => Some(SubagentOutcome::Failed { error: summary }),
            _ => Some(SubagentOutcome::Done {
                summary,
                iterations: 0,
                tool_calls_count: 0,
                token_usage: 0,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finish_tool_schema_structure() {
        let schema = canonical_finish_tool_schema();
        assert_eq!(schema["name"], FINISH_TOOL_NAME);
        assert_eq!(schema["parameters"]["type"], "object");
        assert!(schema["parameters"]["properties"]["status"].is_object());
    }

    #[test]
    fn test_subagent_session_budget_exhaustion() {
        let config = SubagentConfig::from_args(&json!({ "prompt": "Test task" }));
        let mut session = SubagentSession::new(config, "parent-1")
            .with_max_iters(3)
            .with_max_tokens_budget(Some(100));

        assert!(session.is_budget_exhausted().is_none());

        session.record_turn(40, 1);
        assert!(session.is_budget_exhausted().is_none());

        session.record_turn(70, 1); // 110 total > 100
        assert!(session.is_budget_exhausted().is_some());
    }

    #[test]
    fn test_check_finish_tool_call() {
        let args = json!({
            "status": "done",
            "summary": "All tests pass"
        });
        let outcome = SubagentSession::check_finish_tool_call(FINISH_TOOL_NAME, &args);
        assert!(outcome.is_some());
        if let Some(SubagentOutcome::Done { summary, .. }) = outcome {
            assert_eq!(summary, "All tests pass");
        } else {
            panic!("Expected SubagentOutcome::Done");
        }
    }
}