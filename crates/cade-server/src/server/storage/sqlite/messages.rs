use super::*;

pub fn last_assistant_message(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> Result<Option<MessageRow>> {
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;

    let sql = if conversation_id.is_some() {
        "SELECT id, agent_id, conversation_id, role, content, char_count FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'assistant'
         ORDER BY created_at DESC, rowid DESC LIMIT 1"
    } else {
        "SELECT id, agent_id, conversation_id, role, content, char_count FROM messages
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
        let char_count: i64 = r.get(5)?;
        Ok(Some(MessageRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            conversation_id: r.get(2)?,
            role: r.get(3)?,
            content,
            char_count: char_count as usize,
        }))
    } else {
        Ok(None)
    }
}

pub fn insert_message(db: &Db, row: &MessageRow) -> Result<()> {
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    conn.execute(
        "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at, char_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            row.id,
            row.agent_id,
            row.conversation_id,
            row.role,
            row.content.to_string(),
            now_ts(),
            row.char_count as i64
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
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    // Filter: conversation_id IS NULL for legacy messages, or matches given id.
    let sql = if conversation_id.is_some() {
        "SELECT id, agent_id, conversation_id, role, content, char_count FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2
         ORDER BY created_at DESC, rowid DESC LIMIT ?3 OFFSET ?4"
    } else {
        "SELECT id, agent_id, conversation_id, role, content, char_count FROM messages
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
                row.get::<_, i64>(5)?,
            ))
        },
    )?;
    let mut result: Vec<MessageRow> = rows
        .filter_map(|r| r.ok())
        .map(
            |(id, agent_id, conversation_id, role, content, char_count)| MessageRow {
                id,
                agent_id,
                conversation_id,
                role,
                content: serde_json::from_str(&content).unwrap_or(Value::String(content)),
                char_count: char_count as usize,
            },
        )
        .collect();
    // list_messages historically returned oldest-first; keep that invariant here
    result.reverse();
    Ok(result)
}

/// Fetch messages backwards until the cumulative char_count exceeds the budget.
/// This offloads context assembly math into SQLite using a window function.
pub fn get_context_window(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
    char_budget: usize,
) -> Result<Vec<MessageRow>> {
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let sql = if conversation_id.is_some() {
        "WITH ranked AS (
             SELECT id, agent_id, conversation_id, role, content, char_count, created_at, rowid,
                    SUM(char_count) OVER (ORDER BY created_at DESC, rowid DESC) as running_total
             FROM messages
             WHERE agent_id = ?1 AND conversation_id = ?2
         )
         SELECT id, agent_id, conversation_id, role, content, char_count
         FROM ranked
         WHERE running_total - char_count <= ?3
         ORDER BY created_at DESC, rowid DESC"
    } else {
        "WITH ranked AS (
             SELECT id, agent_id, conversation_id, role, content, char_count, created_at, rowid,
                    SUM(char_count) OVER (ORDER BY created_at DESC, rowid DESC) as running_total
             FROM messages
             WHERE agent_id = ?1 AND conversation_id IS NULL
         )
         SELECT id, agent_id, conversation_id, role, content, char_count
         FROM ranked
         WHERE running_total - char_count <= ?3
         ORDER BY created_at DESC, rowid DESC"
    };

    let mut stmt = conn.prepare(sql)?;
    let conv_placeholder = conversation_id.unwrap_or("");
    let rows = stmt.query_map(
        params![agent_id, conv_placeholder, char_budget as i64],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;

    let mut result: Vec<MessageRow> = rows
        .filter_map(|r| r.ok())
        .map(
            |(id, agent_id, conversation_id, role, content, char_count)| MessageRow {
                id,
                agent_id,
                conversation_id,
                role,
                content: serde_json::from_str(&content).unwrap_or(Value::String(content)),
                char_count: char_count as usize,
            },
        )
        .collect();

    // The query returns newest-first because of ORDER BY ... DESC.
    // The calling code expects oldest-first.
    result.reverse();
    Ok(result)
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;
    use serde_json::json;

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
    fn test_insert_and_list_messages() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        insert_message(
            &db,
            &MessageRow {
                id: "m1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!("hello"),
                char_count: 5,
            },
        )?;
        insert_message(
            &db,
            &MessageRow {
                id: "m2".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "assistant".into(),
                content: json!("hi there"),
                char_count: 8,
            },
        )?;

        let msgs = list_messages(&db, "a1", None, 10)?;
        assert_eq!(msgs.len(), 2);
        Ok(())
    }

    #[test]
    fn test_last_assistant_message() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // No messages yet
        let last = last_assistant_message(&db, "a1", None)?;
        assert!(last.is_none());

        insert_message(
            &db,
            &MessageRow {
                id: "m1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!("hello"),
                char_count: 5,
            },
        )?;
        insert_message(
            &db,
            &MessageRow {
                id: "m2".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "assistant".into(),
                content: json!("response"),
                char_count: 8,
            },
        )?;

        let last = last_assistant_message(&db, "a1", None)?;
        assert!(last.is_some());
        assert_eq!(last.unwrap().id, "m2");
        Ok(())
    }

    #[test]
    fn test_list_messages_page() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        for i in 0..5 {
            insert_message(
                &db,
                &MessageRow {
                    id: format!("m{i}"),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: "user".into(),
                    content: json!(format!("msg {i}")),
                    char_count: 5,
                },
            )?;
        }

        let page1 = list_messages_page(&db, "a1", None, 2, 0)?;
        assert_eq!(page1.len(), 2);

        let page2 = list_messages_page(&db, "a1", None, 2, 2)?;
        assert_eq!(page2.len(), 2);

        let page3 = list_messages_page(&db, "a1", None, 2, 4)?;
        assert_eq!(page3.len(), 1);
        Ok(())
    }

    #[test]
    fn test_get_context_window() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Insert messages with known char_count
        for i in 0..10 {
            insert_message(
                &db,
                &MessageRow {
                    id: format!("m{i}"),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                    content: json!(format!("message number {i}")),
                    char_count: 20,
                },
            )?;
        }

        // Large budget → all messages
        let all = get_context_window(&db, "a1", None, 999_999)?;
        assert_eq!(all.len(), 10);

        // Tiny budget → only the most recent messages
        let few = get_context_window(&db, "a1", None, 50)?;
        assert!(few.len() < 10);
        assert!(!few.is_empty());
        Ok(())
    }
}

// endregion: --- Tests
