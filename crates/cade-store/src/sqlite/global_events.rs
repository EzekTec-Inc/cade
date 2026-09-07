//! Global events storage for server-wide SSE event broadcasting and replay.

// region:    --- Imports

use super::*;
use crate::error::Result;

// endregion: --- Imports

// region:    --- Functions

/// Append a global event to the durable event log. Returns assigned `seq`.
pub fn append_global_event(db: &Db, event_type: &str, payload: &str) -> Result<i64> {
    let conn = db.get()?;
    let now = now_ts();

    let seq: i64 = conn.query_row(
        "INSERT INTO global_events (event_type, payload, created_at)
         VALUES (?1, ?2, ?3)
         RETURNING seq",
        params![event_type, payload, now],
        |r| r.get(0),
    )?;

    Ok(seq)
}

/// Load global events with `seq > after_seq`, ordered by `seq ASC`.
pub fn global_events_after(db: &Db, after_seq: i64) -> Result<Vec<(i64, String, String)>> {
    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT seq, event_type, payload FROM global_events
         WHERE seq > ?1
         ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map(params![after_seq], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// endregion: --- Functions

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_append_and_replay_global_events() -> Result<()> {
        let db = crate::sqlite::open(":memory:")?;

        let seq1 = append_global_event(&db, "run_started", r#"{"run_id":"run-1"}"#)?;
        let seq2 = append_global_event(&db, "tool_progress", r#"{"status":"started"}"#)?;
        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);

        let events = global_events_after(&db, 0)?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, 1);
        assert_eq!(events[0].1, "run_started");

        let events_since_1 = global_events_after(&db, 1)?;
        assert_eq!(events_since_1.len(), 1);
        assert_eq!(events_since_1[0].0, 2);
        assert_eq!(events_since_1[0].1, "tool_progress");

        Ok(())
    }
}

// endregion: --- Tests
