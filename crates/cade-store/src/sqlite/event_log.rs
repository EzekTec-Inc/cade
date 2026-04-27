use super::{now_ts, Result, Db};
use rusqlite::params;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EventLogEntry {
    pub id: String,
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub event_type: String,
    pub content: String,
    pub created_at: i64,
}

pub fn insert_event(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
    event_type: &str,
    content: &str,
) -> Result<String> {
    let id = format!("ev_{}", uuid::Uuid::new_v4().to_string().replace("-", ""));
    let created_at = now_ts();

    let conn = db.lock();
    let mut stmt = conn.prepare_cached(
        r#"
        INSERT INTO event_log (id, agent_id, conversation_id, event_type, content, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )?;

    stmt.execute(params![
        &id,
        agent_id,
        conversation_id,
        event_type,
        content,
        created_at,
    ])?;

    // FTS
    let mut fts_stmt = conn.prepare_cached(
        r#"
        INSERT INTO event_log_fts (id, agent_id, event_type, content, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )?;
    let _ = fts_stmt.execute(params![&id, agent_id, event_type, content, created_at]);

    Ok(id)
}

pub fn query_event_log(
    db: &Db,
    agent_id: &str,
    keyword: &str,
    limit: usize,
) -> Result<Vec<EventLogEntry>> {
    let conn = db.lock();
    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, agent_id, conversation_id, event_type, content, created_at
        FROM event_log
        WHERE agent_id = ?1 AND id IN (
            SELECT id FROM event_log_fts WHERE content MATCH ?2
        )
        ORDER BY created_at DESC
        LIMIT ?3
        "#,
    )?;

    let iter = stmt.query_map(params![agent_id, keyword, limit], |row| {
        Ok(EventLogEntry {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            conversation_id: row.get(2)?,
            event_type: row.get(3)?,
            content: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;

    let mut results = Vec::new();
    for entry in iter {
        results.push(entry?);
    }
    Ok(results)
}

pub fn list_recent_events(
    db: &Db,
    agent_id: &str,
    limit: usize,
) -> Result<Vec<EventLogEntry>> {
    let conn = db.lock();
    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, agent_id, conversation_id, event_type, content, created_at
        FROM event_log
        WHERE agent_id = ?1
        ORDER BY created_at DESC
        LIMIT ?2
        "#,
    )?;

    let iter = stmt.query_map(params![agent_id, limit], |row| {
        Ok(EventLogEntry {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            conversation_id: row.get(2)?,
            event_type: row.get(3)?,
            content: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;

    let mut results = Vec::new();
    for entry in iter {
        results.push(entry?);
    }
    Ok(results)
}

