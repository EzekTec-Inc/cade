//! Unified CapabilityMesh Execution Seam (ADR-0020).
//!
//! Provides a single, deep abstraction between agent reasoning loops and all
//! executable capabilities (built-in Rust tools, external MCP servers, declarative skills).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Tagged tool schema representation for LLM context injection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaggedCapabilitySchema {
    pub schema: Value,
    pub tags: Vec<String>,
}

/// Intent to invoke a capability through the mesh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityIntent {
    pub tool_call_id: String,
    pub capability_name: String,
    pub arguments: Value,
    pub caller_id: Option<String>,
}

/// Standardized output envelope returned by capability execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilityOutput {
    pub tool_call_id: String,
    pub capability_name: String,
    pub output: String,
    pub is_error: bool,
    pub ui_resource_uri: Option<String>,
}

/// Real-time event emitted during capability execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CapabilityEvent {
    Progress {
        tool_call_id: String,
        progress: f64,
        total: Option<f64>,
        message: Option<String>,
    },
    Telemetry {
        tool_call_id: String,
        event_name: String,
        payload: Value,
    },
}

/// Execution error types returned by the CapabilityMesh.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ExecutionError {
    NotFound(String),
    PermissionDenied(String, String),
    Disconnected(String, String),
    ExecutionFailed(String, String),
    Protocol(String),
}

impl std::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(name) => write!(f, "Capability '{name}' not found or unconfigured"),
            Self::PermissionDenied(name, reason) => {
                write!(f, "Permission denied for capability '{name}': {reason}")
            }
            Self::Disconnected(name, reason) => {
                write!(f, "Capability provider '{name}' disconnected: {reason}")
            }
            Self::ExecutionFailed(name, reason) => {
                write!(f, "Execution failed for capability '{name}': {reason}")
            }
            Self::Protocol(msg) => write!(f, "Capability protocol error: {msg}"),
        }
    }
}

impl std::error::Error for ExecutionError {}

/// Execution context passed to capability invocations.
pub struct CapabilityExecutionContext {
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub progress_tx: Option<tokio::sync::mpsc::Sender<CapabilityEvent>>,
}

impl CapabilityExecutionContext {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            conversation_id: None,
            progress_tx: None,
        }
    }

    pub fn with_conversation_id(mut self, conv_id: impl Into<String>) -> Self {
        self.conversation_id = Some(conv_id.into());
        self
    }

    pub fn with_progress_tx(mut self, tx: tokio::sync::mpsc::Sender<CapabilityEvent>) -> Self {
        self.progress_tx = Some(tx);
        self
    }
}

/// The unified deep seam for capability execution, schema injection, and lifecycle management.
#[async_trait]
pub trait CapabilityMesh: Send + Sync {
    /// Execute an action through the mesh.
    async fn execute(
        &self,
        intent: CapabilityIntent,
        cx: &mut CapabilityExecutionContext,
    ) -> Result<CapabilityOutput, ExecutionError>;

    /// Returns the active capability catalog formatted for LLM schema injection.
    async fn active_catalog(&self, cx: &CapabilityExecutionContext) -> Vec<TaggedCapabilitySchema>;
}