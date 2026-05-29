use crate::error::Result;
use crate::sqlite::Db;
use rusqlite::params;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunCheckpoint {
    pub id: String,
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub current_iteration: usize,
    pub serialized_state: String,
    pub updated_at: i64,
}

pub fn upsert_run_checkpoint(
    db: &Db,
    id: &str,
    agent_id: &str,
    conversation_id: Option<&str>,
    current_iteration: usize,
    serialized_state: &str,
) -> Result<()> {
    let conn = db.get()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    conn.execute(
        "INSERT INTO run_checkpoints (id, agent_id, conversation_id, current_iteration, serialized_state, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
             current_iteration = excluded.current_iteration,
             serialized_state = excluded.serialized_state,
             updated_at = excluded.updated_at",
        params![id, agent_id, conversation_id, current_iteration as i64, serialized_state, now],
    )?;

    Ok(())
}

pub fn get_run_checkpoint(db: &Db, id: &str) -> Result<Option<RunCheckpoint>> {
    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, conversation_id, current_iteration, serialized_state, updated_at
         FROM run_checkpoints WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(RunCheckpoint {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            conversation_id: row.get(2)?,
            current_iteration: row.get::<_, i64>(3)? as usize,
            serialized_state: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;

    if let Some(r) = rows.next() {
        Ok(Some(r?))
    } else {
        Ok(None)
    }
}

pub fn delete_run_checkpoint(db: &Db, id: &str) -> Result<()> {
    let conn = db.get()?;
    conn.execute("DELETE FROM run_checkpoints WHERE id = ?1", params![id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::open;

    #[test]
    fn test_run_checkpoints_roundtrip() -> Result<()> {
        let db = open(":memory:")?;
        
        upsert_run_checkpoint(&db, "run-1", "agent-1", Some("conv-1"), 2, "{\"state\": \"data\"}")?;
        
        let cp_opt = get_run_checkpoint(&db, "run-1")?;
        assert!(cp_opt.is_some());
        let cp = cp_opt.unwrap();
        assert_eq!(cp.agent_id, "agent-1");
        assert_eq!(cp.conversation_id.as_deref(), Some("conv-1"));
        assert_eq!(cp.current_iteration, 2);
        assert_eq!(cp.serialized_state, "{\"state\": \"data\"}");

        // Update it
        upsert_run_checkpoint(&db, "run-1", "agent-1", Some("conv-1"), 3, "{\"state\": \"updated\"}")?;
        let cp = get_run_checkpoint(&db, "run-1")?.unwrap();
        assert_eq!(cp.current_iteration, 3);
        assert_eq!(cp.serialized_state, "{\"state\": \"updated\"}");

        delete_run_checkpoint(&db, "run-1")?;
        let cp_opt = get_run_checkpoint(&db, "run-1")?;
        assert!(cp_opt.is_none());

        Ok(())
    }
}
