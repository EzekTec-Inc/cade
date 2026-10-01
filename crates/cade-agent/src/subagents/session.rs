//! Autonomous SubagentSession Execution Harness (ADR-0021 / Issues #49, #50, #51).
//!
//! Encapsulates the execution loop, canonical finish tool injection,
//! dual budget enforcement (max_iters & max_tokens_budget), RAII workspace isolation,
//! real-time telemetry streaming, and structured outcome models.

use async_trait::async_trait;
use cade_core::permissions::{
    PermissionManager, PermissionService, Verdict, is_write_schema, path_is_protected,
};
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

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
    /// Rebuild provider-facing instructions and capabilities for each candidate
    /// (including failover). The session retains the original policy and history.
    fn prepare_turn(&self, _model: &str, prompt: &str, tools: &[Value]) -> (String, Vec<Value>) {
        (prompt.to_string(), tools.to_vec())
    }

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

    /// Execute in `execution_path`: native relative file paths, filesystem root
    /// validation and subprocess cwd must all use this request-local directory.
    /// An adapter that cannot honor it must return an error. Process-wide cwd
    /// and environment mutation cannot safely isolate concurrent children.
    async fn execute_tool(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &Value,
        execution_path: &Path,
    ) -> Result<String, String>;
}

/// Host-owned ephemeral resources transferred to the session for teardown.
/// Successful writeback and failure discard run before the terminal outcome is
/// published. Implementations must also support synchronous discard on drop.
#[async_trait]
pub trait SubagentCleanup: Send + Sync {
    /// Finalize resources; report writeback and deletion failures to the child.
    async fn finalize(&mut self, success: bool) -> Result<(), String>;
    /// Discard resources after abrupt owner drop. Must be idempotent.
    fn discard(&mut self) -> Result<(), String>;
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
    PauseStateChanged {
        state: SubagentPauseState,
    },
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
    SteeringApplied {
        messages: usize,
    },
    Finished {
        outcome: SubagentOutcome,
    },
}

/// The observable state of the in-process pause gate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentPauseState {
    Queued,
    Running,
    PauseRequested,
    Paused,
    Finished,
}

impl SubagentPauseState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::PauseRequested => "pause_requested",
            Self::Paused => "paused",
            Self::Finished => "finished",
        }
    }
}

/// Shared control for a single live session. Requests never acknowledge a
/// boundary transition until the child actually reaches the boundary.
#[derive(Clone)]
pub struct SubagentPause {
    tx: tokio::sync::watch::Sender<SubagentPauseState>,
}

impl SubagentPause {
    pub fn new() -> Self {
        let (tx, _) = tokio::sync::watch::channel(SubagentPauseState::Queued);
        Self { tx }
    }

    pub fn state(&self) -> SubagentPauseState {
        *self.tx.borrow()
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<SubagentPauseState> {
        self.tx.subscribe()
    }

    pub fn pause(&self) -> Result<SubagentPauseState, &'static str> {
        let mut result = Err("subagent is not running");
        self.tx.send_modify(|state| {
            if *state == SubagentPauseState::Running {
                *state = SubagentPauseState::PauseRequested;
                result = Ok(*state);
            }
        });
        result
    }

    pub fn resume(&self) -> Result<SubagentPauseState, &'static str> {
        let mut result = Err("subagent is not paused");
        self.tx.send_modify(|state| {
            if matches!(
                state,
                SubagentPauseState::Paused | SubagentPauseState::PauseRequested
            ) {
                *state = SubagentPauseState::Running;
                result = Ok(*state);
            }
        });
        result
    }

    fn start(&self) {
        self.tx.send_modify(|state| {
            if *state == SubagentPauseState::Queued {
                *state = SubagentPauseState::Running;
            }
        });
    }

    pub fn finish(&self) {
        self.tx.send_replace(SubagentPauseState::Finished);
    }

    async fn boundary(&self, emitter: &SubagentEventEmitter) {
        let mut rx = self.tx.subscribe();
        loop {
            let state = *rx.borrow_and_update();
            if state == SubagentPauseState::PauseRequested {
                let mut paused = false;
                self.tx.send_modify(|current| {
                    if *current == SubagentPauseState::PauseRequested {
                        *current = SubagentPauseState::Paused;
                        paused = true;
                    }
                });
                if paused {
                    emitter
                        .emit(SubagentEvent::PauseStateChanged {
                            state: SubagentPauseState::Paused,
                        })
                        .await;
                }
                continue;
            }
            if state != SubagentPauseState::Paused {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Default for SubagentPause {
    fn default() -> Self {
        Self::new()
    }
}

/// Asynchronous event broadcaster for subagents supporting unicast & broadcast subscribers.
#[derive(Clone)]
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

    // Terminal state is stored before publication. Publishing must not be
    // cancelled halfway through finalization, nor block workspace teardown.
    fn emit_terminal(&self, outcome: SubagentOutcome) {
        let event = SubagentEvent::Finished { outcome };
        if let Some(tx) = &self.broadcast_tx {
            let _ = tx.send(event.clone());
        }
        if let Some(tx) = &self.tx {
            match tx.try_send(event) {
                Err(tokio::sync::mpsc::error::TrySendError::Full(event)) => {
                    if let Ok(handle) = tokio::runtime::Handle::try_current() {
                        let tx = tx.clone();
                        handle.spawn(async move {
                            let _ = tx.send(event).await;
                        });
                    }
                }
                Ok(()) | Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {}
            }
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
    /// The child definition's model-visible tool set. Execution additionally
    /// checks inherited capabilities, mode, MCP metadata and permission rules.
    pub fn definition_allows(tools: &SubagentTools, name: &str) -> bool {
        match tools {
            SubagentTools::All => true,
            SubagentTools::Readonly => {
                matches!(
                    name,
                    "read_file"
                        | "glob"
                        | "grep"
                        | "search_memory"
                        | "conversation_search"
                        | "archival_memory_search"
                        | "recall"
                        | "fetch_doc"
                ) || (name.contains("__") && readonly_mcp_name(name))
            }
            SubagentTools::List(names) => names.iter().any(|n| n == name),
            SubagentTools::Restricted { allowed_tools, .. } => {
                allowed_tools.iter().any(|n| n == name)
            }
        }
    }

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
        if !Self::definition_allows(&self.tools, name) {
            return Err(format!("Tool '{name}' is not allowed by child definition"));
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
    workspace_guard: Option<IsolatedWorkspaceGuard>,
    pub event_emitter: SubagentEventEmitter,
    pub findings: Vec<SubagentFinding>,
    pub approval_channel: SubagentApprovalChannel,
    permission_service: Option<Arc<dyn PermissionService>>,
    pub tool_policy: Option<SubagentToolPolicy>,
    pub steering_queue: Vec<String>,
    pub pending_model_swap: Option<String>,
    /// Legacy control handle; hosts should observe `subscribe_pause` and use the
    /// identified-child control methods rather than mutate terminal state.
    pub pause: SubagentPause,
    model_control: Option<SubagentModelControl>,
    parent_context: Vec<SubagentMessage>,
    control: Option<SubagentControl>,
    completion: SubagentCompletion,
    cleanup: Vec<Box<dyn SubagentCleanup>>,
    workspace_applied: bool,
}

/// In-process lifecycle and next-turn guidance for an identified child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentStatus {
    Queued,
    Running,
    PauseRequested,
    Paused,
    Finished { outcome: String },
}

impl std::fmt::Display for SubagentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Running => write!(f, "running"),
            Self::PauseRequested => write!(f, "pause requested"),
            Self::Paused => write!(f, "paused"),
            Self::Finished { outcome } => write!(f, "finished ({outcome})"),
        }
    }
}

#[derive(Clone)]
pub struct SubagentControl(Arc<Mutex<ControlState>>);

struct ControlState {
    status: SubagentStatus,
    guidance: Vec<String>,
    pause: SubagentPause,
    model: SubagentModelControl,
    completion: SubagentCompletion,
}

fn controls() -> &'static Mutex<HashMap<String, SubagentControl>> {
    static CONTROLS: OnceLock<Mutex<HashMap<String, SubagentControl>>> = OnceLock::new();
    CONTROLS.get_or_init(|| Mutex::new(HashMap::new()))
}

