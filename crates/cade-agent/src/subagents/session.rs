//! Autonomous SubagentSession Execution Harness (ADR-0021 / Issues #49, #50, #51).
//!
//! Encapsulates the execution loop, canonical finish tool injection,
//! dual budget enforcement (max_iters & max_tokens_budget), RAII workspace isolation,
//! real-time telemetry streaming, and structured outcome models.

use async_trait::async_trait;
use cade_core::permissions::{PermissionManager, Verdict, is_write_schema, path_is_protected};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

use super::SubagentTools;

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

/// Message representation within an autonomous subagent execution turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<SubagentToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl SubagentMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(
        content: impl Into<String>,
        tool_calls: Option<Vec<SubagentToolCall>>,
    ) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// Tool invocation request within a turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// Turn completion response returned by a subagent LLM executor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentTurnResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<SubagentToolCall>,
    pub tokens_used: u64,
}

/// Abstraction for LLM completion driving subagent turns.
#[async_trait]
pub trait SubagentLlmExecutor: Send + Sync {
    async fn complete_turn(
        &self,
        model: &str,
        system_prompt: &str,
        messages: &[SubagentMessage],
        tools: &[Value],
    ) -> Result<SubagentTurnResponse, String>;
}

/// Abstraction for executing tools during a subagent session.
#[async_trait]
pub trait SubagentToolExecutor: Send + Sync {
    /// Consult the live capability metadata, not just the spelling of an MCP tool.
    async fn is_mcp_write(&self, _tool_name: &str) -> bool {
        false
    }

    async fn execute_tool(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &Value,
        execution_path: &Path,
    ) -> Result<String, String>;
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

    pub fn with_broadcast(
        mut self,
        broadcast_tx: tokio::sync::broadcast::Sender<SubagentEvent>,
    ) -> Self {
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
            Err("No interactive approval adapter available".to_string())
        }
    }
}

/// Execution authority, independent of the schemas sent to the model.
#[derive(Clone)]
pub struct SubagentToolPolicy {
    pub permissions: PermissionManager,
    pub tools: SubagentTools,
    /// Names inherited from the parent's actual capability catalog (before child filtering).
    pub inherited_tools: Vec<String>,
    pub allow_nesting: bool,
    pub max_depth: usize,
}

impl SubagentToolPolicy {
    pub fn permits_name(
        &self,
        name: &str,
        is_mcp_write: bool,
        mode: &str,
        depth: usize,
    ) -> Result<(), String> {
        if !self.inherited_tools.iter().any(|n| n == name) {
            return Err(format!("Tool '{name}' is not inherited from the parent"));
        }
        if matches!(name, "run_subagent" | "run_parallel_subagents" | "subagent")
            && (!self.allow_nesting || depth + 1 >= self.max_depth)
        {
            return Err("Nested subagent delegation is not permitted at this depth".into());
        }
        let readonly =
            mode == "plan" || mode == "recall" || matches!(self.tools, SubagentTools::Readonly);
        if readonly && (is_write_schema(name) || is_mcp_write || matches!(name, "bash" | "shell")) {
            return Err(format!("Read-only subagent cannot execute '{name}'"));
        }
        if readonly && name.contains("__") && !readonly_mcp_name(name) {
            return Err(format!("MCP tool '{name}' is not a known read capability"));
        }
        match &self.tools {
            SubagentTools::All => {}
            SubagentTools::Readonly => {
                // A read-only definition can use inherited MCP read capabilities.
                if !matches!(
                    name,
                    "read_file"
                        | "glob"
                        | "grep"
                        | "search_memory"
                        | "conversation_search"
                        | "archival_memory_search"
                        | "recall"
                        | "fetch_doc"
                ) && !(name.contains("__") && readonly_mcp_name(name))
                {
                    return Err(format!("Tool '{name}' is not in the read-only tool set"));
                }
            }
            SubagentTools::List(names) => {
                if !names.iter().any(|n| n == name) {
                    return Err(format!("Tool '{name}' is not allowed by child definition"));
                }
            }
            SubagentTools::Restricted { allowed_tools, .. } => {
                if !allowed_tools.iter().any(|n| n == name) {
                    return Err(format!("Tool '{name}' is not allowed by child definition"));
                }
            }
        }
        Ok(())
    }

    fn permits_path(&self, name: &str, args: &Value, cwd: &Path) -> Result<(), String> {
        let SubagentTools::Restricted { allowed_paths, .. } = &self.tools else {
            return Ok(());
        };
        if !matches!(
            name,
            "read_file" | "write_file" | "edit_file" | "apply_patch" | "grep" | "glob"
        ) {
            return Ok(());
        }
        let path = args
            .get("path")
            .or_else(|| args.get("file_path"))
            .and_then(Value::as_str)
            .ok_or("Restricted file tool requires a path")?;
        let absolute = if Path::new(path).is_absolute() {
            Path::new(path).to_path_buf()
        } else {
            cwd.join(path)
        };
        let resolved = absolute
            .canonicalize()
            .or_else(|_| {
                absolute
                    .parent()
                    .ok_or_else(|| std::io::Error::other("missing parent"))?
                    .canonicalize()
                    .map(|p| p.join(absolute.file_name().unwrap_or_default()))
            })
            .map_err(|e| format!("Cannot validate restricted path: {e}"))?;
        if allowed_paths.iter().any(|allowed| {
            let p = Path::new(allowed);
            let absolute_allowed = if p.is_absolute() {
                p.to_path_buf()
            } else {
                cwd.join(p)
            };
            absolute_allowed
                .canonicalize()
                .is_ok_and(|p| resolved.starts_with(p))
        }) {
            Ok(())
        } else {
            Err(format!("Path '{}' is outside the allowed paths", path))
        }
    }
}

