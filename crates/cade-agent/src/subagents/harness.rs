//! Deep Subagent & Worktree Isolation Harness (`AgentHarness`).
//!
//! Encapsulates:
//! - Isolation policies (`InProcess`, `ReadOnly`, `WorktreeBranch`, `VirtualSandbox`, `Docker`)
//! - RAII cleanup and graceful teardown
//! - Background task tracking, signal cancellation, and timeout management
//! - Cross-platform platform probes and fallback cascades

// region:    --- Imports

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

use crate::backends::ExecutionBackend;
use crate::backends::local::LocalBackend;
use crate::backends::readonly::ReadOnlyBackend;
use crate::backends::virtual_sandbox::VirtualSandboxBackend;
use crate::subagents::workspace_guard::IsolatedWorkspaceGuard;
use crate::{Error, Result};

// endregion: --- Imports

// region:    --- Types

/// Isolation strategy for executing a subagent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq)]
pub enum IsolationPolicy {
    /// In-process local execution sharing the host environment.
    #[default]
    InProcess,
    /// Read-only sandbox restricting all write tools.
    ReadOnly,
    /// Isolated temporary git worktree with automatic teardown.
    WorktreeBranch { branch_name: Option<String> },
    /// In-memory copy-on-write virtual sandbox.
    VirtualSandbox,
    /// Containerized Docker execution.
    #[cfg(feature = "backend-docker")]
    Docker { image: String },
}

/// Specification for a task dispatched to the harness.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HarnessTaskSpec {
    pub subagent_id: String,
    pub parent_agent_id: String,
    pub prompt: String,
    pub isolation: IsolationPolicy,
    pub working_dir: PathBuf,
    pub timeout_secs: Option<u64>,
}

impl HarnessTaskSpec {
    pub fn new(
        subagent_id: impl Into<String>,
        parent_agent_id: impl Into<String>,
        prompt: impl Into<String>,
        working_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            subagent_id: subagent_id.into(),
            parent_agent_id: parent_agent_id.into(),
            prompt: prompt.into(),
            isolation: IsolationPolicy::default(),
            working_dir: working_dir.into(),
            timeout_secs: Some(300),
        }
    }

    pub fn with_isolation(mut self, isolation: IsolationPolicy) -> Self {
        self.isolation = isolation;
        self
    }

    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = Some(timeout_secs);
        self
    }
}

/// Lifecycle status of a managed harness task.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HarnessLifecycleState {
    Pending,
    Running,
    Completed { output: String, is_error: bool },
    TimedOut,
    Cancelled,
    Failed { error: String },
}

/// Active execution entry managed by the harness.
struct ActiveExecution {
    _spec: HarnessTaskSpec,
    state: HarnessLifecycleState,
    _worktree_guard: Option<IsolatedWorkspaceGuard>,
    cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

// endregion: --- Types

// region:    --- AgentHarness

/// Deep harness managing isolated subagent execution, workspaces, and lifecycles.
#[derive(Clone)]
pub struct AgentHarness {
    base_dir: PathBuf,
    executions: Arc<RwLock<HashMap<String, Arc<Mutex<ActiveExecution>>>>>,
}

impl AgentHarness {
    /// Construct a new AgentHarness instance.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            executions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Access the base working directory.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Prepare the execution backend and workspace according to the requested isolation policy.
    pub async fn prepare_backend(
        &self,
        spec: &HarnessTaskSpec,
    ) -> Result<(Arc<dyn ExecutionBackend>, Option<IsolatedWorkspaceGuard>)> {
        debug!(
            target: "cade_agent::harness",
            subagent_id = %spec.subagent_id,
            isolation = ?spec.isolation,
            "AgentHarness: preparing isolation backend"
        );

        match &spec.isolation {
            IsolationPolicy::InProcess => Ok((Arc::new(LocalBackend), None)),
            IsolationPolicy::ReadOnly => Ok((Arc::new(ReadOnlyBackend::new(LocalBackend)), None)),
            IsolationPolicy::VirtualSandbox => {
                let sandbox = VirtualSandboxBackend::new(spec.working_dir.clone());
                Ok((Arc::new(sandbox), None))
            }
            IsolationPolicy::WorktreeBranch { branch_name } => {
                let guard = IsolatedWorkspaceGuard::new(&spec.working_dir, branch_name.clone())
                    .await
                    .map_err(|e| Error::custom(format!("worktree preparation failed: {e}")))?;
                let backend = Arc::new(LocalBackend);
                Ok((backend, Some(guard)))
            }
            #[cfg(feature = "backend-docker")]
            IsolationPolicy::Docker { image } => {
                let docker = crate::backends::docker::DockerBackend::new(image.clone(), vec![]);
                Ok((Arc::new(docker), None))
            }
        }
    }

