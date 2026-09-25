use crate::error::Result;
use crate::sqlite::Db;
use rusqlite::params;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingApproval {
    pub id: String,
    pub agent_id: String,
    pub subagent_id: Option<String>,
    pub tool_name: String,
    pub arguments: String,
    pub reason: Option<String>,
    pub status: String, // "pending", "approved", "denied"
    pub created_at: i64,
}

pub fn create_pending_approval(
    db: &Db,
    id: &str,
    agent_id: &str,
    subagent_id: Option<&str>,
    tool_name: &str,
    arguments: &str,
) -> Result<()> {
    let conn = db.get()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT INTO pending_approvals (id, agent_id, subagent_id, tool_name, arguments, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6)",
        params![id, agent_id, subagent_id, tool_name, arguments, now],
    )?;

    Ok(())
}

/// The reviewed invocation and its durable run-stream payload.
pub struct RunApproval<'a> {
    pub id: &'a str,
    pub agent_id: &'a str,
    pub run_id: &'a str,
    pub tool_name: &'a str,
    pub arguments: &'a str,
    pub reason: &'a str,
    pub event: &'a str,
}

/// Create an actionable approval and its replayable run event together. If
/// either insert fails, no pending request can be exposed to a client.
pub fn create_run_approval(db: &Db, approval: &RunApproval<'_>) -> Result<i64> {
    let mut conn = db.get()?;
    let tx = conn.transaction()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    tx.execute(
        "INSERT INTO pending_approvals (id, agent_id, subagent_id, tool_name, arguments, status, created_at, reason)
         VALUES (?1, ?2, NULL, ?3, ?4, 'pending', ?5, ?6)",
        params![approval.id, approval.agent_id, approval.tool_name, approval.arguments, now, approval.reason],
    )?;
    let seq = tx.query_row(
        "INSERT INTO run_events (run_id, seq_id, data)
         VALUES (?1, (SELECT COALESCE(MAX(seq_id), -1) + 1 FROM run_events WHERE run_id = ?1), ?2)
         RETURNING seq_id",
        params![approval.run_id, approval.event],
        |row| row.get(0),
    )?;
    tx.commit()?;
    Ok(seq)
}

pub fn get_approval_status(db: &Db, id: &str) -> Result<Option<String>> {
    let conn = db.get()?;
    let mut stmt = conn.prepare("SELECT status FROM pending_approvals WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;

    if let Some(r) = rows.next() {
        Ok(Some(r?))
    } else {
        Ok(None)
    }
}

pub fn set_approval_status(db: &Db, id: &str, status: &str) -> Result<()> {
    let conn = db.get()?;
    conn.execute(
        "UPDATE pending_approvals SET status = ?2 WHERE id = ?1",
        params![id, status],
    )?;
    Ok(())
}

/// Only the first decision wins; a late response cannot resurrect a timed-out
/// or cancelled request or approve a tool that has already been denied.
pub fn resolve_pending_approval(db: &Db, id: &str, status: &str) -> Result<bool> {
    let conn = db.get()?;
    Ok(conn.execute(
        "UPDATE pending_approvals SET status = ?2 WHERE id = ?1 AND status = 'pending'",
        params![id, status],
    )? == 1)
}

pub fn list_pending_approvals(db: &Db) -> Result<Vec<PendingApproval>> {
    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, subagent_id, tool_name, arguments, status, created_at, reason
         FROM pending_approvals WHERE status = 'pending' ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(PendingApproval {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            subagent_id: row.get(2)?,
            tool_name: row.get(3)?,
            arguments: row.get(4)?,
            status: row.get(5)?,
            created_at: row.get(6)?,
            reason: row.get(7)?,
        })
    })?;

    let mut list = Vec::new();
    for r in rows {
        list.push(r?);
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open;

    #[test]
    fn test_pending_approvals_roundtrip() -> Result<()> {
        let db = open(":memory:")?;

        create_pending_approval(
            &db,
            "app-1",
            "agent-1",
            Some("sub-1"),
            "bash",
            "{\"command\": \"cargo build\"}",
        )?;

        let status_opt = get_approval_status(&db, "app-1")?;
        assert_eq!(status_opt.as_deref(), Some("pending"));

        let pending = list_pending_approvals(&db)?;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "app-1");
        assert_eq!(pending[0].tool_name, "bash");

        set_approval_status(&db, "app-1", "approved")?;
        let status_opt = get_approval_status(&db, "app-1")?;
        assert_eq!(status_opt.as_deref(), Some("approved"));

        let pending_after = list_pending_approvals(&db)?;
        assert_eq!(pending_after.len(), 0);

        Ok(())
    }

    #[test]
    fn run_approval_restores_reviewed_reason_and_arguments() -> Result<()> {
        let db = open(":memory:")?;
        crate::sqlite::create_agent(
            &db,
            &crate::sqlite::AgentRow {
                id: "agent".into(),
                name: "Agent".into(),
                model: "test".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
                active_plan_json: None,
                parent_id: None,
            },
        )?;
        let run = crate::sqlite::create_run(&db, "agent", None)?;
        create_run_approval(
            &db,
            &RunApproval {
                id: "app-1",
                agent_id: "agent",
                run_id: &run.id,
                tool_name: "write_file",
                arguments: r#"{"path":"a.txt"}"#,
                reason: "Write requires approval",
                event: r#"{"message_type":"approval_required"}"#,
            },
        )?;
        let rows = list_pending_approvals(&db)?;
        assert_eq!(rows[0].reason.as_deref(), Some("Write requires approval"));
        assert_eq!(rows[0].arguments, r#"{"path":"a.txt"}"#);
        Ok(())
    }

    #[test]
    fn existing_pending_rows_survive_reason_migration() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("old.db");
        {
            let conn = rusqlite::Connection::open(&path)?;
            conn.execute_batch("CREATE TABLE pending_approvals (
                id TEXT PRIMARY KEY, agent_id TEXT NOT NULL, subagent_id TEXT,
                tool_name TEXT NOT NULL, arguments TEXT NOT NULL, status TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            INSERT INTO pending_approvals VALUES ('old', 'agent', NULL, 'write_file', '{}', 'pending', 1);
            PRAGMA user_version = 21;")?;
        }
        let db = open(path.to_str().expect("db path"))?;
        let rows = list_pending_approvals(&db)?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "old");
        assert_eq!(rows[0].reason, None);
        Ok(())
    }
}