impl SubagentControl {
    pub fn status(&self) -> SubagentStatus {
        let state = self.0.lock().unwrap();
        if matches!(state.status, SubagentStatus::Finished { .. }) {
            return state.status.clone();
        }
        match state.pause.state() {
            SubagentPauseState::PauseRequested => SubagentStatus::PauseRequested,
            SubagentPauseState::Paused => SubagentStatus::Paused,
            _ => state.status.clone(),
        }
    }

    pub fn running(&self) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !state.completion.is_finishing()
            && !matches!(state.status, SubagentStatus::Finished { .. })
        {
            state.status = SubagentStatus::Running;
        }
    }

    pub fn finished(&self, outcome: impl Into<String>) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(state.status, SubagentStatus::Finished { .. }) {
            return;
        }
        state.status = SubagentStatus::Finished {
            outcome: outcome.into(),
        };
        state.guidance.clear();
        state.model.close();
    }

    pub fn pause_state(&self) -> SubagentPauseState {
        self.0.lock().unwrap().pause.state()
    }

    pub fn set_paused(&self, resume: bool) -> Result<SubagentPauseState, String> {
        let state = self.0.lock().unwrap();
        if state.completion.is_finishing() {
            return Err("subagent is finalizing or finished".into());
        }
        (if resume {
            state.pause.resume()
        } else {
            state.pause.pause()
        })
        .map_err(str::to_string)
    }

    pub fn request_model(&self, model: String) -> Result<(), String> {
        let state = self.0.lock().unwrap();
        if state.completion.is_finishing()
            || matches!(state.status, SubagentStatus::Finished { .. })
        {
            return Err("subagent is no longer accepting model changes".into());
        }
        state.model.request(model).map_err(str::to_string)
    }

    pub fn steer(&self, message: String) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        if state.completion.is_finishing() {
            return Err("subagent is finalizing or finished".into());
        }
        let status = match state.pause.state() {
            SubagentPauseState::PauseRequested => SubagentStatus::PauseRequested,
            SubagentPauseState::Paused => SubagentStatus::Paused,
            _ => state.status.clone(),
        };
        if !matches!(
            status,
            SubagentStatus::Running | SubagentStatus::PauseRequested | SubagentStatus::Paused
        ) {
            return Err(format!("Cannot steer subagent in {status} state"));
        }
        if message.trim().is_empty() {
            return Err("Steering guidance must not be empty".into());
        }
        state.guidance.push(message);
        Ok(())
    }

    fn take_guidance(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap().guidance)
    }
}

impl SubagentSession {
    /// Register the stable child ID when the invocation is admitted.
    pub fn register_control(&mut self, queued: bool) -> SubagentControl {
        if let Some(control) = &self.control {
            return control.clone();
        }
        let model = self
            .model_control
            .get_or_insert_with(SubagentModelControl::new)
            .clone();
        let control = SubagentControl(Arc::new(Mutex::new(ControlState {
            status: if queued {
                SubagentStatus::Queued
            } else {
                SubagentStatus::Running
            },
            guidance: Vec::new(),
            pause: self.pause.clone(),
            model,
            completion: self.completion(),
        })));
        controls()
            .lock()
            .unwrap()
            .insert(self.session_id.clone(), control.clone());
        self.control = Some(control.clone());
        control
    }

    pub fn child_status(id: &str) -> Result<SubagentStatus, String> {
        controls()
            .lock()
            .unwrap()
            .get(id)
            .map(SubagentControl::status)
            .ok_or_else(|| {
                format!("Subagent '{id}' not found in this process; check the child ID or its host")
            })
    }

    pub fn steer_child(id: &str, message: String) -> Result<(), String> {
        let control = controls().lock().unwrap().get(id).cloned().ok_or_else(|| {
            format!("Subagent '{id}' not found in this process; check the child ID or its host")
        })?;
        control.steer(message)
    }

    pub fn pause_state(id: &str) -> Option<SubagentPauseState> {
        controls()
            .lock()
            .unwrap()
            .get(id)
            .map(SubagentControl::pause_state)
    }

    pub fn control_pause(id: &str, resume: bool) -> Result<SubagentPauseState, String> {
        let control = controls()
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| format!("no active subagent found with ID {id}"))?;
        control
            .set_paused(resume)
            .map_err(|e| format!("subagent {id} {e}"))
    }

    pub fn swap_child_model(id: &str, model: String) -> Result<(), String> {
        let control = controls()
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| format!("No active subagent found with ID {id}"))?;
        control.request_model(model)
    }
}

impl Drop for SubagentSession {
    fn drop(&mut self) {
        if self.completion.outcome().is_some() {
            return;
        }
        if std::thread::panicking() {
            self.completion.interrupt(SubagentLaunchFailure::Panicked);
        }
        let reason = self
            .completion
            .failure()
            .map(|f| f.to_string())
            .unwrap_or_else(|| "Subagent execution dropped before completion".into());
        let outcome = self.discard_resources(SubagentOutcome::Failed { error: reason });
        self.publish_outcome(outcome);
    }
}

/// Live next-turn model selection shared with the controlling client. Clones
/// reference the same child; closing the handle rejects late deliveries.
#[derive(Clone, Default)]
pub struct SubagentModelControl(Arc<Mutex<(bool, Option<String>)>>);

impl SubagentModelControl {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new((true, None))))
    }

    pub fn request(&self, model: String) -> Result<(), &'static str> {
        let mut state = self.0.lock().unwrap();
        if !state.0 {
            return Err("subagent is no longer accepting model changes");
        }
        state.1 = Some(model);
        Ok(())
    }

    fn take(&self) -> Option<String> {
        self.0.lock().unwrap().1.take()
    }

    pub fn close(&self) {
        self.0.lock().unwrap().0 = false;
    }
}

/// Acknowledgement of an in-process child. `queued` describes the slot at
/// admission time; the stable ID is also used by inspection and cancellation.
pub struct SubagentLaunch {
    pub child_id: String,
    pub queued: bool,
}

/// Failure before or during an independently owned background child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentLaunchFailure {
    Cancelled,
    TimedOut,
    Closed,
    Panicked,
}

impl std::fmt::Display for SubagentLaunchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "Subagent cancelled by parent",
            Self::TimedOut => "timed out waiting for a subagent slot",
            Self::Closed => "subagent semaphore closed",
            Self::Panicked => "subagent task panicked",
        })
    }
}

/// Read-only receipt for an independently owned child's actual terminal result.
/// The receipt is populated after workspace cleanup, including interrupted or
/// panicking executions whose owner was dropped. It cannot control the child.
#[derive(Clone, Default)]
pub struct SubagentCompletion(Arc<Mutex<CompletionState>>);

#[derive(Default)]
struct CompletionState {
    outcome: Option<SubagentOutcome>,
    failure: Option<SubagentLaunchFailure>,
    finishing: bool,
    cancellation_requested: bool,
}

