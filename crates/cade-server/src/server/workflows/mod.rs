//! Workflow discovery, dependency scheduling and canonical agent-run orchestration.
//! Plugin lifecycle and tool routing live in their own modules.

mod definitions;
mod execution;
#[cfg(test)]
mod tests;

use crate::server::api::run::runtime::ServerAgentRuntime;
use crate::server::state::AppState;
use cade_api_types::{WorkflowRunSummary, WorkflowStatus, WorkflowStepEvent, WorkflowSummary};
use cade_store::sqlite::{self, Db, WorkflowRunRecord};
pub use definitions::{WorkflowConfig, WorkflowDef};
use std::path::PathBuf;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct WorkflowEngine {
    db: Db,
    runtime: ServerAgentRuntime,
    directory: PathBuf,
}

/// Accepted workflow and the first actual canonical agent execution. The legacy
/// webhook execution_id remains queryable through the ordinary agent-run API.
#[derive(Debug, Clone)]
pub struct WorkflowDispatch {
    pub run_id: String,
    pub execution_id: String,
    pub agent_id: String,
}

impl WorkflowEngine {
    pub fn new(state: AppState) -> Self {
        Self::with_runtime(state.db.clone(), ServerAgentRuntime::new(state))
    }

    /// Reuses the real runtime, including its injectable provider/context seams in tests.
    pub fn with_runtime(db: Db, runtime: ServerAgentRuntime) -> Self {
        Self {
            db,
            runtime,
            directory: PathBuf::from(".cade/workflows"),
        }
    }

    pub fn with_directory(mut self, directory: PathBuf) -> Self {
        self.directory = directory;
        self
    }

    pub async fn list_workflows(&self) -> Result<Vec<WorkflowSummary>, String> {
        let definitions = self.definitions()?;
        definitions
            .into_iter()
            .map(|def| {
                let last_run = sqlite::list_workflow_runs(&self.db, Some(&def.name), 1)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .next()
                    .map(run_summary);
                Ok(WorkflowSummary {
                    id: def.name.clone(),
                    name: def.name,
                    description: def.description,
                    steps_count: def.steps.len(),
                    steps: def.steps,
                    last_run,
                })
            })
            .collect()
    }

    pub async fn subscribe_events(
        &self,
        run_id: &str,
    ) -> Option<broadcast::Receiver<WorkflowStepEvent>> {
        sqlite::get_workflow_run(&self.db, run_id).ok().flatten()?;
        execution::subscribe(run_id).await
    }

    /// Cancellation is a request; the engine records terminal cancellation after
    /// the active canonical runtime run has stopped, keeping its receiver alive.
    pub async fn cancel(&self, run_id: &str) -> Result<bool, String> {
        if sqlite::get_workflow_run(&self.db, run_id)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Ok(false);
        }
        execution::cancel(&self.db, run_id).await
    }

    pub async fn dispatch(
        &self,
        workflow: WorkflowDef,
        params: serde_json::Value,
    ) -> Result<(String, broadcast::Receiver<WorkflowStepEvent>), String> {
        let (accepted, events) = self.dispatch_with_execution(workflow, params).await?;
        Ok((accepted.run_id, events))
    }

    /// Wait for canonical runtime acceptance, never for provider completion.
    pub async fn dispatch_with_execution(
        &self,
        workflow: WorkflowDef,
        params: serde_json::Value,
    ) -> Result<(WorkflowDispatch, broadcast::Receiver<WorkflowStepEvent>), String> {
        execution::dispatch(self.clone(), workflow, params).await
    }
}

pub fn run_summary(record: WorkflowRunRecord) -> WorkflowRunSummary {
    WorkflowRunSummary {
        run_id: record.run_id,
        workflow_name: record.workflow_name,
        status: match record.status.as_str() {
            "running" | "cancelling" => WorkflowStatus::Running,
            "succeeded" => WorkflowStatus::Succeeded,
            "failed" => WorkflowStatus::Failed,
            "cancelled" => WorkflowStatus::Cancelled,
            "skipped" => WorkflowStatus::Skipped,
            _ => WorkflowStatus::Pending,
        },
        created_at: record.created_at,
        completed_at: record.completed_at,
        current_step: record.current_step,
        total_steps: record.total_steps,
        error: record.error,
    }
}
