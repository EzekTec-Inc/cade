use super::*;

pub fn create_conversation(db: &Db, agent_id: &str, title: &str) -> Result<ConversationRow> {
    let id = format!("conv-{}", uuid::Uuid::new_v4());
    let ts = now_ts();
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
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
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT c.id, c.agent_id, c.title, c.created_at, c.updated_at,
                COUNT(m.id) as message_count
         FROM conversations c
         LEFT JOIN messages m ON m.conversation_id = c.id OR (m.conversation_id IS NULL AND c.id = '')
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
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT c.id, c.agent_id, c.title, c.created_at, c.updated_at,
                COUNT(m.id) as message_count
         FROM conversations c
         LEFT JOIN messages m ON m.conversation_id = c.id OR (m.conversation_id IS NULL AND c.id = '')
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
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
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
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    conn.execute(
        "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
        params![title, now_ts(), conv_id],
    )?;
    Ok(())
}

/// Touch updated_at (called when a new message is added to a conversation).
pub fn touch_conversation(db: &Db, conv_id: &str) -> Result<()> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
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
                compaction_model: None, theme: None,
            },
        )?;
        Ok(())
    }

    #[test]
    fn test_create_and_get_conversation() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;
        let conv = create_conversation(&db, "a1", "My Chat")?;
        assert_eq!(conv.agent_id, "a1");
        assert_eq!(conv.title, "My Chat");

        let got = get_conversation(&db, &conv.id)?.expect("should exist");
        assert_eq!(got.id, conv.id);
        assert_eq!(got.title, "My Chat");
        Ok(())
    }

    #[test]
    fn test_get_conversation_not_found() -> Result<()> {
        let db = setup_mem_db()?;
        assert!(get_conversation(&db, "nope")?.is_none());
        Ok(())
    }

    #[test]
    fn test_list_conversations() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;
        make_agent(&db, "a2")?;
        assert!(list_conversations(&db, "a1")?.is_empty());
        create_conversation(&db, "a1", "C1")?;
        create_conversation(&db, "a1", "C2")?;
        create_conversation(&db, "a2", "C3")?;
        assert_eq!(list_conversations(&db, "a1")?.len(), 2);
        assert_eq!(list_conversations(&db, "a2")?.len(), 1);
        Ok(())
    }

    #[test]
    fn test_delete_conversation() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;
        let conv = create_conversation(&db, "a1", "C1")?;
        assert!(delete_conversation(&db, &conv.id)?);
        assert!(get_conversation(&db, &conv.id)?.is_none());
        assert!(!delete_conversation(&db, "nope")?);
        Ok(())
    }

    #[test]
    fn test_update_conversation_title() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;
        let conv = create_conversation(&db, "a1", "Old")?;
        update_conversation_title(&db, &conv.id, "New")?;
        let got = get_conversation(&db, &conv.id)?.unwrap();
        assert_eq!(got.title, "New");
        Ok(())
    }

    #[test]
    fn test_touch_conversation() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;
        let conv = create_conversation(&db, "a1", "C1")?;
        let before = get_conversation(&db, &conv.id)?.unwrap().updated_at;
        // Sleep a tiny bit so timestamp advances
        std::thread::sleep(std::time::Duration::from_millis(10));
        touch_conversation(&db, &conv.id)?;
        let after = get_conversation(&db, &conv.id)?.unwrap().updated_at;
        assert!(after >= before);
        Ok(())
    }
}

// endregion: --- Tests