fn readonly_mcp_name(name: &str) -> bool {
    [
        "read", "find", "get", "list", "search", "inspect", "describe", "show", "view", "check",
        "status", "select", "ask", "query", "skeleton", "extract",
    ]
    .iter()
    .any(|part| name.rsplit("__").next().unwrap_or(name).contains(part))
}

fn targets_protected_path(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, v)| {
            if matches!(
                key.as_str(),
                "path"
                    | "file_path"
                    | "filename"
                    | "command"
                    | "cmd"
                    | "patch"
                    | "source"
                    | "destination"
            ) {
                v.as_str().is_some_and(path_is_protected)
            } else {
                targets_protected_path(v)
            }
        }),
        Value::Array(values) => values.iter().any(targets_protected_path),
        _ => false,
    }
}

/// Autonomous session harness for running subagents.
pub struct SubagentSession {
    pub session_id: String,
    pub parent_agent_id: String,
    pub parent_conversation_id: Option<String>,
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
    pub tool_policy: Option<SubagentToolPolicy>,
    pub steering_queue: Vec<String>,
    pub pending_model_swap: Option<String>,
    parent_context: Vec<SubagentMessage>,
}

impl SubagentSession {
    /// Create a new subagent session instance.
    pub fn new(config: SubagentConfig, parent_agent_id: impl Into<String>) -> Self {
        let max_tokens = config.max_tokens_budget;
        Self {
            session_id: format!("subagent-sess-{}", uuid::Uuid::new_v4()),
            parent_agent_id: parent_agent_id.into(),
            parent_conversation_id: None,
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
            tool_policy: None,
            steering_queue: Vec::new(),
            pending_model_swap: None,
            parent_context: Vec::new(),
        }
    }

    /// Bind this child to the conversation that invoked it. `None` preserves
    /// the legacy unscoped CLI/foreground invocation.
    pub fn with_parent_conversation_id(mut self, conversation_id: Option<String>) -> Self {
        self.parent_conversation_id = conversation_id;
        self
    }

    /// Supply the invoking conversation's recent messages, never an agent-wide
    /// "latest conversation". Only the last eight messages enter the prompt.
    pub fn with_parent_context(mut self, messages: Vec<SubagentMessage>) -> Self {
        self.parent_context = messages.into_iter().rev().take(8).collect();
        self.parent_context.reverse();
        self
    }

