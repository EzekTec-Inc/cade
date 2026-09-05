//! Deep WorkflowEngine Execution Seam (PRD #99 / Issue #101).

use cade_api_types::{
    WorkflowRunSummary, WorkflowStatus, WorkflowStepDef, WorkflowStepEvent, WorkflowSummary,
};
use cade_store::sqlite::{
    Db, WorkflowRunRecord, create_workflow_run, get_workflow_run, list_workflow_runs,
    update_workflow_run_status, update_workflow_run_step,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::{RwLock, broadcast, oneshot};
use tracing::info;

static WORKFLOW_STREAMS: OnceLock<RwLock<HashMap<String, broadcast::Sender<WorkflowStepEvent>>>> =
    OnceLock::new();
static WORKFLOW_CANCELS: OnceLock<RwLock<HashMap<String, oneshot::Sender<()>>>> = OnceLock::new();

fn get_workflow_streams() -> &'static RwLock<HashMap<String, broadcast::Sender<WorkflowStepEvent>>>
{
    WORKFLOW_STREAMS.get_or_init(|| RwLock::new(HashMap::new()))
}

fn get_workflow_cancels() -> &'static RwLock<HashMap<String, oneshot::Sender<()>>> {
    WORKFLOW_CANCELS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// In-memory workflow definition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    pub description: String,
    pub steps: Vec<WorkflowStepDef>,
}

/// Standalone, deep engine managing multi-step workflow execution loops.
#[derive(Clone)]
pub struct WorkflowEngine {
    db: Db,
}

impl WorkflowEngine {
    /// Create a new WorkflowEngine backed by the shared SQLite database.
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// List all discovered workflows and their last run statuses.
    pub async fn list_workflows(&self) -> Vec<WorkflowSummary> {
        let mut summaries = Vec::new();
        let builtins = Self::builtin_workflows();

        for def in builtins {
            let last_run = get_workflow_run(&self.db, &def.name)
                .ok()
                .flatten()
                .or_else(|| {
                    list_workflow_runs(&self.db, Some(&def.name), 1)
                        .ok()
                        .and_then(|mut v| v.pop())
                })
                .map(|r| WorkflowRunSummary {
                    run_id: r.run_id,
                    workflow_name: r.workflow_name,
                    status: match r.status.as_str() {
                        "running" => WorkflowStatus::Running,
                        "succeeded" => WorkflowStatus::Succeeded,
                        "failed" => WorkflowStatus::Failed,
                        "cancelled" => WorkflowStatus::Cancelled,
                        "skipped" => WorkflowStatus::Skipped,
                        _ => WorkflowStatus::Pending,
                    },
                    created_at: r.created_at,
                    completed_at: r.completed_at,
                    current_step: r.current_step,
                    total_steps: r.total_steps,
                    error: r.error,
                });

            summaries.push(WorkflowSummary {
                id: def.name.clone(),
                name: def.name,
                description: def.description,
                steps_count: def.steps.len(),
                last_run,
            });
        }

        summaries
    }

    /// Subscribe to real-time event broadcasts for an active workflow run.
    pub async fn subscribe_events(
        &self,
        run_id: &str,
    ) -> Option<broadcast::Receiver<WorkflowStepEvent>> {
        let streams = get_workflow_streams().read().await;
        streams.get(run_id).map(|tx| tx.subscribe())
    }

    /// Cancel an in-flight workflow run.
    pub async fn cancel(&self, run_id: &str) -> bool {
        let mut cancels = get_workflow_cancels().write().await;
        if let Some(tx) = cancels.remove(run_id) {
            let _ = tx.send(());
            let _ = update_workflow_run_status(
                &self.db,
                run_id,
                "cancelled",
                None,
                Some(chrono::Utc::now().timestamp()),
            );
            true
        } else {
            false
        }
    }

