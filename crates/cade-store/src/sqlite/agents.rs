use super::*;

pub fn create_agent(db: &Db, row: &AgentRow) -> Result<()> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    conn.execute(
        "INSERT INTO agents (id, name, model, description, system_prompt, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            row.id,
            row.name,
            row.model,
            row.description,
            row.system_prompt,
            now_ts()
        ],
    )?;
    Ok(())
}

pub fn get_agent(db: &Db, id: &str) -> Result<Option<AgentRow>> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT id, name, model, description, system_prompt, created_at, compaction_model, theme FROM agents WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(AgentRow {
            id: row.get(0)?,
            name: row.get(1)?,
            model: row.get(2)?,
            description: row.get(3)?,
            system_prompt: row.get(4)?,
            created_at: row.get(5)?,
            compaction_model: row.get(6)?,
            theme: row.get(7)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn list_agents(db: &Db) -> Result<Vec<AgentRow>> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT id, name, model, description, system_prompt, created_at, compaction_model, theme FROM agents ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AgentRow {
            id: row.get(0)?,
            name: row.get(1)?,
            model: row.get(2)?,
            description: row.get(3)?,
            system_prompt: row.get(4)?,
            created_at: row.get(5)?,
            compaction_model: row.get(6)?,
            theme: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn delete_agent(db: &Db, id: &str) -> Result<bool> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let n = conn.execute("DELETE FROM agents WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

/// Update the model used by an agent. Returns false if the agent was not found.
pub fn update_agent_model(db: &Db, id: &str, model: &str) -> Result<bool> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let n = conn.execute(
        "UPDATE agents SET model = ?1 WHERE id = ?2",
        params![model, id],
    )?;
    Ok(n > 0)
}

pub fn update_agent_name(db: &Db, id: &str, name: &str) -> Result<bool> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let n = conn.execute(
        "UPDATE agents SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    Ok(n > 0)
}

pub fn update_agent_system_prompt(db: &Db, id: &str, prompt: &str) -> Result<bool> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let n = conn.execute(
        "UPDATE agents SET system_prompt = ?1 WHERE id = ?2",
        params![prompt, id],
    )?;
    Ok(n > 0)
}

/// Update the compaction (summarization) model for an agent.
/// Pass `None` to clear the override and fall back to the main model.
pub fn update_agent_compaction_model(db: &Db, id: &str, model: Option<&str>) -> Result<bool> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let n = conn.execute(
        "UPDATE agents SET compaction_model = ?1 WHERE id = ?2",
        params![model, id],
    )?;
    Ok(n > 0)
}

/// Persist the theme name for an agent (set by `/theme <name>`).
/// Pass `None` to clear the override and inherit the global setting.
pub fn update_agent_theme(db: &Db, id: &str, theme: Option<&str>) -> Result<bool> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let n = conn.execute(
        "UPDATE agents SET theme = ?1 WHERE id = ?2",
        params![theme, id],
    )?;
    Ok(n > 0)
}

/// Associate a set of tool IDs with an agent (upsert).
pub fn attach_tools_to_agent(db: &Db, agent_id: &str, tool_ids: &[String]) -> Result<()> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    for tid in tool_ids {
        conn.execute(
            "INSERT OR IGNORE INTO agent_tools (agent_id, tool_id) VALUES (?1, ?2)",
            params![agent_id, tid],
        )?;
    }
    Ok(())
}