    fn bounded_parent_context(&self) -> String {
        if self.parent_context.is_empty() {
            return String::new();
        }
        let mut context = String::from(
            "\n\n<parent_context>\nBelow is the recent chat history from your parent session. Use this to understand the current work context, recently viewed files, and goals:\n",
        );
        for message in &self.parent_context {
            let text = &message.content;
            let truncated = text.lines().take(5).collect::<Vec<_>>().join("\n");
            let suffix = if text.lines().count() > 5 {
                " ... [truncated]"
            } else {
                ""
            };
            context.push_str(&format!(
                "[{}]: {truncated}{suffix}\n",
                message.role.to_uppercase()
            ));
        }
        context.push_str("</parent_context>\n");
        context
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

    /// Prepare an isolated workspace before the child can invoke any tools.
    pub async fn prepare_workspace(
        &mut self,
        primary_path: &Path,
        branch_name: Option<String>,
    ) -> std::io::Result<()> {
        self.workspace_guard = None;
        self.workspace_guard = Some(IsolatedWorkspaceGuard::new(primary_path, branch_name).await?);
        Ok(())
    }

    pub fn with_event_emitter(mut self, emitter: SubagentEventEmitter) -> Self {
        self.event_emitter = emitter;
        self
    }

    pub fn with_approval_channel(mut self, channel: SubagentApprovalChannel) -> Self {
        self.approval_channel = channel;
        self
    }

    pub fn with_tool_policy(mut self, policy: SubagentToolPolicy) -> Self {
        self.tool_policy = Some(policy);
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
        let outcome = if outcome.is_success() {
            if let Some(ref mut guard) = self.workspace_guard {
                match guard.commit_and_merge().await {
                    Ok(()) => outcome,
                    Err(e) => SubagentOutcome::Failed {
                        error: format!("Failed to merge isolated workspace changes back: {e}"),
                    },
                }
            } else {
                outcome
            }
        } else {
            outcome
        };
        // Release the workspace on every terminal outcome, not only when the
        // session itself is eventually dropped.
        self.workspace_guard = None;
        self.event_emitter
            .emit(SubagentEvent::Finished {
                outcome: outcome.clone(),
            })
            .await;
        outcome
    }

    /// Enqueue a steering message to be prioritized on the subagent's subsequent turn.
    pub fn steer(&mut self, message: String) -> bool {
        self.steering_queue.push(message);
        true
    }

    /// Request a model hot-swap taking effect on the subsequent turn.
    pub fn hot_swap_model(&mut self, new_model: String) -> bool {
        self.pending_model_swap = Some(new_model);
        true
    }

    pub fn take_pending_steering(&mut self) -> Vec<String> {
        std::mem::take(&mut self.steering_queue)
    }

    pub fn take_pending_model_hot_swap(&mut self) -> Option<String> {
        self.pending_model_swap.take()
    }

    /// Inspect a tool call to determine if it is the canonical `finish` or `finish_task` tool.
    pub fn check_finish_tool_call(tool_name: &str, arguments: &Value) -> Option<SubagentOutcome> {
        if tool_name != FINISH_TOOL_NAME && tool_name != "finish_task" {
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

    /// Execute the full autonomous reasoning loop until completion, budget exhaustion, or error.
    pub async fn run_autonomous_loop<L: SubagentLlmExecutor, T: SubagentToolExecutor>(
        &mut self,
        llm: &L,
        tools: &T,
        mut model: String,
        system_prompt: String,
        initial_prompt: String,
        tool_schemas: Vec<Value>,
        failover_models: Vec<String>,
        primary_path: &Path,
    ) -> SubagentOutcome {
        if self.config.enforce_isolation && self.workspace_guard.is_none() {
            return self
                .finalize_outcome(SubagentOutcome::Failed {
                    error: "Required subagent isolation could not be established; refusing to run in the live workspace".into(),
                })
                .await;
        }
        let system_prompt = format!("{system_prompt}{}", self.bounded_parent_context());
        let mut messages = vec![SubagentMessage::user(initial_prompt)];
        let mut last_text = String::new();
        let mut failover_idx = 0;

        for _iter in 0..self.max_iters {
            // 1. Dynamic Model Hot-Swap check
            if let Some(new_model) = self.take_pending_model_hot_swap()
                && new_model != model
            {
                model = new_model;
            }

            // 2. Priority Steering Guidance Queue Drain
            let steer_msgs = self.take_pending_steering();
            if !steer_msgs.is_empty() {
                let guidance = format!(
                    "[Supervisor Steering Guidance]:\n\n{}",
                    steer_msgs.join("\n\n")
                );
                messages.push(SubagentMessage::user(guidance));
            }

            // 3. Emit Turn Started
            self.event_emitter
                .emit(SubagentEvent::TurnStarted {
                    turn: self.current_iteration + 1,
                    max_turns: self.max_iters,
                })
                .await;

            // 4. Call LLM with Multi-Provider Failover
            let mut turn_resp = None;
            let mut last_error = None;
            let active_candidates = {
                let mut c = vec![model.clone()];
                c.extend(failover_models.clone());
                c
            };

            for candidate in active_candidates.iter().skip(failover_idx) {
                match llm
                    .complete_turn(candidate, &system_prompt, &messages, &tool_schemas)
                    .await
                {
                    Ok(resp) => {
                        turn_resp = Some(resp);
                        break;
                    }
                    Err(e) => {
                        last_error = Some(e);
                        failover_idx += 1;
                    }
                }
            }

            let resp = match turn_resp {
                Some(r) => r,
                None => {
                    let err_msg = last_error.unwrap_or_else(|| "LLM execution failed".to_string());
                    return self
                        .finalize_outcome(SubagentOutcome::Failed { error: err_msg })
                        .await;
                }
            };

            // 5. Accumulate text and record turn metrics
            if let Some(ref txt) = resp.content
                && !txt.is_empty()
            {
                if !last_text.is_empty() {
                    last_text.push_str("\n\n");
                }
                last_text.push_str(txt);
                self.event_emitter
                    .emit(SubagentEvent::OutputChunk { text: txt.clone() })
                    .await;
            }

            self.record_turn(resp.tokens_used, resp.tool_calls.len())
                .await;

            // 6. Check budget limits
            if let Some(reason) = self.is_budget_exhausted() {
                return self
                    .finalize_outcome(SubagentOutcome::Exhausted {
                        reason,
                        iterations: self.current_iteration,
                        tokens_used: self.cumulative_tokens as usize,
                    })
                    .await;
            }

            // 7. Check for canonical finish / finish_task tool calls
            if let Some(finish_tc) = resp
                .tool_calls
                .iter()
                .find(|tc| tc.name == FINISH_TOOL_NAME || tc.name == "finish_task")
            {
                let outcome = Self::check_finish_tool_call(&finish_tc.name, &finish_tc.arguments)
                    .unwrap_or_else(|| SubagentOutcome::Done {
                        summary: last_text.clone(),
                        iterations: self.current_iteration,
                        tool_calls_count: self.total_tool_calls,
                        token_usage: self.cumulative_tokens as usize,
                    });
                return self.finalize_outcome(outcome).await;
            }

            // 8. Natural completion (no tool calls and has text)
            if resp.tool_calls.is_empty() {
                let summary = if !last_text.is_empty() {
                    last_text
                } else {
                    "Task concluded without tool calls.".to_string()
                };
                return self
                    .finalize_outcome(SubagentOutcome::Done {
                        summary,
                        iterations: self.current_iteration,
                        tool_calls_count: self.total_tool_calls,
                        token_usage: self.cumulative_tokens as usize,
                    })
                    .await;
            }

            // 9. Execute tools
            let exec_path = self.execution_path(primary_path).to_path_buf();
            messages.push(SubagentMessage::assistant(
                resp.content.clone().unwrap_or_default(),
                Some(resp.tool_calls.clone()),
            ));

            for tc in &resp.tool_calls {
                self.event_emitter
                    .emit(SubagentEvent::ToolExecuting {
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                    })
                    .await;

                let output_res = async {
                    let policy = self
                        .tool_policy
                        .as_ref()
                        .ok_or("No subagent tool policy configured")?;
                    // Remote MCP backends may not supply mutation metadata. An
                    // unclassified capability is not automatically read-only.
                    let is_mcp_write = tools.is_mcp_write(&tc.name).await
                        || (tc.name.contains("__") && !readonly_mcp_name(&tc.name));
                    policy.permits_name(
                        &tc.name,
                        is_mcp_write,
                        &self.config.mode,
                        self.config.depth,
                    )?;
                    policy.permits_path(&tc.name, &tc.arguments, &exec_path)?;
                    if (is_write_schema(&tc.name) || is_mcp_write)
                        && targets_protected_path(&tc.arguments)
                    {
                        return Err("security: protected path access denied".into());
                    }
                    match policy
                        .permissions
                        .resolve(&tc.name, &tc.arguments, is_mcp_write)
                    {
                        Verdict::Deny(reason) => return Err(reason),
                        Verdict::Ask(reason) => {
                            let approval_id = format!("appr-{}", uuid::Uuid::new_v4());
                            self.event_emitter
                                .emit(SubagentEvent::ApprovalRequired {
                                    tool_name: tc.name.clone(),
                                    arguments: tc.arguments.clone(),
                                    approval_id: approval_id.clone(),
                                })
                                .await;
                            let response = self
                                .approval_channel
                                .request_approval(&approval_id, &tc.name, &tc.arguments)
                                .await;
                            let approved = response.as_ref().is_ok_and(|r| r.approved);
                            self.event_emitter
                                .emit(SubagentEvent::ApprovalResolved {
                                    approval_id,
                                    approved,
                                    feedback: response
                                        .as_ref()
                                        .ok()
                                        .and_then(|r| r.feedback.clone()),
                                })
                                .await;
                            if !approved {
                                return Err(response.err().unwrap_or(reason));
                            }
                        }
                        Verdict::Allow => {}
                    }
                    tools
                        .execute_tool(&tc.id, &tc.name, &tc.arguments, &exec_path)
                        .await
                }
                .await;

                let is_error = output_res.is_err();
                let output_text = match output_res {
                    Ok(out) => out,
                    Err(err) => format!("Tool error: {err}"),
                };

                self.event_emitter
                    .emit(SubagentEvent::ToolCompleted {
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        is_error,
                    })
                    .await;

                messages.push(SubagentMessage::tool_result(&tc.id, output_text));
            }
        }

        // Final iteration limit reached: check if last_text provides a valid answer
        let final_outcome = if !last_text.is_empty() {
            SubagentOutcome::Done {
                summary: last_text,
                iterations: self.current_iteration,
                tool_calls_count: self.total_tool_calls,
                token_usage: self.cumulative_tokens as usize,
            }
        } else {
            SubagentOutcome::Exhausted {
                reason: format!(
                    "Iteration limit of {} reached without converging",
                    self.max_iters
                ),
                iterations: self.current_iteration,
                tokens_used: self.cumulative_tokens as usize,
            }
        };

        self.finalize_outcome(final_outcome).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cade_core::permissions::PermissionMode;
    use tempfile::tempdir;

    #[test]
    fn test_finish_tool_schema_structure() {
        let schema = canonical_finish_tool_schema();
        assert_eq!(schema["name"], FINISH_TOOL_NAME);
        assert_eq!(schema["parameters"]["type"], "object");
        assert!(schema["parameters"]["properties"]["status"].is_object());
    }

    #[tokio::test]
    async fn invoking_conversation_context_is_bounded_and_separate() {
        struct CapturingLlm(std::sync::Mutex<Vec<String>>);
        #[async_trait]
        impl SubagentLlmExecutor for CapturingLlm {
            async fn complete_turn(
                &self,
                _model: &str,
                system_prompt: &str,
                _messages: &[SubagentMessage],
                _tools: &[Value],
            ) -> Result<SubagentTurnResponse, String> {
                self.0.lock().unwrap().push(system_prompt.to_owned());
                Ok(SubagentTurnResponse {
                    content: Some("finished".into()),
                    tool_calls: vec![],
                    tokens_used: 1,
                })
            }
        }
        struct NoTools;
        #[async_trait]
        impl SubagentToolExecutor for NoTools {
            async fn execute_tool(
                &self,
                _: &str,
                _: &str,
                _: &Value,
                _: &Path,
            ) -> Result<String, String> {
                panic!("no tool calls expected")
            }
        }

        let llm = CapturingLlm(std::sync::Mutex::new(Vec::new()));
        for (conv, own, other) in [("first", "alpha", "beta"), ("second", "beta", "alpha")] {
            let messages = (0..10)
                .map(|i| {
                    SubagentMessage::user(format!("{own}-{i}\nline2\nline3\nline4\nline5\nhidden"))
                })
                .collect();
            let config = SubagentConfig::from_args(&json!({"prompt": "task"}));
            let mut session = SubagentSession::new(config, "same-agent")
                .with_parent_conversation_id(Some(conv.into()))
                .with_parent_context(messages);
            assert_eq!(session.parent_conversation_id.as_deref(), Some(conv));
            let outcome = session
                .run_autonomous_loop(
                    &llm,
                    &NoTools,
                    "test".into(),
                    "system".into(),
                    "task".into(),
                    vec![],
                    vec![],
                    Path::new("."),
                )
                .await;
            assert!(outcome.is_success());
            let prompts = llm.0.lock().unwrap();
            let prompt = prompts.last().unwrap();
            assert!(prompt.contains(&format!("{own}-2")));
            assert!(prompt.contains(&format!("{own}-9")));
            assert!(!prompt.contains(&format!("{own}-1")));
            assert!(!prompt.contains(other));
            assert!(!prompt.contains("hidden"));
        }
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

    #[tokio::test]
    async fn test_subagent_budget_permits_large_initial_prompt() {
        // Budget of 5,000 generation tokens
        let config = SubagentConfig::from_args(&json!({
            "prompt": "Large prompt...",
            "max_tokens_budget": 5000
        }));
        let mut session = SubagentSession::new(config, "parent-1").with_max_iters(10);

        // Before any generations, budget is not exhausted regardless of prompt size
        assert!(session.is_budget_exhausted().is_none());

        // First turn produces 1,500 generated tokens
        session.record_turn(1500, 2).await;
        assert!(session.is_budget_exhausted().is_none());

        // Second turn produces 2,000 generated tokens (total 3,500 < 5,000)
        session.record_turn(2000, 1).await;
        assert!(session.is_budget_exhausted().is_none());

        // Third turn produces 2,000 generated tokens (total 5,500 > 5,000)
        session.record_turn(2000, 1).await;
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
        session.record_finding(
            "api_convention",
            "REST with JSON",
            "Discovered in repo",
            "convention",
            0.95,
        );

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
        emitter
            .emit(SubagentEvent::Thought {
                text: "Analyzing code...".to_string(),
            })
            .await;

        let e1 = brx1.recv().await.expect("Subscriber 1 received event");
        let e2 = brx2.recv().await.expect("Subscriber 2 received event");

        assert_eq!(
            e1,
            SubagentEvent::Thought {
                text: "Analyzing code...".to_string()
            }
        );
        assert_eq!(e2, e1);
    }

    #[tokio::test]
    async fn test_subagent_approval_channel_flow() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let channel = SubagentApprovalChannel::new(tx);

        let approval_task = tokio::spawn(async move {
            channel
                .request_approval("appr-1", "write_file", &json!({ "path": "src/main.rs" }))
                .await
        });

        let (appr_id, tool_name, args, responder) =
            rx.recv().await.expect("Received approval request");
        assert_eq!(appr_id, "appr-1");
        assert_eq!(tool_name, "write_file");
        assert_eq!(args["path"], "src/main.rs");

        responder
            .send(SubagentApprovalResponse {
                approved: true,
                feedback: Some("Approved with caution".to_string()),
            })
            .expect("Sent approval");

        let verdict = approval_task
            .await
            .expect("Task completed")
            .expect("Approval succeeded");
        assert!(verdict.approved);
        assert_eq!(verdict.feedback.as_deref(), Some("Approved with caution"));
    }

    struct MockLlm {
        turns: std::sync::Mutex<Vec<SubagentTurnResponse>>,
        observed_models: std::sync::Mutex<Vec<String>>,
        observed_messages: std::sync::Mutex<Vec<Vec<SubagentMessage>>>,
    }

    #[async_trait]
    impl SubagentLlmExecutor for MockLlm {
        async fn complete_turn(
            &self,
            model: &str,
            _system_prompt: &str,
            messages: &[SubagentMessage],
            _tools: &[Value],
        ) -> Result<SubagentTurnResponse, String> {
            self.observed_models.lock().unwrap().push(model.to_string());
            self.observed_messages
                .lock()
                .unwrap()
                .push(messages.to_vec());
            let mut turns = self.turns.lock().unwrap();
            if turns.is_empty() {
                Ok(SubagentTurnResponse {
                    content: Some("Default completion".to_string()),
                    tool_calls: Vec::new(),
                    tokens_used: 10,
                })
            } else {
                Ok(turns.remove(0))
            }
        }
    }

    struct MockToolExecutor;

    #[async_trait]
    impl SubagentToolExecutor for MockToolExecutor {
        async fn execute_tool(
            &self,
            _tool_call_id: &str,
            tool_name: &str,
            _arguments: &Value,
            _execution_path: &Path,
        ) -> Result<String, String> {
            Ok(format!("Output from {tool_name}"))
        }
    }

    struct RecordingTools(std::sync::Mutex<Vec<String>>);

    #[async_trait]
    impl SubagentToolExecutor for RecordingTools {
        async fn is_mcp_write(&self, name: &str) -> bool {
            name == "mcp__set_secret"
        }

        async fn execute_tool(
            &self,
            _: &str,
            name: &str,
            _: &Value,
            _: &Path,
        ) -> Result<String, String> {
            self.0.lock().unwrap().push(name.to_owned());
            Ok("executed".into())
        }
    }

    fn scripted_calls(calls: Vec<(&str, Value)>) -> MockLlm {
        MockLlm {
            turns: std::sync::Mutex::new(vec![SubagentTurnResponse {
                content: None,
                tool_calls: calls
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, arguments))| SubagentToolCall {
                        id: format!("call-{i}"),
                        name: name.into(),
                        arguments,
                    })
                    .collect(),
                tokens_used: 1,
            }]),
            observed_models: std::sync::Mutex::new(vec![]),
            observed_messages: std::sync::Mutex::new(vec![]),
        }
    }

    fn policy(mode: &str) -> SubagentToolPolicy {
        SubagentToolPolicy {
            permissions: PermissionManager::new(if mode == "plan" {
                PermissionMode::Plan
            } else {
                PermissionMode::Default
            }),
            tools: SubagentTools::All,
            inherited_tools: [
                "read_file",
                "write_file",
                "mcp__list_items",
                "mcp__set_secret",
                "run_subagent",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            allow_nesting: false,
            max_depth: 3,
        }
    }

    async fn exercise(session: &mut SubagentSession, llm: &MockLlm, tools: &RecordingTools) {
        session
            .run_autonomous_loop(
                llm,
                tools,
                "model".into(),
                "system".into(),
                "task".into(),
                vec![json!({"name": "read_file"})],
                vec![],
                Path::new("."),
            )
            .await;
    }

    #[tokio::test]
    async fn execution_policy_checks_real_calls_not_visible_schemas() {
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_tool_policy(policy("build"));
        session
            .tool_policy
            .as_ref()
            .unwrap()
            .permissions
            .add_deny_rule(cade_core::permissions::PermissionRule::parse("read_file").unwrap());
        let calls = scripted_calls(vec![
            ("read_file", json!({"path":"src/lib.rs"})),
            ("mcp__list_items", json!({})),
            ("write_file", json!({"path":".env", "content":"secret"})),
            ("unknown_tool", json!({})),
            ("run_subagent", json!({"prompt":"nested"})),
        ]);
        let tools = RecordingTools(std::sync::Mutex::new(vec![]));
        exercise(&mut session, &calls, &tools).await;
        assert_eq!(*tools.0.lock().unwrap(), vec!["mcp__list_items"]);
        let seen = calls.observed_messages.lock().unwrap();
        let results = &seen[1];
        assert!(
            results
                .iter()
                .any(|m| m.content.contains("blocked by deny rule"))
        );
        assert!(results.iter().any(|m| m.content.contains("protected path")));
        assert!(results.iter().any(|m| m.content.contains("not inherited")));
        assert!(
            results
                .iter()
                .any(|m| m.content.contains("Nested subagent"))
        );
    }

    #[tokio::test]
    async fn ask_waits_for_approval_and_fails_closed_without_adapter() {
        let calls = || scripted_calls(vec![("write_file", json!({"path":"src/lib.rs"}))]);
        let tools = RecordingTools(std::sync::Mutex::new(vec![]));
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task", "human_review":true})),
            "parent",
        )
        .with_tool_policy(policy("build"));
        exercise(&mut session, &calls(), &tools).await;
        assert!(
            tools.0.lock().unwrap().is_empty(),
            "post-run review cannot approve an execution"
        );

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_tool_policy(policy("build"))
        .with_approval_channel(SubagentApprovalChannel::new(tx));
        let llm = calls();
        let execution = async {
            exercise(&mut session, &llm, &tools).await;
        };
        let responder = async {
            let (_, name, _, reply) = rx.recv().await.unwrap();
            assert_eq!(name, "write_file");
            assert!(
                tools.0.lock().unwrap().is_empty(),
                "execution must wait for user verdict"
            );
            reply
                .send(SubagentApprovalResponse {
                    approved: true,
                    feedback: None,
                })
                .unwrap();
        };
        tokio::join!(execution, responder);
        assert_eq!(*tools.0.lock().unwrap(), vec!["write_file"]);

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_tool_policy(policy("build"))
        .with_approval_channel(SubagentApprovalChannel::new(tx));
        let denied_calls = calls();
        tokio::join!(exercise(&mut session, &denied_calls, &tools), async {
            let (_, _, _, reply) = rx.recv().await.unwrap();
            reply
                .send(SubagentApprovalResponse {
                    approved: false,
                    feedback: None,
                })
                .unwrap();
        });
        assert_eq!(*tools.0.lock().unwrap(), vec!["write_file"]);
    }

    #[tokio::test]
    async fn plan_and_mcp_metadata_block_mutations_but_allow_inherited_reads() {
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task", "mode":"plan"})),
            "parent",
        )
        .with_tool_policy(policy("plan"));
        let llm = scripted_calls(vec![
            ("mcp__list_items", json!({})),
            ("mcp__set_secret", json!({"path":"src/lib.rs"})),
            ("write_file", json!({"path":"src/lib.rs"})),
        ]);
        let tools = RecordingTools(std::sync::Mutex::new(vec![]));
        exercise(&mut session, &llm, &tools).await;
        assert_eq!(*tools.0.lock().unwrap(), vec!["mcp__list_items"]);
    }