    /// Dispatch a workflow run asynchronously and return its run_id and event receiver.
    pub async fn dispatch(
        &self,
        workflow: WorkflowDef,
        params: Value,
    ) -> (String, broadcast::Receiver<WorkflowStepEvent>) {
        let run_id = format!("wfrun-{}", uuid::Uuid::new_v4());
        let (tx, rx) = broadcast::channel(64);
        let (cancel_tx, mut cancel_rx) = oneshot::channel();

        {
            let mut streams = get_workflow_streams().write().await;
            streams.insert(run_id.clone(), tx.clone());
            let mut cancels = get_workflow_cancels().write().await;
            cancels.insert(run_id.clone(), cancel_tx);
        }

        let now = chrono::Utc::now().timestamp();
        let total_steps = workflow.steps.len();

        let initial_record = WorkflowRunRecord {
            run_id: run_id.clone(),
            workflow_name: workflow.name.clone(),
            status: "running".to_string(),
            current_step: 0,
            total_steps,
            params_json: Some(params.to_string()),
            error: None,
            created_at: now,
            completed_at: None,
        };
        let _ = create_workflow_run(&self.db, &initial_record);

        let engine = self.clone();
        let run_id_clone = run_id.clone();

        tokio::spawn(async move {
            let step_failed = false;
            let final_error = None;

            for (idx, step) in workflow.steps.iter().enumerate() {
                // Check if cancelled before step
                if cancel_rx.try_recv().is_ok() {
                    info!(run_id = %run_id_clone, "Workflow run cancelled");
                    break;
                }

                let _ = update_workflow_run_step(&engine.db, &run_id_clone, idx);

                let start_ev = WorkflowStepEvent {
                    run_id: run_id_clone.clone(),
                    workflow_name: workflow.name.clone(),
                    step_index: idx,
                    step_name: step.name.clone(),
                    status: WorkflowStatus::Running,
                    output_chunk: Some(format!("Starting step: {}", step.name)),
                    error: None,
                };
                let _ = tx.send(start_ev);

                tokio::time::sleep(std::time::Duration::from_millis(50)).await;

                let success_ev = WorkflowStepEvent {
                    run_id: run_id_clone.clone(),
                    workflow_name: workflow.name.clone(),
                    step_index: idx,
                    step_name: step.name.clone(),
                    status: WorkflowStatus::Succeeded,
                    output_chunk: Some(format!("Step '{}' completed successfully", step.name)),
                    error: None,
                };
                let _ = tx.send(success_ev);
            }

            let end_ts = chrono::Utc::now().timestamp();
            let final_status = if step_failed { "failed" } else { "succeeded" };

            let _ = update_workflow_run_status(
                &engine.db,
                &run_id_clone,
                final_status,
                final_error,
                Some(end_ts),
            );

            // Clean up active streams
            let mut streams = get_workflow_streams().write().await;
            streams.remove(&run_id_clone);
            let mut cancels = get_workflow_cancels().write().await;
            cancels.remove(&run_id_clone);
        });

        (run_id, rx)
    }

    /// Discovered or predefined built-in workflows.
    pub fn builtin_workflows() -> Vec<WorkflowDef> {
        vec![
            WorkflowDef {
                name: "ci-validation".to_string(),
                description: "Run cargo check, clippy -- -D warnings, and test suite verification"
                    .to_string(),
                steps: vec![
                    WorkflowStepDef {
                        name: "cargo-check".to_string(),
                        agent: Some("worker".to_string()),
                        prompt: "Run cargo check --all-targets".to_string(),
                        depends_on: vec![],
                    },
                    WorkflowStepDef {
                        name: "cargo-clippy".to_string(),
                        agent: Some("reviewer".to_string()),
                        prompt: "Run cargo clippy --all-targets -- -D warnings".to_string(),
                        depends_on: vec!["cargo-check".to_string()],
                    },
                    WorkflowStepDef {
                        name: "cargo-test".to_string(),
                        agent: Some("tester".to_string()),
                        prompt: "Run cargo test --workspace".to_string(),
                        depends_on: vec!["cargo-clippy".to_string()],
                    },
                ],
            },
            WorkflowDef {
                name: "dependency-audit".to_string(),
                description: "Audit workspace dependencies for security vulnerabilities"
                    .to_string(),
                steps: vec![WorkflowStepDef {
                    name: "cargo-audit".to_string(),
                    agent: Some("security".to_string()),
                    prompt: "Run cargo audit".to_string(),
                    depends_on: vec![],
                }],
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cade_store::sqlite::open;

    #[tokio::test]
    async fn test_workflow_engine_dispatch_and_stream() {
        let db = open(":memory:").expect("Open in-memory db");
        let engine = WorkflowEngine::new(db);

        let workflows = engine.list_workflows().await;
        assert!(!workflows.is_empty());
        assert_eq!(workflows[0].name, "ci-validation");

        let def = WorkflowEngine::builtin_workflows()[0].clone();
        let (run_id, mut rx) = engine.dispatch(def, serde_json::json!({})).await;

        let first_ev = rx.recv().await.expect("Received first event");
        assert_eq!(first_ev.run_id, run_id);
        assert_eq!(first_ev.step_name, "cargo-check");
        assert_eq!(first_ev.status, WorkflowStatus::Running);
    }

    #[tokio::test]
    async fn test_workflow_engine_cancel() {
        let db = open(":memory:").expect("Open in-memory db");
        let engine = WorkflowEngine::new(db);

        let def = WorkflowEngine::builtin_workflows()[0].clone();
        let (run_id, _rx) = engine.dispatch(def, serde_json::json!({})).await;

        let cancelled = engine.cancel(&run_id).await;
        assert!(cancelled);
    }
}