/// Return tool IDs associated with an agent (if any; falls back to all tools).
pub fn get_agent_tool_ids(db: &Db, agent_id: &str) -> Result<Vec<String>> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare("SELECT tool_id FROM agent_tools WHERE agent_id = ?1")?;
    let rows = stmt.query_map(params![agent_id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Return (tool_id, tool_name) pairs for all tools attached to an agent.
pub fn get_agent_tools_with_names(db: &Db, agent_id: &str) -> Result<Vec<(String, String)>> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT at.tool_id, t.name FROM agent_tools at
         JOIN tools t ON t.id = at.tool_id
         WHERE at.agent_id = ?1
         ORDER BY t.name",
    )?;
    let rows = stmt.query_map(params![agent_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Detach ALL tools from an agent (clear agent_tools rows for this agent).
pub fn detach_all_tools_from_agent(db: &Db, agent_id: &str) -> Result<usize> {
    let conn = db
        .lock()
        .map_err(|e| crate::error::Error::custom(format!("db lock poisoned: {e}")))?;
    let n = conn.execute(
        "DELETE FROM agent_tools WHERE agent_id = ?1",
        params![agent_id],
    )?;
    Ok(n)
}

// -- Conversations

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationRow {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
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

    fn test_agent(id: &str) -> AgentRow {
        AgentRow {
            id: id.into(),
            name: format!("Agent {id}"),
            model: "test-model".into(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None, theme: None,
        }
    }

    #[test]
    fn test_create_and_get_agent() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;
        let got = get_agent(&db, "a1")?.expect("agent should exist");
        assert_eq!(got.id, "a1");
        assert_eq!(got.name, "Agent a1");
        assert_eq!(got.model, "test-model");
        assert!(got.created_at.is_some());
        Ok(())
    }

    #[test]
    fn test_get_agent_not_found() -> Result<()> {
        let db = setup_mem_db()?;
        assert!(get_agent(&db, "nope")?.is_none());
        Ok(())
    }

    #[test]
    fn test_list_agents_empty_and_populated() -> Result<()> {
        let db = setup_mem_db()?;
        assert!(list_agents(&db)?.is_empty());
        create_agent(&db, &test_agent("a1"))?;
        create_agent(&db, &test_agent("a2"))?;
        assert_eq!(list_agents(&db)?.len(), 2);
        Ok(())
    }

    #[test]
    fn test_delete_agent() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;
        assert!(delete_agent(&db, "a1")?);
        assert!(get_agent(&db, "a1")?.is_none());
        assert!(!delete_agent(&db, "nope")?);
        Ok(())
    }

    #[test]
    fn test_update_agent_model() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;
        assert!(update_agent_model(&db, "a1", "gpt-4o")?);
        assert_eq!(get_agent(&db, "a1")?.unwrap().model, "gpt-4o");
        assert!(!update_agent_model(&db, "nope", "x")?);
        Ok(())
    }

    #[test]
    fn test_update_agent_name() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;
        assert!(update_agent_name(&db, "a1", "Renamed")?);
        assert_eq!(get_agent(&db, "a1")?.unwrap().name, "Renamed");
        assert!(!update_agent_name(&db, "nope", "x")?);
        Ok(())
    }

    #[test]
    fn test_update_agent_system_prompt() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;
        assert!(update_agent_system_prompt(&db, "a1", "Be helpful")?);
        assert_eq!(
            get_agent(&db, "a1")?.unwrap().system_prompt,
            Some("Be helpful".into())
        );
        assert!(!update_agent_system_prompt(&db, "nope", "x")?);
        Ok(())
    }

    #[test]
    fn test_attach_and_get_agent_tools() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;
        // Create two tools via upsert_tool (from tools module, re-exported by super)
        upsert_tool(
            &db,
            &ToolRow {
                id: "t1".into(),
                name: "bash".into(),
                description: None,
                source_code: None,
                json_schema: None,
                tags: vec![],
            },
        )?;
        upsert_tool(
            &db,
            &ToolRow {
                id: "t2".into(),
                name: "grep".into(),
                description: None,
                source_code: None,
                json_schema: None,
                tags: vec![],
            },
        )?;
        attach_tools_to_agent(&db, "a1", &["t1".into(), "t2".into()])?;
        let ids = get_agent_tool_ids(&db, "a1")?;
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"t1".to_string()));
        assert!(ids.contains(&"t2".to_string()));
        Ok(())
    }

    #[test]
    fn update_agent_theme_round_trip() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("theme-1"))?;

        // Initially no theme
        let before = get_agent(&db, "theme-1")?.unwrap();
        assert_eq!(before.theme, None, "fresh agent must have no theme");

        // Set
        let changed = update_agent_theme(&db, "theme-1", Some("tokyo-night"))?;
        assert!(changed, "update_agent_theme must report change");

        let after = get_agent(&db, "theme-1")?.unwrap();
        assert_eq!(after.theme.as_deref(), Some("tokyo-night"));

        // Clear
        update_agent_theme(&db, "theme-1", None)?;
        let cleared = get_agent(&db, "theme-1")?.unwrap();
        assert_eq!(cleared.theme, None, "passing None must clear the theme");

        Ok(())
    }

    #[test]
    fn update_agent_theme_missing_agent_returns_false() -> Result<()> {
        let db = setup_mem_db()?;
        let changed = update_agent_theme(&db, "nope", Some("dark"))?;
        assert!(!changed, "update on missing agent must return false");
        Ok(())
    }

    #[test]
    fn test_get_agent_tools_with_names() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;
        upsert_tool(
            &db,
            &ToolRow {
                id: "t1".into(),
                name: "bash".into(),
                description: None,
                source_code: None,
                json_schema: None,
                tags: vec![],
            },
        )?;
        attach_tools_to_agent(&db, "a1", &["t1".into()])?;
        let names = get_agent_tools_with_names(&db, "a1")?;
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], ("t1".into(), "bash".into()));
        Ok(())
    }

    #[test]
    fn test_detach_all_tools() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;
        upsert_tool(
            &db,
            &ToolRow {
                id: "t1".into(),
                name: "bash".into(),
                description: None,
                source_code: None,
                json_schema: None,
                tags: vec![],
            },
        )?;
        attach_tools_to_agent(&db, "a1", &["t1".into()])?;
        assert_eq!(detach_all_tools_from_agent(&db, "a1")?, 1);
        assert!(get_agent_tool_ids(&db, "a1")?.is_empty());
        Ok(())
    }

    #[test]
    fn test_compaction_model_default_is_none() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;
        let agent = get_agent(&db, "a1")?.unwrap();
        assert!(agent.compaction_model.is_none());
        Ok(())
    }

    #[test]
    fn test_update_and_clear_compaction_model() -> Result<()> {
        let db = setup_mem_db()?;
        create_agent(&db, &test_agent("a1"))?;

        // Set compaction model
        assert!(update_agent_compaction_model(&db, "a1", Some("gpt-4o-mini"))?);
        let agent = get_agent(&db, "a1")?.unwrap();
        assert_eq!(agent.compaction_model.as_deref(), Some("gpt-4o-mini"));

        // Clear compaction model
        assert!(update_agent_compaction_model(&db, "a1", None)?);
        let agent = get_agent(&db, "a1")?.unwrap();
        assert!(agent.compaction_model.is_none());

        // Non-existent agent returns false
        assert!(!update_agent_compaction_model(&db, "nope", Some("x"))?);
        Ok(())
    }
}

// endregion: --- Tests
