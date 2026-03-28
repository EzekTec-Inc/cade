use super::*;

pub fn create_conversation(db: &Db, agent_id: &str, title: &str) -> Result<ConversationRow> {
    let id = format!("conv-{}", uuid::Uuid::new_v4());
    let ts = now_ts();
    let conn = db.lock().expect("db lock poisoned");
    conn.execute(
        "INSERT INTO conversations (id, agent_id, title, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, agent_id, title, ts, ts],
    )?;
    Ok(ConversationRow {
        id,
        agent_id: agent_id.to_string(),
        title: title.to_string(),
        created_at: ts,
        updated_at: ts,
        message_count: 0,
    })
}

pub fn get_conversation(db: &Db, conv_id: &str) -> Result<Option<ConversationRow>> {
    let conn = db.lock().expect("db lock poisoned");
    let mut stmt = conn.prepare(
        "SELECT c.id, c.agent_id, c.title, c.created_at, c.updated_at,
                COUNT(m.id) as message_count
         FROM conversations c
         LEFT JOIN messages m ON m.conversation_id = c.id
         WHERE c.id = ?1
         GROUP BY c.id",
    )?;
    let mut rows = stmt.query(params![conv_id])?;
    if let Some(r) = rows.next()? {
        Ok(Some(ConversationRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            title: r.get(2)?,
            created_at: r.get(3)?,
            updated_at: r.get(4)?,
            message_count: r.get(5)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn list_conversations(db: &Db, agent_id: &str) -> Result<Vec<ConversationRow>> {
    let conn = db.lock().expect("db lock poisoned");
    let mut stmt = conn.prepare(
        "SELECT c.id, c.agent_id, c.title, c.created_at, c.updated_at,
                COUNT(m.id) as message_count
         FROM conversations c
         LEFT JOIN messages m ON m.conversation_id = c.id
         WHERE c.agent_id = ?1
         GROUP BY c.id
         ORDER BY c.updated_at DESC",
    )?;
    let rows = stmt.query_map(params![agent_id], |r| {
        Ok(ConversationRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            title: r.get(2)?,
            created_at: r.get(3)?,
            updated_at: r.get(4)?,
            message_count: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn delete_conversation(db: &Db, conv_id: &str) -> Result<bool> {
    let conn = db.lock().expect("db lock poisoned");
    // CASCADE deletes the messages too
    let n = conn.execute("DELETE FROM conversations WHERE id = ?1", params![conv_id])?;
    // Also clean up orphaned messages (fallback for rows without FK enforcement)
    let _ = conn.execute(
        "DELETE FROM messages WHERE conversation_id = ?1",
        params![conv_id],
    );
    Ok(n > 0)
}

/// Update the conversation's title and bump updated_at.
pub fn update_conversation_title(db: &Db, conv_id: &str, title: &str) -> Result<()> {
    let conn = db.lock().expect("db lock poisoned");
    conn.execute(
        "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
        params![title, now_ts(), conv_id],
    )?;
    Ok(())
}

/// Touch updated_at (called when a new message is added to a conversation).
pub fn touch_conversation(db: &Db, conv_id: &str) -> Result<()> {
    let conn = db.lock().expect("db lock poisoned");
    conn.execute(
        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
        params![now_ts(), conv_id],
    )?;
    Ok(())
}

// -- Runs (background mode)

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunRow {
    pub id: String,
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub status: String, // "running" | "completed" | "failed"
    pub created_at: i64,
    pub updated_at: i64,
}

