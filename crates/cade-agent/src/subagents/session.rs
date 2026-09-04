//! Autonomous SubagentSession Execution Harness (ADR-0021 / Issues #49, #50, #51).
//!
//! Encapsulates the execution loop, canonical finish tool injection,
//! dual budget enforcement (max_iters & max_tokens_budget), RAII workspace isolation,
//! real-time telemetry streaming, and structured outcome models.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

use super::config::SubagentConfig;
use super::workspace_guard::IsolatedWorkspaceGuard;

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

/// Structured memory finding produced during subagent execution for writeback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentFinding {
    pub label: String,
    pub value: String,
    pub description: String,
    pub memory_type: String,
    pub confidence: f64,
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

/// Real-time event emitted during a subagent session (Issue #51 / Issue #89).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum SubagentEvent {
    TurnStarted {
        turn: usize,
        max_turns: usize,
    },
    Thought {
        text: String,
    },
    ToolExecuting {
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
    },
    ToolCompleted {
        tool_call_id: String,
        tool_name: String,
        is_error: bool,
    },
    Progress {
        percent: f64,
        message: Option<String>,
    },
    ApprovalRequired {
        tool_name: String,
        arguments: Value,
        approval_id: String,
    },
    ApprovalResolved {
        approval_id: String,
        approved: bool,
        feedback: Option<String>,
    },
    OutputChunk {
        text: String,
    },
    Finished {
        outcome: SubagentOutcome,
    },
}

/// Asynchronous event broadcaster for subagents supporting unicast & broadcast subscribers.
#[derive(Clone, Default)]
pub struct SubagentEventEmitter {
    tx: Option<tokio::sync::mpsc::Sender<SubagentEvent>>,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<SubagentEvent>>,
}

impl SubagentEventEmitter {
    pub fn new(tx: Option<tokio::sync::mpsc::Sender<SubagentEvent>>) -> Self {
        Self {
            tx,
            broadcast_tx: None,
        }
    }

    pub fn with_broadcast(mut self, broadcast_tx: tokio::sync::broadcast::Sender<SubagentEvent>) -> Self {
        self.broadcast_tx = Some(broadcast_tx);
        self
    }

    pub fn noop() -> Self {
        Self {
            tx: None,
            broadcast_tx: None,
        }
    }

    pub async fn emit(&self, event: SubagentEvent) {
        if let Some(ref tx) = self.tx {
            let _ = tx.send(event.clone()).await;
        }
        if let Some(ref btx) = self.broadcast_tx {
            let _ = btx.send(event);
        }
    }
}

/// Human-In-The-Loop approval verdict.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentApprovalResponse {
    pub approved: bool,
    pub feedback: Option<String>,
}

pub type ApprovalResponder = tokio::sync::oneshot::Sender<SubagentApprovalResponse>;
pub type ApprovalRequestPayload = (String, String, Value, ApprovalResponder);

/// Channel for intercepting and requesting human-in-the-loop approvals.
#[derive(Clone, Default)]
pub struct SubagentApprovalChannel {
    tx: Option<tokio::sync::mpsc::Sender<ApprovalRequestPayload>>,
}

impl SubagentApprovalChannel {
    pub fn new(tx: tokio::sync::mpsc::Sender<ApprovalRequestPayload>) -> Self {
        Self { tx: Some(tx) }
    }

    pub fn noop() -> Self {
        Self { tx: None }
    }

