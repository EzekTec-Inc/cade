use super::*;

pub fn last_assistant_message(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> Result<Option<MessageRow>> {
    let conn = db.lock().expect("db lock poisoned");

    let sql = if conversation_id.is_some() {
        "SELECT id, agent_id, conversation_id, role, content FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'assistant'
         ORDER BY created_at DESC, rowid DESC LIMIT 1"
    } else {
        "SELECT id, agent_id, conversation_id, role, content FROM messages
         WHERE agent_id = ?1 AND conversation_id IS NULL AND role = 'assistant'
         ORDER BY created_at DESC, rowid DESC LIMIT 1"
    };

    let mut stmt = conn.prepare(sql)?;
    let mut rows = if let Some(cid) = conversation_id {
        stmt.query(params![agent_id, cid])?
    } else {
        stmt.query(params![agent_id])?
    };

    if let Some(r) = rows.next()? {
        let content_str: String = r.get(4)?;
        let content: Value = serde_json::from_str(&content_str).unwrap_or(Value::Null);
        Ok(Some(MessageRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            conversation_id: r.get(2)?,
            role: r.get(3)?,
            content,
        }))
    } else {
        Ok(None)
    }
}

pub fn insert_message(db: &Db, row: &MessageRow) -> Result<()> {
    let conn = db.lock().expect("db lock poisoned");
    conn.execute(
        "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            row.id,
            row.agent_id,
            row.conversation_id,
            row.role,
            row.content.to_string(),
            now_ts()
        ],
    )?;
    Ok(())
}

/// Load the last `limit` messages for an agent (or a specific conversation), oldest-first.
/// If `conversation_id` is None → load messages with NULL conversation_id (legacy/default).
/// Pass `Some("")` for the stub "all messages" mode — but we don't use that; always filter.
pub fn list_messages(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
    limit: usize,
) -> Result<Vec<MessageRow>> {
    list_messages_page(db, agent_id, conversation_id, limit, 0)
}

/// Page through messages with limit/offset, newest-first at the SQL level,
/// returned oldest-first for convenience.
pub fn list_messages_page(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<Vec<MessageRow>> {
    let conn = db.lock().expect("db lock poisoned");
    // Filter: conversation_id IS NULL for legacy messages, or matches given id.
    let sql = if conversation_id.is_some() {
        "SELECT id, agent_id, conversation_id, role, content FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2
         ORDER BY created_at DESC, rowid DESC LIMIT ?3 OFFSET ?4"
    } else {
        "SELECT id, agent_id, conversation_id, role, content FROM messages
         WHERE agent_id = ?1 AND conversation_id IS NULL
         ORDER BY created_at DESC, rowid DESC LIMIT ?3 OFFSET ?4"
    };

    let mut stmt = conn.prepare(sql)?;
    let conv_placeholder = conversation_id.unwrap_or("");
    let rows = stmt.query_map(
        params![agent_id, conv_placeholder, limit as i64, offset as i64],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    )?;
    let mut result: Vec<MessageRow> = rows
        .filter_map(|r| r.ok())
        .map(
            |(id, agent_id, conversation_id, role, content)| MessageRow {
                id,
                agent_id,
                conversation_id,
                role,
                content: serde_json::from_str(&content).unwrap_or(Value::String(content)),
            },
        )
        .collect();
    // list_messages historically returned oldest-first; keep that invariant here
    result.reverse();
    Ok(result)
}

