use super::*;

pub fn create_run(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<RunRow> {
    let id = format!("run-{}", uuid::Uuid::new_v4());
    let ts = now_ts();
    let conn = db.get()?;
    conn.execute(
        "INSERT INTO runs (id, agent_id, conversation_id, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'running', ?4, ?5)",
        params![id, agent_id, conversation_id, ts, ts],
    )?;
    Ok(RunRow {
        id,
        agent_id: agent_id.to_string(),
        conversation_id: conversation_id.map(String::from),
        status: "running".to_string(),
        created_at: ts,
        updated_at: ts,
    })
}

pub fn get_run(db: &Db, run_id: &str) -> Result<Option<RunRow>> {
    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, conversation_id, status, created_at, updated_at
         FROM runs WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![run_id])?;
    if let Some(r) = rows.next()? {
        Ok(Some(RunRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            conversation_id: r.get(2)?,
            status: r.get(3)?,
            created_at: r.get(4)?,
            updated_at: r.get(5)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn finish_run(db: &Db, run_id: &str, status: &str) -> Result<()> {
    let conn = db.get()?;
    conn.execute(
        "UPDATE runs SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status, now_ts(), run_id],
    )?;
    Ok(())
}

/// Commit an active Run's terminal status and journal event together. The
/// returned cursor is publishable only after commit; an existing terminal Run
/// is left alone. Missing Runs and ignored writes are errors, not completion.
pub fn finish_run_with_event(
    db: &Db,
    run_id: &str,
    status: &str,
    data: &str,
) -> Result<Option<i64>> {
    if !matches!(status, "done" | "error" | "cancelled") {
        return Err(crate::error::Error::custom("Run outcome must be terminal"));
    }
    let mut conn = db.get()?;
    let tx = conn.transaction()?;
    if !set_active_run_terminal_status(&tx, run_id, status)? {
        return Ok(None);
    }
    let sequence = tx.query_row(
        "INSERT INTO run_events (run_id, seq_id, data)
         VALUES (?1, (SELECT COALESCE(MAX(seq_id), -1) + 1 FROM run_events WHERE run_id = ?1), ?2)
         RETURNING seq_id",
        params![run_id, data],
        |row| row.get(0),
    )?;
    tx.commit()?;
    Ok(Some(sequence))
}

/// Last-resort failure state when terminal journaling is unavailable. Never
/// converts an already-terminal Run or claims a journal entry was persisted.
pub fn mark_run_failed(db: &Db, run_id: &str) -> Result<bool> {
    let mut conn = db.get()?;
    let tx = conn.transaction()?;
    let changed = set_active_run_terminal_status(&tx, run_id, "error")?;
    tx.commit()?;
    Ok(changed)
}

fn set_active_run_terminal_status(
    conn: &rusqlite::Connection,
    run_id: &str,
    status: &str,
) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE runs SET status = ?1, updated_at = ?2
         WHERE id = ?3 AND status IN ('running', 'cancelling')",
        params![status, now_ts(), run_id],
    )?;
    let stored: String = conn.query_row(
        "SELECT status FROM runs WHERE id = ?1",
        params![run_id],
        |row| row.get(0),
    )?;
    if changed == 0 {
        if matches!(stored.as_str(), "done" | "error" | "cancelled") {
            return Ok(false);
        }
        return Err(crate::error::Error::custom(
            "Run terminal status was not written",
        ));
    }
    if stored != status {
        return Err(crate::error::Error::custom(
            "Run terminal status was not retained",
        ));
    }
    Ok(true)
}

/// Request cancellation for an active run without overwriting a terminal outcome.
///
/// Returns `true` when a running run was transitioned to `cancelling`.
pub fn request_run_cancellation(db: &Db, run_id: &str) -> Result<bool> {
    let conn = db.get()?;
    let changed = conn.execute(
        "UPDATE runs SET status = 'cancelling', updated_at = ?1
         WHERE id = ?2 AND status = 'running'",
        params![now_ts(), run_id],
    )?;
    Ok(changed == 1)
}

/// Returns whether an active run has a durable cancellation request.
pub fn is_run_cancellation_requested(db: &Db, run_id: &str) -> Result<bool> {
    let conn = db.get()?;
    let status: Option<String> = conn
        .query_row(
            "SELECT status FROM runs WHERE id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(status.as_deref() == Some("cancelling"))
}

/// List recent runs for an agent ordered by created_at DESC.
pub fn list_agent_runs(db: &Db, agent_id: &str, limit: usize) -> Result<Vec<RunRow>> {
    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, conversation_id, status, created_at, updated_at
         FROM runs WHERE agent_id = ?1
         ORDER BY created_at DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![agent_id, limit as i64], |r| {
        Ok(RunRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            conversation_id: r.get(2)?,
            status: r.get(3)?,
            created_at: r.get(4)?,
            updated_at: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Append an SSE event payload to the run's event log.
/// Returns the assigned seq_id.
pub fn append_run_event(db: &Db, run_id: &str, data: &str) -> Result<i64> {
    let conn = db.get()?;

    let next_seq: i64 = conn.query_row(
        "INSERT INTO run_events (run_id, seq_id, data)
         VALUES (?1, (SELECT COALESCE(MAX(seq_id), -1) + 1 FROM run_events WHERE run_id = ?1), ?2)
         RETURNING seq_id",
        params![run_id, data],
        |r| r.get(0),
    )?;

    Ok(next_seq)
}

/// Load run events after a given seq_id (exclusive).
pub fn run_events_after(db: &Db, run_id: &str, after_seq: i64) -> Result<Vec<(i64, String)>> {
    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT seq_id, data FROM run_events
         WHERE run_id = ?1 AND seq_id > ?2
         ORDER BY seq_id ASC",
    )?;
    let rows = stmt.query_map(params![run_id, after_seq], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// -- Messages

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageRow {
    pub id: String,
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub role: String,
    pub content: Value,
    pub char_count: usize,
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;

    fn setup_mem_db() -> Result<Db> {
        Ok(super::open(":memory:")?)
    }

    fn make_agent(db: &Db, id: &str) -> Result<()> {
        agents::create_agent(
            db,
            &AgentRow {
                id: id.into(),
                name: "A".into(),
                model: "m".into(),
                description: None,
                system_prompt: None,
                created_at: None,
                compaction_model: None,
                theme: None,
                active_plan_json: None,
                parent_id: None,
            },
        )?;
        Ok(())
    }

    #[test]
    fn test_create_and_get_run() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;
        let run = create_run(&db, "a1", None)?;
        assert_eq!(run.agent_id, "a1");
        assert_eq!(run.status, "running");
        assert!(run.conversation_id.is_none());

        let got = get_run(&db, &run.id)?.expect("run should exist");
        assert_eq!(got.id, run.id);
        assert_eq!(got.status, "running");
        Ok(())
    }

    #[test]
    fn test_get_run_not_found() -> Result<()> {
        let db = setup_mem_db()?;
        assert!(get_run(&db, "nope")?.is_none());
        Ok(())
    }

    #[test]
    fn test_finish_run() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;
        let run = create_run(&db, "a1", None)?;
        finish_run(&db, &run.id, "completed")?;
        let got = get_run(&db, &run.id)?.unwrap();
        assert_eq!(got.status, "completed");
        Ok(())
    }

    #[test]
    fn terminal_commit_is_idempotent_and_cannot_rewrite_an_outcome() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;
        let run = create_run(&db, "a1", None)?;
        let data = r#"{"message_type":"run_done","status":"cancelled"}"#;
        assert_eq!(
            finish_run_with_event(&db, &run.id, "cancelled", data)?,
            Some(0)
        );
        assert_eq!(finish_run_with_event(&db, &run.id, "done", "unused")?, None);
        assert!(!mark_run_failed(&db, &run.id)?);
        assert_eq!(get_run(&db, &run.id)?.unwrap().status, "cancelled");
        assert_eq!(run_events_after(&db, &run.id, -1)?, vec![(0, data.into())]);
        assert!(finish_run_with_event(&db, "missing", "done", "unused").is_err());
        assert!(mark_run_failed(&db, "missing").is_err());
        Ok(())
    }

    #[test]
    fn test_append_and_get_run_events() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;
        let run = create_run(&db, "a1", None)?;

        let seq1 = append_run_event(&db, &run.id, "event one")?;
        let seq2 = append_run_event(&db, &run.id, "event two")?;
        let seq3 = append_run_event(&db, &run.id, "event three")?;
        assert!(seq2 > seq1);
        assert!(seq3 > seq2);

        // Get events after seq1 (exclusive — should return seq2 and seq3)
        let events = run_events_after(&db, &run.id, seq1)?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].1, "event two");
        assert_eq!(events[1].1, "event three");

        // Get all events (after seq -1 to include seq 0)
        let all = run_events_after(&db, &run.id, -1)?;
        assert_eq!(all.len(), 3);
        Ok(())
    }
}

// endregion: --- Tests
