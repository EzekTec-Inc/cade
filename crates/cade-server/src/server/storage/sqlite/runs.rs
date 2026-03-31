use super::*;

pub fn create_run(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<RunRow> {
    let id = format!("run-{}", uuid::Uuid::new_v4());
    let ts = now_ts();
    let conn = db.lock().expect("db lock poisoned");
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
    let conn = db.lock().expect("db lock poisoned");
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
    let conn = db.lock().expect("db lock poisoned");
    conn.execute(
        "UPDATE runs SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status, now_ts(), run_id],
    )?;
    Ok(())
}

/// Append an SSE event payload to the run's event log.
/// Returns the assigned seq_id.
pub fn append_run_event(db: &Db, run_id: &str, data: &str) -> Result<i64> {
    let conn = db.lock().expect("db lock poisoned");
    // Find current max seq_id for this run
    let max_seq: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq_id), -1) FROM run_events WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap_or(-1);
    let next_seq = max_seq + 1;
    conn.execute(
        "INSERT INTO run_events (run_id, seq_id, data) VALUES (?1, ?2, ?3)",
        params![run_id, next_seq, data],
    )?;
    Ok(next_seq)
}

/// Load run events after a given seq_id (exclusive).
pub fn run_events_after(db: &Db, run_id: &str, after_seq: i64) -> Result<Vec<(i64, String)>> {
    let conn = db.lock().expect("db lock poisoned");
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
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        apply_schema(&conn)?;
        run_migrations(&conn)?;
        Ok(Arc::new(Mutex::new(conn)))
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