    #[tokio::test]
    async fn restricted_paths_and_protected_nested_mcp_arguments_are_checked_before_execution() {
        let root = tempdir().unwrap();
        std::fs::create_dir(root.path().join("allowed")).unwrap();
        std::fs::create_dir(root.path().join("other")).unwrap();
        let mut policy = policy("build");
        policy.tools = SubagentTools::Restricted {
            allowed_tools: vec!["read_file".into(), "mcp__set_secret".into()],
            allowed_paths: vec![root.path().join("allowed").to_string_lossy().to_string()],
        };
        policy
            .permissions
            .set_mode(PermissionMode::BypassPermissions);
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_tool_policy(policy);
        let llm = scripted_calls(vec![
            (
                "read_file",
                json!({"path":root.path().join("other/secret")}),
            ),
            ("read_file", json!({"path":root.path().join("allowed/ok")})),
            ("mcp__set_secret", json!({"params":{"path":".env"}})),
        ]);
        let tools = RecordingTools(std::sync::Mutex::new(vec![]));
        exercise(&mut session, &llm, &tools).await;
        assert_eq!(*tools.0.lock().unwrap(), vec!["read_file"]);
    }

    #[tokio::test]
    async fn permitted_mcp_write_and_nesting_depth_follow_effective_policy() {
        let mut access = policy("build");
        access
            .permissions
            .set_mode(PermissionMode::BypassPermissions);
        access.allow_nesting = true;
        access.max_depth = 2;
        assert!(
            access
                .permits_name("run_subagent", false, "build", 0)
                .is_ok()
        );
        assert!(
            access
                .permits_name("run_subagent", false, "build", 1)
                .is_err()
        );
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_tool_policy(access);
        let tools = RecordingTools(std::sync::Mutex::new(vec![]));
        let calls = scripted_calls(vec![("mcp__set_secret", json!({"value":"ordinary"}))]);
        exercise(&mut session, &calls, &tools).await;
        assert_eq!(*tools.0.lock().unwrap(), vec!["mcp__set_secret"]);
    }