impl SubagentCompletion {
    /// Finalization is a commit boundary: new control requests are refused.
    pub fn is_finishing(&self) -> bool {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.finishing || state.failure.is_some() || state.outcome.is_some()
    }
    /// The once-only terminal result, including reconciliation/cleanup errors.
    pub fn outcome(&self) -> Option<SubagentOutcome> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .outcome
            .clone()
    }

    /// Why execution was interrupted, if applicable. Cleanup can additionally
    /// fail; consult `outcome` for the full diagnostic.
    pub fn failure(&self) -> Option<SubagentLaunchFailure> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).failure
    }

    fn interrupt(&self, reason: SubagentLaunchFailure) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.outcome.is_none() && state.failure.is_none() {
            state.failure = Some(reason);
        }
    }
}

/// A single-use cancellation request. Shared clones cannot acknowledge the
/// same request twice, and a dropped receiver is never reported as live.
#[derive(Clone)]
pub struct SubagentCancellation {
    inner: Arc<Mutex<(tokio::sync::mpsc::Sender<()>, bool)>>,
    completion: Option<SubagentCompletion>,
}

impl SubagentCancellation {
    pub fn new(sender: tokio::sync::mpsc::Sender<()>) -> Self {
        Self {
            inner: Arc::new(Mutex::new((sender, false))),
            completion: None,
        }
    }

    pub fn cancel(&self) -> Result<(), &'static str> {
        let mut completion = self
            .completion
            .as_ref()
            .map(|receipt| receipt.0.lock().unwrap_or_else(|e| e.into_inner()));
        if completion.as_ref().is_some_and(|state| {
            state.finishing || state.failure.is_some() || state.outcome.is_some()
        }) {
            return Err("subagent is finalizing or finished");
        }
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.1 || state.0.is_closed() {
            return Err("subagent is no longer accepting cancellation");
        }
        state
            .0
            .try_send(())
            .map_err(|_| "subagent is no longer accepting cancellation")?;
        state.1 = true;
        if let Some(completion) = completion.as_mut() {
            completion.cancellation_requested = true;
        }
        Ok(())
    }

    pub fn close(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .1 = true;
    }

    /// Bind cancellation admission to the child's finalization boundary.
    pub fn with_completion(mut self, completion: SubagentCompletion) -> Self {
        self.completion = Some(completion);
        self
    }
}