    /// Register a new subagent execution under harness supervision.
    pub async fn register(
        &self,
        spec: HarnessTaskSpec,
        worktree_guard: Option<IsolatedWorkspaceGuard>,
        cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<()> {
        let id = spec.subagent_id.clone();
        let entry = Arc::new(Mutex::new(ActiveExecution {
            _spec: spec,
            state: HarnessLifecycleState::Running,
            _worktree_guard: worktree_guard,
            cancel_tx,
        }));

        let mut execs = self.executions.write().await;
        execs.insert(id, entry);
        Ok(())
    }

    /// Update execution state upon completion or failure.
    pub async fn mark_completed(
        &self,
        subagent_id: &str,
        output: String,
        is_error: bool,
    ) -> Result<()> {
        let execs = self.executions.read().await;
        if let Some(entry) = execs.get(subagent_id) {
            let mut guard = entry.lock().await;
            guard.state = HarnessLifecycleState::Completed { output, is_error };
            info!(
                target: "cade_agent::harness",
                subagent_id = %subagent_id,
                is_error = is_error,
                "Harness task completed"
            );
        }
        Ok(())
    }

    /// Cancel a running execution by sending a cancellation signal.
    pub async fn cancel(&self, subagent_id: &str) -> Result<bool> {
        let execs = self.executions.read().await;
        if let Some(entry) = execs.get(subagent_id) {
            let mut guard = entry.lock().await;
            if let Some(tx) = guard.cancel_tx.take() {
                let _ = tx.send(());
            }
            guard.state = HarnessLifecycleState::Cancelled;
            warn!(
                target: "cade_agent::harness",
                subagent_id = %subagent_id,
                "Harness task cancelled"
            );
            return Ok(true);
        }
        Ok(false)
    }

    /// Poll current lifecycle status of an execution.
    pub async fn get_status(&self, subagent_id: &str) -> Option<HarnessLifecycleState> {
        let execs = self.executions.read().await;
        if let Some(entry) = execs.get(subagent_id) {
            let guard = entry.lock().await;
            return Some(guard.state.clone());
        }
        None
    }

    /// Clean up completed or stale execution records and release memory.
    pub async fn prune_completed(&self) -> usize {
        let mut execs = self.executions.write().await;
        let before = execs.len();
        let mut active = HashMap::new();

        for (k, v) in execs.drain() {
            let is_finished = {
                let guard = v.lock().await;
                matches!(
                    guard.state,
                    HarnessLifecycleState::Completed { .. }
                        | HarnessLifecycleState::TimedOut
                        | HarnessLifecycleState::Cancelled
                        | HarnessLifecycleState::Failed { .. }
                )
            };
            if !is_finished {
                active.insert(k, v);
            }
        }

        *execs = active;
        before.saturating_sub(execs.len())
    }
}

// endregion: --- AgentHarness

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_harness_in_process_preparation() -> Result<()> {
        let temp_dir = std::env::temp_dir();
        let harness = AgentHarness::new(&temp_dir);

        let spec = HarnessTaskSpec::new("sub-1", "parent-1", "Do work", &temp_dir)
            .with_isolation(IsolationPolicy::InProcess);

        let (backend, guard) = harness.prepare_backend(&spec).await?;
        assert!(guard.is_none());
        assert_eq!(backend.name(), "local");

        Ok(())
    }

    #[tokio::test]
    async fn test_harness_virtual_sandbox_preparation() -> Result<()> {
        let temp_dir = std::env::temp_dir();
        let harness = AgentHarness::new(&temp_dir);

        let spec = HarnessTaskSpec::new("sub-2", "parent-1", "Sandboxed task", &temp_dir)
            .with_isolation(IsolationPolicy::VirtualSandbox);

        let (backend, guard) = harness.prepare_backend(&spec).await?;
        assert!(guard.is_none());
        assert_eq!(backend.name(), "virtual_sandbox");

        Ok(())
    }

    #[tokio::test]
    async fn test_harness_lifecycle_and_cancellation() -> Result<()> {
        let temp_dir = std::env::temp_dir();
        let harness = AgentHarness::new(&temp_dir);

        let spec = HarnessTaskSpec::new("sub-3", "parent-1", "Async task", &temp_dir);
        let (tx, rx) = tokio::sync::oneshot::channel();

        harness.register(spec, None, Some(tx)).await?;

        let status = harness.get_status("sub-3").await;
        assert_eq!(status, Some(HarnessLifecycleState::Running));

        let cancelled = harness.cancel("sub-3").await?;
        assert!(cancelled);

        // Verify receiver caught cancellation signal
        let signal = rx.await;
        assert!(signal.is_ok());

        let status_after = harness.get_status("sub-3").await;
        assert_eq!(status_after, Some(HarnessLifecycleState::Cancelled));

        let pruned = harness.prune_completed().await;
        assert_eq!(pruned, 1);
        assert_eq!(harness.get_status("sub-3").await, None);

        Ok(())
    }
}

// endregion: --- Tests