    struct WritingTool;

    #[async_trait]
    impl SubagentToolExecutor for WritingTool {
        async fn execute_tool(
            &self,
            _id: &str,
            _name: &str,
            _arguments: &Value,
            execution_path: &Path,
        ) -> Result<String, String> {
            std::fs::write(execution_path.join("code.rs"), "child wrote here")
                .map_err(|e| e.to_string())?;
            Ok("written".into())
        }
    }

    fn writing_llm(status: &str) -> MockLlm {
        MockLlm {
            turns: std::sync::Mutex::new(vec![
                SubagentTurnResponse {
                    content: None,
                    tool_calls: vec![SubagentToolCall {
                        id: "write".into(),
                        name: "write_file".into(),
                        arguments: json!({}),
                    }],
                    tokens_used: 1,
                },
                SubagentTurnResponse {
                    content: None,
                    tool_calls: vec![SubagentToolCall {
                        id: "finish".into(),
                        name: "finish".into(),
                        arguments: json!({ "status": status, "summary": status }),
                    }],
                    tokens_used: 1,
                },
            ]),
            observed_models: std::sync::Mutex::new(Vec::new()),
            observed_messages: std::sync::Mutex::new(Vec::new()),
        }
    }

    async fn run_writing_session(
        session: &mut SubagentSession,
        llm: &MockLlm,
        primary: &Path,
    ) -> SubagentOutcome {
        session
            .run_autonomous_loop(
                llm,
                &WritingTool,
                "test-model".into(),
                "system".into(),
                "write".into(),
                vec![],
                vec![],
                primary,
            )
            .await
    }