impl SubagentSession {
    /// Transfer ownership of a child to the runtime before waiting for a slot.
    /// Completion is delivered once even if the invoking future has ended.
    pub fn launch_background<R, F, Fut, D, Delivery>(
        mut self,
        semaphore: Arc<Semaphore>,
        timeout: Duration,
        mut cancel: tokio::sync::mpsc::Receiver<()>,
        run: F,
        deliver: D,
    ) -> SubagentLaunch
    where
        R: Send + 'static,
        F: FnOnce(Self, OwnedSemaphorePermit) -> Fut + Send + 'static,
        Fut: Future<Output = R> + Send + 'static,
        D: FnOnce(Result<R, SubagentLaunchFailure>) -> Delivery + Send + 'static,
        Delivery: Future<Output = ()> + Send + 'static,
    {
        let launch = SubagentLaunch {
            child_id: self.session_id.clone(),
            queued: semaphore.available_permits() == 0,
        };
        let completion = self.completion();
        tokio::spawn(async move {
            let acquired = self.acquire_slot(semaphore, timeout, &mut cancel).await;
            let result = match acquired {
                Err(reason) => Err(reason),
                Ok(permit) => {
                    let future =
                        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            run(self, permit)
                        })) {
                            Ok(future) => future,
                            Err(_) => {
                                completion.interrupt(SubagentLaunchFailure::Panicked);
                                deliver(Err(SubagentLaunchFailure::Panicked)).await;
                                return;
                            }
                        };
                    // Box the owned future explicitly: it must be dropped (and
                    // finish cleanup) before completion is delivered.
                    let mut running = Box::pin(std::panic::AssertUnwindSafe(future).catch_unwind());
                    let result = tokio::select! {
                        biased;
                        Some(()) = cancel.recv() => {
                            if completion.is_finishing() {
                                // A committed completion cannot be turned into
                                // cancellation by dropping its cleanup owner.
                                running.as_mut().await.map_err(|_| SubagentLaunchFailure::Panicked)
                            } else {
                                completion.interrupt(SubagentLaunchFailure::Cancelled);
                                Err(SubagentLaunchFailure::Cancelled)
                            }
                        }
                        outcome = &mut running => match outcome {
                            Ok(result) => Ok(result),
                            Err(_) => {
                                completion.interrupt(SubagentLaunchFailure::Panicked);
                                Err(SubagentLaunchFailure::Panicked)
                            }
                        },
                    };
                    drop(running);
                    result
                }
            };
            deliver(result).await;
        });
        launch
    }

    /// The stable, read-only completion receipt survives transfer to a child.
    pub fn completion(&self) -> SubagentCompletion {
        self.completion.clone()
    }

    /// Observe pause transitions without exposing lifecycle mutation to hosts.
    pub fn subscribe_pause(&self) -> tokio::sync::watch::Receiver<SubagentPauseState> {
        self.pause.subscribe()
    }

    /// Wait for a concurrency slot while retaining queued cancellation. All
    /// admission failures finalize through the same session-owned cleanup path.
    ///
    /// # Errors
    /// Returns cancellation, queue timeout, or a closed semaphore.
    pub async fn acquire_slot(
        &mut self,
        semaphore: Arc<Semaphore>,
        timeout: Duration,
        cancel: &mut tokio::sync::mpsc::Receiver<()>,
    ) -> Result<OwnedSemaphorePermit, SubagentLaunchFailure> {
        if self.completion.outcome().is_some() {
            return Err(SubagentLaunchFailure::Closed);
        }
        let result = tokio::select! {
                biased;
                Some(()) = cancel.recv() => Err(SubagentLaunchFailure::Cancelled),
                acquired = tokio::time::timeout(timeout, semaphore.acquire_owned()) => match acquired {
                    Ok(Ok(permit)) => Ok(permit),
                    Ok(Err(_)) => Err(SubagentLaunchFailure::Closed),
                    Err(_) => Err(SubagentLaunchFailure::TimedOut),
                }
        };
        match &result {
            Ok(_) => {
                if let Some(control) = &self.control {
                    control.running();
                }
            }
            Err(reason) => {
                self.finalize_interruption(*reason).await;
            }
        }
        result
    }
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
            permission_service: None,
            tool_policy: None,
            steering_queue: Vec::new(),
            pending_model_swap: None,
            pause: SubagentPause::new(),
            model_control: None,
            parent_context: Vec::new(),
            control: None,
            completion: SubagentCompletion::default(),
            cleanup: Vec::new(),
            workspace_applied: false,
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
        if self.completion.outcome().is_some() || self.workspace_guard.is_some() {
            return Err(std::io::Error::other(
                "Subagent workspace is already prepared or finalized",
            ));
        }
        self.workspace_guard = Some(IsolatedWorkspaceGuard::new(primary_path, branch_name).await?);
        Ok(())
    }

    pub fn with_event_emitter(mut self, emitter: SubagentEventEmitter) -> Self {
        self.event_emitter = emitter;
        self
    }

    /// Transfer ephemeral host resources to this child's cleanup owner.
    /// Resources finalize in reverse registration order, so admission resources
    /// outlive workspace execution and its later-registered ephemeral state.
    pub fn with_cleanup(mut self, cleanup: Box<dyn SubagentCleanup>) -> Self {
        self.cleanup.push(cleanup);
        self
    }

    pub fn with_approval_channel(mut self, channel: SubagentApprovalChannel) -> Self {
        self.approval_channel = channel;
        self
    }

    pub fn with_permission_service(mut self, service: Arc<dyn PermissionService>) -> Self {
        self.permission_service = Some(service);
        self
    }

    pub fn with_tool_policy(mut self, policy: SubagentToolPolicy) -> Self {
        self.tool_policy = Some(policy);
        self
    }

    pub fn with_model_control(mut self, control: SubagentModelControl) -> Self {
        if let Some(registered) = &self.control {
            registered.0.lock().unwrap().model = control.clone();
        }
        self.model_control = Some(control);
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

    /// Finalize once, reconciling successful work and closing the workspace on
    /// every outcome. The returned result includes merge and cleanup failures.
    pub async fn finalize_outcome(&mut self, outcome: SubagentOutcome) -> SubagentOutcome {
        if let Some(finalized) = self.completion.outcome() {
            return finalized;
        }
        let mut outcome = {
            let mut completion = self.completion.0.lock().unwrap_or_else(|e| e.into_inner());
            // Cancellation admission and the commit boundary share this lock:
            // an acknowledged request cannot be lost between the last turn and
            // workspace reconciliation.
            completion.finishing = true;
            if completion.cancellation_requested {
                completion.failure = Some(SubagentLaunchFailure::Cancelled);
                let mut error = SubagentLaunchFailure::Cancelled.to_string();
                if !outcome.is_success() && outcome.summary_text() != error.as_str() {
                    error.push_str(&format!(
                        "; prior execution outcome: {}",
                        outcome.summary_text()
                    ));
                }
                SubagentOutcome::Failed { error }
            } else {
                outcome
            }
        };
        if let SubagentOutcome::Done {
            iterations,
            tool_calls_count,
            token_usage,
            ..
        } = &mut outcome
        {
            *iterations = self.current_iteration;
            *tool_calls_count = self.total_tool_calls;
            *token_usage = self.cumulative_tokens as usize;
        }
        if let Some(guard) = &mut self.workspace_guard
            && let Err(error) = guard.finalize(outcome.is_success()).await
        {
            outcome = Self::cleanup_failed(outcome, error);
        }
        self.workspace_applied |= self
            .workspace_guard
            .as_ref()
            .is_some_and(IsolatedWorkspaceGuard::is_committed);
        self.workspace_guard = None;
        for cleanup in self.cleanup.iter_mut().rev() {
            if let Err(error) = cleanup.finalize(outcome.is_success()).await {
                outcome = Self::resource_cleanup_failed(outcome, error);
            }
        }
        self.cleanup.clear();
        self.annotate_applied_workspace(&mut outcome);
        self.publish_outcome(outcome.clone());
        outcome
    }

    fn cleanup_failed(outcome: SubagentOutcome, error: std::io::Error) -> SubagentOutcome {
        SubagentOutcome::Failed {
            error: format!(
                "workspace finalization failed: {error}; prior execution outcome: {}",
                outcome.summary_text()
            ),
        }
    }

    fn resource_cleanup_failed(outcome: SubagentOutcome, error: String) -> SubagentOutcome {
        SubagentOutcome::Failed {
            error: format!(
                "ephemeral cleanup/writeback failed: {error}; prior execution outcome: {}",
                outcome.summary_text()
            ),
        }
    }

    fn annotate_applied_workspace(&self, outcome: &mut SubagentOutcome) {
        if self.workspace_applied
            && let SubagentOutcome::Failed { error } = outcome
        {
            *error = format!(
                "isolated workspace reconciliation succeeded before cleanup failed; {error}"
            );
        }
    }

    // Used by both owner drop and a finalization deadline. No asynchronous
    // cleanup can keep a terminated child (or its concurrency slot) alive.
    fn discard_resources(&mut self, mut outcome: SubagentOutcome) -> SubagentOutcome {
        if let Some(mut guard) = self.workspace_guard.take() {
            self.workspace_applied |= guard.is_committed();
            if let Err(error) = guard.close() {
                outcome = Self::cleanup_failed(outcome, error);
            }
        }
        for mut cleanup in std::mem::take(&mut self.cleanup).into_iter().rev() {
            if let Err(error) = cleanup.discard() {
                outcome = Self::resource_cleanup_failed(outcome, error);
            }
        }
        self.annotate_applied_workspace(&mut outcome);
        outcome
    }

    fn publish_outcome(&mut self, outcome: SubagentOutcome) {
        {
            let mut completion = self.completion.0.lock().unwrap_or_else(|e| e.into_inner());
            if completion.outcome.is_some() {
                return;
            }
            completion.outcome = Some(outcome.clone());
        }
        self.pause.finish();
        if let Some(control) = &self.model_control {
            control.close();
        }
        self.steering_queue.clear();
        self.pending_model_swap = None;
        if let Some(control) = &self.control {
            control.finished(match self.completion.failure() {
                Some(SubagentLaunchFailure::Cancelled) => "cancelled",
                Some(SubagentLaunchFailure::TimedOut) => "timeout",
                Some(SubagentLaunchFailure::Closed | SubagentLaunchFailure::Panicked) => "error",
                None => match &outcome {
                    SubagentOutcome::Done { .. } => "done",
                    SubagentOutcome::Blocked { .. } => "blocked",
                    SubagentOutcome::Failed { .. } => "error",
                    SubagentOutcome::Exhausted { .. } => "exhausted",
                },
            });
        }
        self.event_emitter.emit_terminal(outcome);
    }

    /// Finalize an admission failure or interrupted run using session ownership.
    pub async fn finalize_interruption(
        &mut self,
        reason: SubagentLaunchFailure,
    ) -> SubagentOutcome {
        self.completion.interrupt(reason);
        self.finalize_outcome(SubagentOutcome::Failed {
            error: reason.to_string(),
        })
        .await
    }

    /// Enqueue a steering message to be prioritized on the subagent's subsequent turn.
    pub fn steer(&mut self, message: String) -> bool {
        if self.completion.is_finishing() {
            return false;
        }
        self.steering_queue.push(message);
        true
    }

    /// Request a model hot-swap taking effect on the subsequent turn.
    pub fn hot_swap_model(&mut self, new_model: String) -> bool {
        if self.completion.is_finishing() {
            return false;
        }
        self.pending_model_swap = Some(new_model);
        true
    }

    pub fn take_pending_steering(&mut self) -> Vec<String> {
        let mut guidance = std::mem::take(&mut self.steering_queue);
        if let Some(control) = &self.control {
            guidance.extend(control.take_guidance());
        }
        guidance
    }

    pub fn take_pending_model_hot_swap(&mut self) -> Option<String> {
        let initial = self.pending_model_swap.take();
        self.model_control
            .as_ref()
            .and_then(SubagentModelControl::take)
            .or(initial)
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

    /// Execute with session-owned wall-clock timeout, cancellation and panic
    /// handling. Interrupted tool/approval futures are dropped before cleanup;
    /// the host receives the actual finalized result through this same seam.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_controlled<L: SubagentLlmExecutor, T: SubagentToolExecutor>(
        &mut self,
        llm: &L,
        tools: &T,
        model: String,
        system_prompt: String,
        initial_prompt: String,
        tool_schemas: Vec<Value>,
        failover_models: Vec<String>,
        primary_path: &Path,
        timeout: Duration,
        cancel: &mut tokio::sync::mpsc::Receiver<()>,
    ) -> SubagentOutcome {
        if let Some(outcome) = self.completion.outcome() {
            return outcome;
        }
        let deadline = tokio::time::Instant::now() + timeout;
        let result = {
            let run = std::panic::AssertUnwindSafe(self.run_reasoning_loop(
                llm,
                tools,
                model,
                system_prompt,
                initial_prompt,
                tool_schemas,
                failover_models,
                primary_path,
            ))
            .catch_unwind();
            tokio::select! {
                biased;
                Some(()) = cancel.recv() => Err(SubagentLaunchFailure::Cancelled),
                result = tokio::time::timeout_at(deadline, run) => match result {
                    Ok(Ok(outcome)) => Ok(outcome),
                    Ok(Err(_)) => Err(SubagentLaunchFailure::Panicked),
                    Err(_) => Err(SubagentLaunchFailure::TimedOut),
                },
            }
        };
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(reason) => {
                self.completion.interrupt(reason);
                let error = if reason == SubagentLaunchFailure::TimedOut {
                    format!(
                        "Subagent wall-clock timeout after {}s",
                        timeout.as_secs_f64()
                    )
                } else {
                    reason.to_string()
                };
                SubagentOutcome::Failed { error }
            }
        };
        let initial_summary = outcome.summary_text().to_string();
        let finalized = tokio::time::timeout_at(
            deadline,
            std::panic::AssertUnwindSafe(self.finalize_outcome(outcome)).catch_unwind(),
        )
        .await;
        if let Some(outcome) = self.completion.outcome() {
            return outcome;
        }
        match finalized {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => {
                self.completion.interrupt(SubagentLaunchFailure::Panicked);
                let outcome = self.discard_resources(SubagentOutcome::Failed {
                    error: format!("Subagent task panicked during finalization; prior execution outcome: {initial_summary}"),
                });
                self.publish_outcome(outcome.clone());
                outcome
            }
            Err(_) => {
                self.completion.interrupt(SubagentLaunchFailure::TimedOut);
                let outcome = self.discard_resources(SubagentOutcome::Failed {
                    error: format!("Subagent wall-clock timeout during finalization after {}s; prior execution outcome: {initial_summary}", timeout.as_secs_f64()),
                });
                self.publish_outcome(outcome.clone());
                outcome
            }
        }
    }

    /// Execute the full autonomous reasoning loop until completion, budget exhaustion, or error.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_autonomous_loop<L: SubagentLlmExecutor, T: SubagentToolExecutor>(
        &mut self,
        llm: &L,
        tools: &T,
        model: String,
        system_prompt: String,
        initial_prompt: String,
        tool_schemas: Vec<Value>,
        failover_models: Vec<String>,
        primary_path: &Path,
    ) -> SubagentOutcome {
        if let Some(outcome) = self.completion.outcome() {
            return outcome;
        }
        let outcome = self
            .run_reasoning_loop(
                llm,
                tools,
                model,
                system_prompt,
                initial_prompt,
                tool_schemas,
                failover_models,
                primary_path,
            )
            .await;
        self.finalize_outcome(outcome).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_reasoning_loop<L: SubagentLlmExecutor, T: SubagentToolExecutor>(
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
        self.pause.start();
        if let Some(control) = &self.control {
            control.running();
        }
        if self.config.enforce_isolation && self.workspace_guard.is_none() {
            return SubagentOutcome::Failed {
                    error: "Required subagent isolation could not be established; refusing to run in the live workspace".into(),
                };
        }
        let system_prompt = format!("{system_prompt}{}", self.bounded_parent_context());
        let mut messages = vec![SubagentMessage::user(initial_prompt)];
        let mut last_text = String::new();
        let mut failover_idx = 0;

        for _iter in 0..self.max_iters {
            // All tool calls from the previous turn finish before this gate.
            // No next turn (or its tools) starts while the gate is paused.
            self.pause.boundary(&self.event_emitter).await;
            // 1. Dynamic Model Hot-Swap check
            if let Some(new_model) = self.take_pending_model_hot_swap()
                && new_model != model
            {
                model = new_model;
                failover_idx = 0;
            }

            // 2. Priority Steering Guidance Queue Drain
            let steer_msgs = self.take_pending_steering();
            if !steer_msgs.is_empty() {
                let guidance = format!(
                    "[Supervisor Steering Guidance]:\n\n{}",
                    steer_msgs.join("\n\n")
                );
                messages.push(SubagentMessage::user(guidance));
                self.event_emitter
                    .emit(SubagentEvent::SteeringApplied {
                        messages: steer_msgs.len(),
                    })
                    .await;
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
                let (candidate_prompt, candidate_tools) =
                    llm.prepare_turn(candidate, &system_prompt, &tool_schemas);
                match llm
                    .complete_turn(candidate, &candidate_prompt, &messages, &candidate_tools)
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
                    return SubagentOutcome::Failed { error: err_msg };
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
                return SubagentOutcome::Exhausted {
                    reason,
                    iterations: self.current_iteration,
                    tokens_used: self.cumulative_tokens as usize,
                };
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
                return outcome;
            }

            // 8. Natural completion (no tool calls and has text)
            if resp.tool_calls.is_empty() {
                let summary = if !last_text.is_empty() {
                    last_text
                } else {
                    "Task concluded without tool calls.".to_string()
                };
                return SubagentOutcome::Done {
                    summary,
                    iterations: self.current_iteration,
                    tool_calls_count: self.total_tool_calls,
                    token_usage: self.cumulative_tokens as usize,
                };
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
                            let response = if let Some(service) = &self.permission_service {
                                service
                                    .request_permission(&tc.name, &tc.arguments)
                                    .await
                                    .map(|approved| SubagentApprovalResponse {
                                        approved,
                                        feedback: None,
                                    })
                            } else {
                                self.approval_channel
                                    .request_approval(&approval_id, &tc.name, &tc.arguments)
                                    .await
                            };
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
        if !last_text.is_empty() {
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
        }
    }
}

#[cfg(test)]
#[path = "session_lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use cade_core::permissions::PermissionMode;
    use tempfile::tempdir;

    #[tokio::test]
    async fn pause_at_turn_boundary_keeps_tools_and_conversation_until_resume() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct TwoTurns {
            calls: AtomicUsize,
            entered: tokio::sync::Notify,
            release: tokio::sync::Notify,
        }
        #[async_trait]
        impl SubagentLlmExecutor for TwoTurns {
            async fn complete_turn(
                &self,
                _: &str,
                _: &str,
                messages: &[SubagentMessage],
                _: &[Value],
            ) -> Result<SubagentTurnResponse, String> {
                let call = self.calls.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    self.entered.notify_one();
                    self.release.notified().await;
                    Ok(SubagentTurnResponse {
                        content: Some("first turn".into()),
                        tokens_used: 1,
                        tool_calls: vec![SubagentToolCall {
                            id: "read".into(),
                            name: "read_file".into(),
                            arguments: json!({}),
                        }],
                    })
                } else {
                    assert!(messages.iter().any(|m| m.content == "first turn"));
                    assert!(
                        messages
                            .iter()
                            .any(|m| m.content.contains("finish the task"))
                    );
                    assert!(
                        messages
                            .iter()
                            .any(|m| m.role == "tool" && m.content == "retained tool result")
                    );
                    Ok(SubagentTurnResponse {
                        content: None,
                        tokens_used: 1,
                        tool_calls: vec![SubagentToolCall {
                            id: "finish".into(),
                            name: "finish".into(),
                            arguments: json!({"status":"done", "summary":"resumed"}),
                        }],
                    })
                }
            }
        }
        struct ReadTool(AtomicUsize);
        #[async_trait]
        impl SubagentToolExecutor for ReadTool {
            async fn execute_tool(
                &self,
                _: &str,
                _: &str,
                _: &Value,
                _: &Path,
            ) -> Result<String, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok("retained tool result".into())
            }
        }

        let llm = Arc::new(TwoTurns {
            calls: AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        let tools = Arc::new(ReadTool(AtomicUsize::new(0)));
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_tool_policy(policy("build"))
        .with_event_emitter(SubagentEventEmitter::new(Some(tx)));
        let id = session.session_id.clone();
        session.register_control(true).running();
        let control = session.pause.clone();
        assert!(control.pause().is_err(), "queued child has not started");
        let run_llm = llm.clone();
        let run_tools = tools.clone();
        let run = tokio::spawn(async move {
            session
                .run_autonomous_loop(
                    &*run_llm,
                    &*run_tools,
                    "test".into(),
                    "system".into(),
                    "task".into(),
                    vec![],
                    vec![],
                    Path::new("."),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), llm.entered.notified())
            .await
            .unwrap();
        assert_eq!(control.pause(), Ok(SubagentPauseState::PauseRequested));
        assert_eq!(
            SubagentSession::child_status(&id).unwrap(),
            SubagentStatus::PauseRequested
        );
        assert!(control.pause().is_err());
        llm.release.notify_one();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    rx.recv().await,
                    Some(SubagentEvent::PauseStateChanged {
                        state: SubagentPauseState::Paused
                    })
                ) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(control.state(), SubagentPauseState::Paused);
        assert_eq!(
            SubagentSession::child_status(&id).unwrap(),
            SubagentStatus::Paused
        );
        SubagentSession::steer_child(&id, "finish the task".into()).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(llm.calls.load(Ordering::SeqCst), 1);
        assert_eq!(tools.0.load(Ordering::SeqCst), 1);
        assert!(!run.is_finished());
        assert_eq!(control.resume(), Ok(SubagentPauseState::Running));
        assert_eq!(
            SubagentSession::child_status(&id).unwrap(),
            SubagentStatus::Running
        );
        assert!(control.resume().is_err());
        assert!(
            matches!(run.await.unwrap(), SubagentOutcome::Done { summary, .. } if summary == "resumed")
        );
        assert_eq!(llm.calls.load(Ordering::SeqCst), 2);
        assert_eq!(control.state(), SubagentPauseState::Finished);
        assert!(control.resume().is_err());
        assert!(control.pause().is_err());
    }

    #[tokio::test]
    async fn live_model_change_preserves_conversation_and_policy_on_next_turn() {
        struct ObservedTurn {
            model: String,
            prompt: String,
            messages: Vec<SubagentMessage>,
            schemas: Vec<Value>,
        }

        struct TwoTurns {
            first_started: tokio::sync::Notify,
            release_first: tokio::sync::Notify,
            seen: Mutex<Vec<ObservedTurn>>,
        }
        #[async_trait]
        impl SubagentLlmExecutor for TwoTurns {
            async fn complete_turn(
                &self,
                model: &str,
                prompt: &str,
                messages: &[SubagentMessage],
                schemas: &[Value],
            ) -> Result<SubagentTurnResponse, String> {
                let first = {
                    let mut seen = self.seen.lock().unwrap();
                    seen.push(ObservedTurn {
                        model: model.into(),
                        prompt: prompt.into(),
                        messages: messages.to_vec(),
                        schemas: schemas.to_vec(),
                    });
                    seen.len() == 1
                };
                if first {
                    self.first_started.notify_one();
                    self.release_first.notified().await;
                    Ok(SubagentTurnResponse {
                        content: Some("prior assistant turn".into()),
                        tool_calls: vec![SubagentToolCall {
                            id: "read-1".into(),
                            name: "read_file".into(),
                            arguments: json!({}),
                        }],
                        tokens_used: 2,
                    })
                } else {
                    Ok(SubagentTurnResponse {
                        content: None,
                        tool_calls: vec![SubagentToolCall {
                            id: "finish-2".into(),
                            name: "finish".into(),
                            arguments: json!({"status":"done", "summary":"changed model saw earlier turn"}),
                        }],
                        tokens_used: 3,
                    })
                }
            }
        }
        let llm = TwoTurns {
            first_started: tokio::sync::Notify::new(),
            release_first: tokio::sync::Notify::new(),
            seen: Mutex::new(vec![]),
        };
        let control = SubagentModelControl::new();
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_model_control(control.clone())
        .with_tool_policy(policy("build"));
        let schemas = vec![json!({"name":"read_file", "parameters":{"type":"object"}})];
        let run = session.run_autonomous_loop(
            &llm,
            &MockToolExecutor,
            "provider-a/first".into(),
            "system instructions".into(),
            "task".into(),
            schemas.clone(),
            vec![],
            Path::new("."),
        );
        tokio::pin!(run);
        tokio::select! {
            _ = llm.first_started.notified() => {}
            _ = &mut run => panic!("child finished before first call was released"),
        }
        control.request("provider-b/second".into()).unwrap();
        assert_eq!(llm.seen.lock().unwrap()[0].model, "provider-a/first");
        llm.release_first.notify_one();
        let outcome = run.await;
        assert_eq!(outcome.summary_text(), "changed model saw earlier turn");
        let seen = llm.seen.lock().unwrap();
        assert_eq!(
            seen.iter().map(|s| s.model.as_str()).collect::<Vec<_>>(),
            vec!["provider-a/first", "provider-b/second"]
        );
        assert_eq!(seen[0].prompt, seen[1].prompt);
        assert_eq!(seen[0].schemas, schemas);
        assert_eq!(seen[1].schemas, schemas);
        assert!(
            seen[1]
                .messages
                .iter()
                .any(|m| m.role == "assistant" && m.content == "prior assistant turn")
        );
        assert!(
            seen[1]
                .messages
                .iter()
                .any(|m| m.role == "tool" && m.content == "Output from read_file")
        );
        assert!(control.request("third".into()).is_err());
    }

    #[tokio::test]
    async fn cancelling_running_child_interrupts_blocked_tool_and_delivers_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct ToolLlm;
        #[async_trait]
        impl SubagentLlmExecutor for ToolLlm {
            async fn complete_turn(
                &self,
                _: &str,
                _: &str,
                _: &[SubagentMessage],
                _: &[Value],
            ) -> Result<SubagentTurnResponse, String> {
                Ok(SubagentTurnResponse {
                    content: None,
                    tokens_used: 1,
                    tool_calls: vec![
                        SubagentToolCall {
                            id: "one".into(),
                            name: "read_file".into(),
                            arguments: json!({}),
                        },
                        SubagentToolCall {
                            id: "two".into(),
                            name: "read_file".into(),
                            arguments: json!({}),
                        },
                    ],
                })
            }
        }
        struct BlockedTool {
            entered: tokio::sync::Notify,
            calls: AtomicUsize,
            dropped: Arc<std::sync::atomic::AtomicBool>,
        }
        #[async_trait]
        impl SubagentToolExecutor for BlockedTool {
            async fn execute_tool(
                &self,
                _: &str,
                _: &str,
                _: &Value,
                _: &Path,
            ) -> Result<String, String> {
                struct DropSignal(Arc<std::sync::atomic::AtomicBool>);
                impl Drop for DropSignal {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::SeqCst);
                    }
                }
                let _guard = DropSignal(self.dropped.clone());
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.entered.notify_one();
                std::future::pending().await
            }
        }
        let tools = Arc::new(BlockedTool {
            entered: tokio::sync::Notify::new(),
            calls: AtomicUsize::new(0),
            dropped: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        let slots = Arc::new(Semaphore::new(1));
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let cancellation = SubagentCancellation::new(tx);
        let (outcome_tx, mut outcome_rx) = tokio::sync::mpsc::unbounded_channel();
        let policy = SubagentToolPolicy {
            permissions: PermissionManager::new(PermissionMode::BypassPermissions),
            tools: SubagentTools::All,
            inherited_tools: vec!["read_file".into()],
            allow_nesting: false,
            max_depth: 2,
        };
        let launch = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_tool_policy(policy)
        .launch_background(
            slots.clone(),
            Duration::from_secs(2),
            rx,
            {
                let tools = tools.clone();
                move |mut session, permit| async move {
                    let _permit = permit;
                    session
                        .run_autonomous_loop(
                            &ToolLlm,
                            tools.as_ref(),
                            "test".into(),
                            "system".into(),
                            "task".into(),
                            vec![],
                            vec![],
                            Path::new("."),
                        )
                        .await
                }
            },
            move |result| async move {
                outcome_tx.send(result).unwrap();
            },
        );
        assert!(!launch.queued);
        tokio::time::timeout(Duration::from_secs(2), tools.entered.notified())
            .await
            .unwrap();
        cancellation.cancel().unwrap();
        assert!(cancellation.cancel().is_err());
        let result = tokio::time::timeout(Duration::from_secs(2), outcome_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), SubagentLaunchFailure::Cancelled);
        assert!(
            tools.dropped.load(Ordering::SeqCst),
            "blocked tool future must be dropped"
        );
        assert_eq!(
            tools.calls.load(Ordering::SeqCst),
            1,
            "no further tool calls"
        );
        assert_eq!(slots.available_permits(), 1);
        assert!(outcome_rx.try_recv().is_err());
        assert!(cancellation.cancel().is_err());
    }

    #[tokio::test]
    async fn cancellation_refuses_closed_and_completed_receivers() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let handle = SubagentCancellation::new(tx);
        drop(rx);
        assert!(handle.cancel().is_err());
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let completed = SubagentCancellation::new(tx);
        completed.close();
        assert!(completed.cancel().is_err());
    }

    #[tokio::test]
    async fn cancellation_releases_pending_approval_response() {
        struct WriteLlm;
        #[async_trait]
        impl SubagentLlmExecutor for WriteLlm {
            async fn complete_turn(
                &self,
                _: &str,
                _: &str,
                _: &[SubagentMessage],
                _: &[Value],
            ) -> Result<SubagentTurnResponse, String> {
                Ok(SubagentTurnResponse {
                    content: None,
                    tokens_used: 1,
                    tool_calls: vec![SubagentToolCall {
                        id: "write".into(),
                        name: "write_file".into(),
                        arguments: json!({"path":"test.txt","content":"x"}),
                    }],
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
                panic!("approval must be resolved before execution")
            }
        }
        let slots = Arc::new(Semaphore::new(1));
        let (cancel_tx, cancel_rx) = tokio::sync::mpsc::channel(1);
        let cancellation = SubagentCancellation::new(cancel_tx);
        let (approval_tx, mut approval_rx) = tokio::sync::mpsc::channel(1);
        let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel();
        let session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_tool_policy(SubagentToolPolicy {
            permissions: PermissionManager::new(PermissionMode::Default),
            tools: SubagentTools::All,
            inherited_tools: vec!["write_file".into()],
            allow_nesting: false,
            max_depth: 2,
        })
        .with_approval_channel(SubagentApprovalChannel::new(approval_tx));
        session.launch_background(
            slots.clone(),
            Duration::from_secs(2),
            cancel_rx,
            |mut session, permit| async move {
                let _permit = permit;
                session
                    .run_autonomous_loop(
                        &WriteLlm,
                        &NoTools,
                        "test".into(),
                        "system".into(),
                        "task".into(),
                        vec![],
                        vec![],
                        Path::new("."),
                    )
                    .await
            },
            move |result| async move {
                outcome_tx.send(result).unwrap();
            },
        );
        let (_, _, _, reply) = tokio::time::timeout(Duration::from_secs(2), approval_rx.recv())
            .await
            .unwrap()
            .unwrap();
        cancellation.cancel().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), outcome_rx)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err()
                == SubagentLaunchFailure::Cancelled
        );
        assert!(
            reply
                .send(SubagentApprovalResponse {
                    approved: true,
                    feedback: None
                })
                .is_err()
        );
        assert_eq!(slots.available_permits(), 1);
    }

    #[tokio::test]
    async fn background_launch_acknowledges_blocked_and_queued_children_and_delivers_once() {
        struct BlockedLlm(tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>);
        #[async_trait]
        impl SubagentLlmExecutor for BlockedLlm {
            async fn complete_turn(
                &self,
                _: &str,
                _: &str,
                _: &[SubagentMessage],
                _: &[Value],
            ) -> Result<SubagentTurnResponse, String> {
                self.0
                    .lock()
                    .await
                    .take()
                    .unwrap()
                    .await
                    .map_err(|e| e.to_string())?;
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
                panic!("unexpected tool invocation")
            }
        }
        let semaphore = Arc::new(Semaphore::new(1));
        let (release, blocked) = tokio::sync::oneshot::channel();
        let (outcome_tx, mut outcomes) = tokio::sync::mpsc::unbounded_channel();
        let llm = Arc::new(BlockedLlm(tokio::sync::Mutex::new(Some(blocked))));
        let run = move |mut session: SubagentSession, _permit: OwnedSemaphorePermit| {
            let llm = llm.clone();
            async move {
                let _hold_slot = _permit;
                session
                    .run_autonomous_loop(
                        llm.as_ref(),
                        &NoTools,
                        "test".into(),
                        "system".into(),
                        "task".into(),
                        vec![],
                        vec![],
                        Path::new("."),
                    )
                    .await
            }
        };
        let first = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"first"})),
            "parent",
        )
        .launch_background(
            semaphore.clone(),
            Duration::from_secs(2),
            tokio::sync::mpsc::channel(1).1,
            run,
            {
                let tx = outcome_tx.clone();
                move |outcome| async move {
                    tx.send(outcome).unwrap();
                }
            },
        );
        assert!(!first.child_id.is_empty());
        // Wait for the child to enter its blocked LLM turn before launching the next.
        tokio::time::timeout(Duration::from_secs(2), async {
            while semaphore.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let second = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"second"})),
            "parent",
        )
        .launch_background(
            semaphore.clone(),
            Duration::from_secs(2),
            tokio::sync::mpsc::channel(1).1,
            |mut session, _permit| async move {
                session
                    .finalize_outcome(SubagentOutcome::Failed {
                        error: "failed".into(),
                    })
                    .await
            },
            {
                let tx = outcome_tx.clone();
                move |outcome| async move {
                    tx.send(outcome).unwrap();
                }
            },
        );
        assert!(second.queued);
        assert_ne!(first.child_id, second.child_id);
        assert!(
            outcomes.try_recv().is_err(),
            "neither child has completed yet"
        );
        release.send(()).unwrap();
        let a = tokio::time::timeout(Duration::from_secs(2), outcomes.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let b = tokio::time::timeout(Duration::from_secs(2), outcomes.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(a.is_success());
        assert!(matches!(b, SubagentOutcome::Failed { .. }));
        assert!(outcomes.try_recv().is_err());
        assert_eq!(semaphore.available_permits(), 1);
    }

    #[tokio::test]
    async fn queued_launch_cancellation_and_timeout_report_failures_without_consuming_capacity() {
        let slots = Arc::new(Semaphore::new(1));
        let held = slots.clone().acquire_owned().await.unwrap();
        for cancel_first in [true, false] {
            let (cancel_tx, cancel_rx) = tokio::sync::mpsc::channel(1);
            let (result_tx, result_rx) = tokio::sync::oneshot::channel();
            let launch = SubagentSession::new(
                SubagentConfig::from_args(&json!({"prompt":"task"})),
                "parent",
            )
            .launch_background(
                slots.clone(),
                Duration::from_millis(30),
                cancel_rx,
                |_, _| async { panic!("queued child must not run") },
                move |result: Result<(), SubagentLaunchFailure>| async move {
                    result_tx.send(result).unwrap();
                },
            );
            assert!(launch.queued);
            if cancel_first {
                cancel_tx.send(()).await.unwrap();
            }
            let failure = tokio::time::timeout(Duration::from_secs(1), result_rx)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(
                failure,
                if cancel_first {
                    SubagentLaunchFailure::Cancelled
                } else {
                    SubagentLaunchFailure::TimedOut
                }
            );
        }
        assert_eq!(slots.available_permits(), 0);
        drop(held);
        assert_eq!(slots.available_permits(), 1);
    }

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
    async fn permission_service_only_sees_ask_and_cannot_override_deny_or_protected_paths() {
        struct RecordingService(Mutex<Vec<String>>);
        #[async_trait]
        impl PermissionService for RecordingService {
            async fn request_permission(&self, name: &str, _: &Value) -> Result<bool, String> {
                self.0.lock().unwrap().push(name.into());
                Ok(true)
            }
        }
        let service = Arc::new(RecordingService(Mutex::new(vec![])));
        let access = policy("build");
        access
            .permissions
            .add_deny_rule(cade_core::permissions::PermissionRule::parse("read_file").unwrap());
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"task"})),
            "parent",
        )
        .with_tool_policy(access)
        .with_permission_service(service.clone());
        let calls = scripted_calls(vec![
            ("read_file", json!({"path":"src/lib.rs"})),
            ("write_file", json!({"path":"src/lib.rs"})),
            ("write_file", json!({"path":".env"})),
            ("mcp__list_items", json!({})),
        ]);
        let tools = RecordingTools(Mutex::new(vec![]));
        exercise(&mut session, &calls, &tools).await;
        assert_eq!(*service.0.lock().unwrap(), vec!["write_file"]);
        assert_eq!(
            *tools.0.lock().unwrap(),
            vec!["write_file", "mcp__list_items"]
        );
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

    fn writing_policy() -> SubagentToolPolicy {
        let access = policy("build");
        access
            .permissions
            .set_mode(PermissionMode::BypassPermissions);
        access
    }

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
            .with_tool_policy(writing_policy())
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
            let mut session =
                SubagentSession::new(config, "parent").with_tool_policy(writing_policy());
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
        let mut session = SubagentSession::new(config, "parent").with_tool_policy(writing_policy());
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
        let mut session = SubagentSession::new(config, "parent").with_tool_policy(writing_policy());
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
        let mut session = SubagentSession::new(config, "parent").with_tool_policy(writing_policy());
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

    #[tokio::test]
    async fn child_control_reports_lifecycle_and_delivers_guidance_on_next_turn() {
        struct GatedLlm {
            entered: tokio::sync::Notify,
            release: tokio::sync::Notify,
            turns: std::sync::atomic::AtomicUsize,
            seen: Mutex<Vec<Vec<SubagentMessage>>>,
        }
        #[async_trait]
        impl SubagentLlmExecutor for GatedLlm {
            async fn complete_turn(
                &self,
                _: &str,
                _: &str,
                messages: &[SubagentMessage],
                _: &[Value],
            ) -> Result<SubagentTurnResponse, String> {
                let turn = self.turns.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.seen.lock().unwrap().push(messages.to_vec());
                if turn == 0 {
                    self.entered.notify_one();
                    self.release.notified().await;
                    Ok(SubagentTurnResponse {
                        content: None,
                        tool_calls: vec![SubagentToolCall {
                            id: "read".into(),
                            name: "read_file".into(),
                            arguments: json!({"path":"test.txt"}),
                        }],
                        tokens_used: 1,
                    })
                } else {
                    Ok(SubagentTurnResponse {
                        content: None,
                        tool_calls: vec![SubagentToolCall {
                            id: "finish".into(),
                            name: "finish".into(),
                            arguments: json!({"status":"done","summary":"guided"}),
                        }],
                        tokens_used: 1,
                    })
                }
            }
        }

        let llm = Arc::new(GatedLlm {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            turns: std::sync::atomic::AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        });
        let semaphore = Arc::new(Semaphore::new(1));
        let held = semaphore.clone().acquire_owned().await.unwrap();
        let mut session = SubagentSession::new(
            SubagentConfig::from_args(&json!({"prompt":"work"})),
            "parent",
        );
        let id = session.session_id.clone();
        let control = session.register_control(true);
        let (events_tx, mut events_rx) = tokio::sync::mpsc::channel(16);
        session = session.with_event_emitter(SubagentEventEmitter::new(Some(events_tx)));
        let (cancel_tx, cancel_rx) = tokio::sync::mpsc::channel(1);
        let (_keep_cancel, (done_tx, done_rx)) = (cancel_tx, tokio::sync::oneshot::channel());
        let running = control.clone();
        let llm_run = llm.clone();
        let launch = session.launch_background(
            semaphore,
            Duration::from_secs(5),
            cancel_rx,
            move |mut session, _permit| async move {
                running.running();
                session
                    .run_autonomous_loop(
                        llm_run.as_ref(),
                        &MockToolExecutor,
                        "model".into(),
                        "system".into(),
                        "work".into(),
                        vec![],
                        vec![],
                        Path::new("."),
                    )
                    .await
            },
            move |result| async move {
                let _ = done_tx.send(result);
            },
        );
        assert_eq!(launch.child_id, id);
        assert!(launch.queued);
        assert_eq!(
            SubagentSession::child_status(&id).unwrap(),
            SubagentStatus::Queued
        );
        assert!(SubagentSession::steer_child(&id, "too early".into()).is_err());
        assert!(SubagentSession::steer_child("missing-child", "no".into()).is_err());
        drop(held);
        tokio::time::timeout(Duration::from_secs(3), llm.entered.notified())
            .await
            .unwrap();
        assert_eq!(
            SubagentSession::child_status(&id).unwrap(),
            SubagentStatus::Running
        );
        SubagentSession::steer_child(&id, "focus on assertions".into()).unwrap();
        llm.release.notify_one();
        let outcome = tokio::time::timeout(Duration::from_secs(3), done_rx)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(outcome.summary_text(), "guided");
        assert_eq!(
            SubagentSession::child_status(&id).unwrap(),
            SubagentStatus::Finished {
                outcome: "done".into()
            }
        );
        assert!(SubagentSession::steer_child(&id, "late guidance".into()).is_err());
        let seen = llm.seen.lock().unwrap();
        assert!(
            !seen[0]
                .iter()
                .any(|m| m.content.contains("focus on assertions"))
        );
        assert!(
            seen[1]
                .iter()
                .any(|m| m.content.contains("[Supervisor Steering Guidance]")
                    && m.content.contains("focus on assertions"))
        );
        drop(seen);
        let mut applied = false;
        while let Ok(event) = events_rx.try_recv() {
            if matches!(event, SubagentEvent::SteeringApplied { messages: 1 }) {
                applied = true;
            }
            if let SubagentEvent::Finished { outcome } = event {
                assert_eq!(outcome.summary_text(), "guided");
                assert!(applied, "steering application must be observable");
                return;
            }
        }
        panic!("terminal event was not delivered");
    }
}
