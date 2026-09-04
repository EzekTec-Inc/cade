//! SQLite persistence accessors for Workflows and Execution Runs (PRD #99 / Issue #100).

use crate::error::Result;
use crate::sqlite::Db;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// Database representation of a workflow execution run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowRunRecord {
    pub run_id: String,
    pub workflow_name: String,
    pub status: String,
    pub current_step: usize,
    pub total_steps: usize,
    pub params_json: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

/// Create a new workflow run entry in the database.
pub fn create_workflow_run(db: &Db, run: &WorkflowRunRecord) -> Result<()> {
    let conn = db.get()?;
    conn.execute(
        "INSERT INTO workflow_runs (
            run_id, workflow_name, status, current_step, total_steps, params_json, error, created_at, completed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            run.run_id,
            run.workflow_name,
            run.status,
            run.current_step as i64,
            run.total_steps as i64,
            run.params_json,
            run.error,
            run.created_at,
            run.completed_at,
        ],
    )?;
    Ok(())
}

/// Update status, error, and completion timestamp of a workflow run.
pub fn update_workflow_run_status(
    db: &Db,
    run_id: &str,
    status: &str,
    error: Option<&str>,
    completed_at: Option<i64>,
) -> Result<()> {
    let conn = db.get()?;
    conn.execute(
        "UPDATE workflow_runs
         SET status = ?1, error = ?2, completed_at = ?3
         WHERE run_id = ?4",
        params![status, error, completed_at, run_id],
    )?;
    Ok(())
}

/// Update the current active step index of a workflow run.
pub fn update_workflow_run_step(db: &Db, run_id: &str, current_step: usize) -> Result<()> {
    let conn = db.get()?;
    conn.execute(
        "UPDATE workflow_runs
         SET current_step = ?1
         WHERE run_id = ?2",
        params![current_step as i64, run_id],
    )?;
    Ok(())
}

/// Fetch a workflow run record by its run ID.
pub fn get_workflow_run(db: &Db, run_id: &str) -> Result<Option<WorkflowRunRecord>> {
    let conn = db.get()?;
    let row = conn
        .query_row(
            "SELECT run_id, workflow_name, status, current_step, total_steps, params_json, error, created_at, completed_at
             FROM workflow_runs
             WHERE run_id = ?1",
            params![run_id],
            |r| {
                let current_step_i64: i64 = r.get(3)?;
                let total_steps_i64: i64 = r.get(4)?;
                Ok(WorkflowRunRecord {
                    run_id: r.get(0)?,
                    workflow_name: r.get(1)?,
                    status: r.get(2)?,
                    current_step: current_step_i64 as usize,
                    total_steps: total_steps_i64 as usize,
                    params_json: r.get(5)?,
                    error: r.get(6)?,
                    created_at: r.get(7)?,
                    completed_at: r.get(8)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// List recent workflow runs, optionally filtered by workflow name.
pub fn list_workflow_runs(
    db: &Db,
    workflow_name: Option<&str>,
    limit: usize,
) -> Result<Vec<WorkflowRunRecord>> {
    let conn = db.get()?;
    let mut stmt = if let Some(name) = workflow_name {
        let mut s = conn.prepare(
            "SELECT run_id, workflow_name, status, current_step, total_steps, params_json, error, created_at, completed_at
             FROM workflow_runs
             WHERE workflow_name = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;
        let rows = s
            .query_map(params![name, limit as i64], |r| {
                let current_step_i64: i64 = r.get(3)?;
                let total_steps_i64: i64 = r.get(4)?;
                Ok(WorkflowRunRecord {
                    run_id: r.get(0)?,
                    workflow_name: r.get(1)?,
                    status: r.get(2)?,
                    current_step: current_step_i64 as usize,
                    total_steps: total_steps_i64 as usize,
                    params_json: r.get(5)?,
                    error: r.get(6)?,
                    created_at: r.get(7)?,
                    completed_at: r.get(8)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        return Ok(rows);
    } else {
        conn.prepare(
            "SELECT run_id, workflow_name, status, current_step, total_steps, params_json, error, created_at, completed_at
             FROM workflow_runs
             ORDER BY created_at DESC
             LIMIT ?1",
        )?
    };

    let rows = stmt
        .query_map(params![limit as i64], |r| {
            let current_step_i64: i64 = r.get(3)?;
            let total_steps_i64: i64 = r.get(4)?;
            Ok(WorkflowRunRecord {
                run_id: r.get(0)?,
                workflow_name: r.get(1)?,
                status: r.get(2)?,
                current_step: current_step_i64 as usize,
                total_steps: total_steps_i64 as usize,
                params_json: r.get(5)?,
                error: r.get(6)?,
                created_at: r.get(7)?,
                completed_at: r.get(8)?,
            })
        })?
        .filter_map(std::result::Result::ok)
        .collect();

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open;

    #[test]
    fn test_workflow_runs_crud() -> Result<()> {
        let db = open(":memory:")?;

        let run = WorkflowRunRecord {
            run_id: "test-run-1".to_string(),
            workflow_name: "qa-test".to_string(),
            status: "running".to_string(),
            current_step: 0,
            total_steps: 2,
            params_json: Some("{\"env\":\"staging\"}".to_string()),
            error: None,
            created_at: 1724500000,
            completed_at: None,
        };

        create_workflow_run(&db, &run)?;

        let fetched = get_workflow_run(&db, "test-run-1")?.expect("Workflow run found");
        assert_eq!(fetched.workflow_name, "qa-test");
        assert_eq!(fetched.status, "running");

        update_workflow_run_step(&db, "test-run-1", 1)?;
        let step_updated = get_workflow_run(&db, "test-run-1")?.unwrap();
        assert_eq!(step_updated.current_step, 1);

        update_workflow_run_status(&db, "test-run-1", "succeeded", None, Some(1724500100))?;
        let completed = get_workflow_run(&db, "test-run-1")?.unwrap();
        assert_eq!(completed.status, "succeeded");
        assert_eq!(completed.completed_at, Some(1724500100));

        let list = list_workflow_runs(&db, Some("qa-test"), 10)?;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].run_id, "test-run-1");

        Ok(())
    }
}