    /// Dispatch an approval request and wait asynchronously for human approval or feedback.
    pub async fn request_approval(
        &self,
        approval_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<SubagentApprovalResponse, String> {
        if let Some(ref tx) = self.tx {
            let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
            tx.send((
                approval_id.to_string(),
                tool_name.to_string(),
                arguments.clone(),
                resp_tx,
            ))
            .await
            .map_err(|e| format!("Failed to dispatch approval request: {e}"))?;
            resp_rx
                .await
                .map_err(|_| "Approval channel closed without response".to_string())
        } else {
            Ok(SubagentApprovalResponse {
                approved: true,
                feedback: None,
            })
        }
    }
}

/// Autonomous session harness for running subagents.
pub struct SubagentSession {
    pub session_id: String,
    pub parent_agent_id: String,
    pub config: SubagentConfig,
    pub max_iters: usize,
    pub max_tokens_budget: Option<u64>,
    pub current_iteration: usize,
    pub cumulative_tokens: u64,
    pub total_tool_calls: usize,
    pub workspace_guard: Option<IsolatedWorkspaceGuard>,
    pub event_emitter: SubagentEventEmitter,
    pub findings: Vec<SubagentFinding>,
    pub approval_channel: SubagentApprovalChannel,
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
            workspace_guard: None,
            event_emitter: SubagentEventEmitter::noop(),
            findings: Vec::new(),
            approval_channel: SubagentApprovalChannel::noop(),
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

    pub fn with_workspace_guard(mut self, guard: IsolatedWorkspaceGuard) -> Self {
        self.workspace_guard = Some(guard);
        self
    }

    pub fn with_event_emitter(mut self, emitter: SubagentEventEmitter) -> Self {
        self.event_emitter = emitter;
        self
    }

    pub fn with_approval_channel(mut self, channel: SubagentApprovalChannel) -> Self {
        self.approval_channel = channel;
        self
    }

    /// Record a structured finding to be synced back to the parent agent.
    pub fn record_finding(
        &mut self,
        label: impl Into<String>,
        value: impl Into<String>,
        description: impl Into<String>,
        memory_type: impl Into<String>,
        confidence: f64,
    ) {
        self.findings.push(SubagentFinding {
            label: label.into(),
            value: value.into(),
            description: description.into(),
            memory_type: memory_type.into(),
            confidence,
        });
    }

    pub fn add_finding(&mut self, finding: SubagentFinding) {
        self.findings.push(finding);
    }

    pub fn findings(&self) -> &[SubagentFinding] {
        &self.findings
    }