    #[tokio::test]
    async fn required_isolation_setup_failure_never_runs_child_in_live_workspace() {
        let live = tempdir().unwrap();
        let file = live.path().join("code.rs");
        std::fs::write(&file, "original").unwrap();
        let config = SubagentConfig::from_args(&json!({
            "prompt": "write", "enforce_isolation": true
        }));
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut session = SubagentSession::new(config, "parent")
            .with_event_emitter(SubagentEventEmitter::new(Some(tx)));
        let failure = session
            .prepare_workspace(&live.path().join("missing"), None)
            .await;
        assert!(
            failure.is_err(),
            "a missing workspace must not clone as empty"
        );

        let llm = writing_llm("done");
        let outcome = run_writing_session(&mut session, &llm, live.path()).await;
        assert!(matches!(outcome, SubagentOutcome::Failed { .. }));
        assert!(outcome.summary_text().contains("isolation"));
        assert!(matches!(
            rx.recv().await,
            Some(SubagentEvent::Finished { .. })
        ));
        assert!(llm.observed_models.lock().unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
    }

    #[tokio::test]
    async fn isolated_execution_merges_only_success_and_cleans_up_on_failure() {
        for status in ["done", "error"] {
            let live = tempdir().unwrap();
            let file = live.path().join("code.rs");
            std::fs::write(&file, "original").unwrap();
            let config = SubagentConfig::from_args(&json!({
                "prompt": "write", "enforce_isolation": true
            }));
            let mut session = SubagentSession::new(config, "parent");
            session.prepare_workspace(live.path(), None).await.unwrap();
            let temp_path = session.execution_path(live.path()).to_path_buf();
            let outcome =
                run_writing_session(&mut session, &writing_llm(status), live.path()).await;
            assert_eq!(outcome.is_success(), status == "done");
            assert_eq!(
                std::fs::read_to_string(file).unwrap(),
                if status == "done" {
                    "child wrote here"
                } else {
                    "original"
                }
            );
            assert!(
                !temp_path.exists(),
                "workspace must be released at completion"
            );
        }
    }

    #[tokio::test]
    async fn optional_nonisolated_execution_remains_usable() {
        let live = tempdir().unwrap();
        let config = SubagentConfig::from_args(&json!({ "prompt": "write" }));
        let mut session = SubagentSession::new(config, "parent");
        let outcome = run_writing_session(&mut session, &writing_llm("done"), live.path()).await;
        assert!(outcome.is_success());
        assert_eq!(
            std::fs::read_to_string(live.path().join("code.rs")).unwrap(),
            "child wrote here"
        );
    }

    struct BlockingWriter(tokio::sync::mpsc::Sender<std::path::PathBuf>);

    #[async_trait]
    impl SubagentToolExecutor for BlockingWriter {
        async fn execute_tool(
            &self,
            _id: &str,
            _name: &str,
            _arguments: &Value,
            execution_path: &Path,
        ) -> Result<String, String> {
            std::fs::write(execution_path.join("code.rs"), "interrupted").unwrap();
            self.0.send(execution_path.to_path_buf()).await.unwrap();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn interrupted_isolated_run_discards_workspace_without_merging() {
        let live = tempdir().unwrap();
        let file = live.path().join("code.rs");
        std::fs::write(&file, "original").unwrap();
        let config = SubagentConfig::from_args(&json!({
            "prompt": "write", "enforce_isolation": true
        }));
        let mut session = SubagentSession::new(config, "parent");
        session.prepare_workspace(live.path(), None).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let live_path = live.path().to_path_buf();
        let task = tokio::spawn(async move {
            session
                .run_autonomous_loop(
                    &writing_llm("done"),
                    &BlockingWriter(tx),
                    "model".into(),
                    "system".into(),
                    "write".into(),
                    vec![],
                    vec![],
                    &live_path,
                )
                .await
        });
        let temp_path = rx.recv().await.unwrap();
        assert!(temp_path.join("code.rs").exists());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!temp_path.exists());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
    }

    #[tokio::test]
    async fn timed_out_isolated_run_discards_workspace_without_merging() {
        let live = tempdir().unwrap();
        let file = live.path().join("code.rs");
        std::fs::write(&file, "original").unwrap();
        let config = SubagentConfig::from_args(&json!({
            "prompt": "write", "enforce_isolation": true
        }));
        let mut session = SubagentSession::new(config, "parent");
        session.prepare_workspace(live.path(), None).await.unwrap();
        let temp_path = session.execution_path(live.path()).to_path_buf();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let llm = writing_llm("done");
        let tools = BlockingWriter(tx);
        let mut loop_future = Box::pin(session.run_autonomous_loop(
            &llm,
            &tools,
            "model".into(),
            "system".into(),
            "write".into(),
            vec![],
            vec![],
            live.path(),
        ));
        tokio::select! {
            _ = &mut loop_future => panic!("blocked tool cannot finish"),
            path = rx.recv() => assert_eq!(path.unwrap(), temp_path),
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), &mut loop_future)
                .await
                .is_err()
        );
        drop(loop_future);
        drop(session);
        assert!(!temp_path.exists());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "original");
    }

    #[tokio::test]
    async fn test_autonomous_loop_natural_completion() {
        let llm = MockLlm {
            turns: std::sync::Mutex::new(vec![SubagentTurnResponse {
                content: Some("Task finished without tool calls".to_string()),
                tool_calls: Vec::new(),
                tokens_used: 25,
            }]),
            observed_models: std::sync::Mutex::new(Vec::new()),
            observed_messages: std::sync::Mutex::new(Vec::new()),
        };
        let tools = MockToolExecutor;
        let config = SubagentConfig::from_args(&json!({ "prompt": "Solve problem" }));
        let mut session = SubagentSession::new(config, "parent");

        let outcome = session
            .run_autonomous_loop(
                &llm,
                &tools,
                "model-primary".to_string(),
                "System prompt".to_string(),
                "Initial prompt".to_string(),
                vec![],
                vec![],
                Path::new("."),
            )
            .await;

        assert!(outcome.is_success());
        assert_eq!(outcome.summary_text(), "Task finished without tool calls");
    }

    #[tokio::test]
    async fn test_autonomous_loop_finish_tool_calls() {
        // Test standard 'finish' tool call
        let llm1 = MockLlm {
            turns: std::sync::Mutex::new(vec![SubagentTurnResponse {
                content: None,
                tool_calls: vec![SubagentToolCall {
                    id: "call_1".to_string(),
                    name: "finish".to_string(),
                    arguments: json!({ "status": "done", "summary": "Finished successfully via finish" }),
                }],
                tokens_used: 15,
            }]),
            observed_models: std::sync::Mutex::new(Vec::new()),
            observed_messages: std::sync::Mutex::new(Vec::new()),
        };
        let tools = MockToolExecutor;
        let config1 = SubagentConfig::from_args(&json!({ "prompt": "Do task 1" }));
        let mut session1 = SubagentSession::new(config1, "parent");

        let outcome1 = session1
            .run_autonomous_loop(
                &llm1,
                &tools,
                "model-1".to_string(),
                "Sys".to_string(),
                "Init".to_string(),
                vec![],
                vec![],
                Path::new("."),
            )
            .await;
        assert_eq!(outcome1.summary_text(), "Finished successfully via finish");

        // Test 'finish_task' tool call
        let llm2 = MockLlm {
            turns: std::sync::Mutex::new(vec![SubagentTurnResponse {
                content: None,
                tool_calls: vec![SubagentToolCall {
                    id: "call_2".to_string(),
                    name: "finish_task".to_string(),
                    arguments: json!({ "status": "done", "summary": "Finished successfully via finish_task" }),
                }],
                tokens_used: 15,
            }]),
            observed_models: std::sync::Mutex::new(Vec::new()),
            observed_messages: std::sync::Mutex::new(Vec::new()),
        };
        let config2 = SubagentConfig::from_args(&json!({ "prompt": "Do task 2" }));
        let mut session2 = SubagentSession::new(config2, "parent");

        let outcome2 = session2
            .run_autonomous_loop(
                &llm2,
                &tools,
                "model-1".to_string(),
                "Sys".to_string(),
                "Init".to_string(),
                vec![],
                vec![],
                Path::new("."),
            )
            .await;
        assert_eq!(
            outcome2.summary_text(),
            "Finished successfully via finish_task"
        );
    }

    #[tokio::test]
    async fn test_autonomous_loop_steering_and_model_hot_swap() {
        let llm = MockLlm {
            turns: std::sync::Mutex::new(vec![
                SubagentTurnResponse {
                    content: Some("First thought".to_string()),
                    tool_calls: vec![SubagentToolCall {
                        id: "call_read".to_string(),
                        name: "read_file".to_string(),
                        arguments: json!({ "path": "test.txt" }),
                    }],
                    tokens_used: 20,
                },
                SubagentTurnResponse {
                    content: None,
                    tool_calls: vec![SubagentToolCall {
                        id: "call_finish".to_string(),
                        name: "finish".to_string(),
                        arguments: json!({ "status": "done", "summary": "Steering incorporated and completed" }),
                    }],
                    tokens_used: 30,
                },
            ]),
            observed_models: std::sync::Mutex::new(Vec::new()),
            observed_messages: std::sync::Mutex::new(Vec::new()),
        };
        let tools = MockToolExecutor;
        let config = SubagentConfig::from_args(&json!({ "prompt": "Initial problem" }));
        let mut session = SubagentSession::new(config, "parent");

        // Inject steering and model hot-swap before turn 1
        session.steer("Focus strictly on test assertion".to_string());
        session.hot_swap_model("hot-swapped-model-v2".to_string());

        let outcome = session
            .run_autonomous_loop(
                &llm,
                &tools,
                "initial-model".to_string(),
                "Sys".to_string(),
                "Initial problem".to_string(),
                vec![],
                vec![],
                Path::new("."),
            )
            .await;

        assert_eq!(
            outcome.summary_text(),
            "Steering incorporated and completed"
        );

        // Verify that model was hot-swapped
        let models = llm.observed_models.lock().unwrap();
        assert_eq!(models[0], "hot-swapped-model-v2");

        // Verify that steering guidance was injected with priority envelope
        let messages = llm.observed_messages.lock().unwrap();
        let turn_0_msgs = &messages[0];
        assert!(
            turn_0_msgs
                .iter()
                .any(|m| m.content.contains("[Supervisor Steering Guidance]"))
        );
        assert!(
            turn_0_msgs
                .iter()
                .any(|m| m.content.contains("Focus strictly on test assertion"))
        );
    }
}