    /// Return the execution working directory for tools (isolated if active, else primary).
    pub fn execution_path<'a>(&'a self, fallback_primary: &'a Path) -> &'a Path {
        self.workspace_guard
            .as_ref()
            .and_then(|g| g.path())
            .unwrap_or(fallback_primary)
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
    pub async fn record_turn(&mut self, tokens_used: u64, tool_calls_count: usize) {
        self.current_iteration += 1;
        self.cumulative_tokens += tokens_used;
        self.total_tool_calls += tool_calls_count;
        self.event_emitter
            .emit(SubagentEvent::TurnStarted {
                turn: self.current_iteration,
                max_turns: self.max_iters,
            })
            .await;
    }

    /// Finalize execution outcome, committing workspace if successful and emitting event.
    pub async fn finalize_outcome(&mut self, outcome: SubagentOutcome) -> SubagentOutcome {
        if outcome.is_success()
            && let Some(ref mut guard) = self.workspace_guard
            && let Err(e) = guard.commit_and_merge().await
        {
            let err_msg = format!("Failed to merge isolated workspace changes back: {e}");
            let failed_outcome = SubagentOutcome::Failed { error: err_msg };
            self.event_emitter
                .emit(SubagentEvent::Finished {
                    outcome: failed_outcome.clone(),
                })
                .await;
            return failed_outcome;
        }
        self.event_emitter
            .emit(SubagentEvent::Finished {
                outcome: outcome.clone(),
            })
            .await;
        outcome
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
    use tempfile::tempdir;

    #[test]
    fn test_finish_tool_schema_structure() {
        let schema = canonical_finish_tool_schema();
        assert_eq!(schema["name"], FINISH_TOOL_NAME);
        assert_eq!(schema["parameters"]["type"], "object");
        assert!(schema["parameters"]["properties"]["status"].is_object());
    }

    #[tokio::test]
    async fn test_subagent_session_budget_exhaustion() {
        let config = SubagentConfig::from_args(&json!({ "prompt": "Test task" }));
        let mut session = SubagentSession::new(config, "parent-1")
            .with_max_iters(3)
            .with_max_tokens_budget(Some(100));

        assert!(session.is_budget_exhausted().is_none());

        session.record_turn(40, 1).await;
        assert!(session.is_budget_exhausted().is_none());

        session.record_turn(70, 1).await; // 110 total > 100
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

    #[tokio::test]
    async fn test_subagent_session_finalize_workspace_merge() -> std::io::Result<()> {
        let temp_primary = tempdir()?;
        let primary_file = temp_primary.path().join("code.rs");
        std::fs::write(&primary_file, "initial")?;

        let guard = IsolatedWorkspaceGuard::new(temp_primary.path(), None).await?;
        let config = SubagentConfig::from_args(&json!({ "prompt": "Refactor code" }));

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let emitter = SubagentEventEmitter::new(Some(tx));

        let mut session = SubagentSession::new(config, "parent-1")
            .with_workspace_guard(guard)
            .with_event_emitter(emitter);

        // Mutate isolated file
        let isolated_file = session
            .workspace_guard
            .as_ref()
            .unwrap()
            .path()
            .unwrap()
            .join("code.rs");
        std::fs::write(&isolated_file, "refactored")?;

        // Finalize with Success
        let outcome = SubagentOutcome::Done {
            summary: "Refactor finished".to_string(),
            iterations: 1,
            tool_calls_count: 1,
            token_usage: 50,
        };
        let final_res = session.finalize_outcome(outcome).await;
        assert!(final_res.is_success());

        // Verify primary received merged content
        assert_eq!(std::fs::read_to_string(&primary_file)?, "refactored");

        // Verify Finished event was emitted
        let event = rx.recv().await.expect("Event emitted");
        if let SubagentEvent::Finished { outcome } = event {
            assert!(outcome.is_success());
        } else {
            panic!("Expected Finished event");
        }
        Ok(())
    }

    #[test]
    fn test_subagent_session_findings_recording() {
        let config = SubagentConfig::from_args(&json!({ "prompt": "Research API patterns" }));
        let mut session = SubagentSession::new(config, "parent-agent-123");

        assert!(session.findings().is_empty());
        session.record_finding("api_convention", "REST with JSON", "Discovered in repo", "convention", 0.95);

        assert_eq!(session.findings().len(), 1);
        let finding = &session.findings()[0];
        assert_eq!(finding.label, "api_convention");
        assert_eq!(finding.value, "REST with JSON");
        assert_eq!(finding.memory_type, "convention");
    }

    #[tokio::test]
    async fn test_subagent_event_emitter_broadcast() {
        let (btx, mut brx1) = tokio::sync::broadcast::channel(16);
        let mut brx2 = btx.subscribe();

        let emitter = SubagentEventEmitter::noop().with_broadcast(btx);
        emitter.emit(SubagentEvent::Thought { text: "Analyzing code...".to_string() }).await;

        let e1 = brx1.recv().await.expect("Subscriber 1 received event");
        let e2 = brx2.recv().await.expect("Subscriber 2 received event");

        assert_eq!(e1, SubagentEvent::Thought { text: "Analyzing code...".to_string() });
        assert_eq!(e2, e1);
    }

    #[tokio::test]
    async fn test_subagent_approval_channel_flow() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let channel = SubagentApprovalChannel::new(tx);

        let approval_task = tokio::spawn(async move {
            channel
                .request_approval(
                    "appr-1",
                    "write_file",
                    &json!({ "path": "src/main.rs" }),
                )
                .await
        });

        let (appr_id, tool_name, args, responder) = rx.recv().await.expect("Received approval request");
        assert_eq!(appr_id, "appr-1");
        assert_eq!(tool_name, "write_file");
        assert_eq!(args["path"], "src/main.rs");

        responder
            .send(SubagentApprovalResponse {
                approved: true,
                feedback: Some("Approved with caution".to_string()),
            })
            .expect("Sent approval");

        let verdict = approval_task.await.expect("Task completed").expect("Approval succeeded");
        assert!(verdict.approved);
        assert_eq!(verdict.feedback.as_deref(), Some("Approved with caution"));
    }
}