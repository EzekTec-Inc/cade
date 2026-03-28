use super::*;

pub fn create_agent(db: &Db, row: &AgentRow) -> Result<()> {
    let conn = db.lock().expect("db lock poisoned");
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
    let conn = db.lock().expect("db lock poisoned");
    let mut stmt = conn.prepare(
        "SELECT id, name, model, description, system_prompt, created_at FROM agents WHERE id = ?1",
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
        }))
    } else {
        Ok(None)
    }
}

pub fn list_agents(db: &Db) -> Result<Vec<AgentRow>> {
    let conn = db.lock().expect("db lock poisoned");
    let mut stmt = conn.prepare(
        "SELECT id, name, model, description, system_prompt, created_at FROM agents ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AgentRow {
            id: row.get(0)?,
            name: row.get(1)?,
            model: row.get(2)?,
            description: row.get(3)?,
            system_prompt: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn delete_agent(db: &Db, id: &str) -> Result<bool> {
    let conn = db.lock().expect("db lock poisoned");
    let n = conn.execute("DELETE FROM agents WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

/// Update the model used by an agent. Returns false if the agent was not found.
pub fn update_agent_model(db: &Db, id: &str, model: &str) -> Result<bool> {
    let conn = db.lock().expect("db lock poisoned");
    let n = conn.execute(
        "UPDATE agents SET model = ?1 WHERE id = ?2",
        params![model, id],
    )?;
    Ok(n > 0)
}

pub fn update_agent_name(db: &Db, id: &str, name: &str) -> Result<bool> {
    let conn = db.lock().expect("db lock poisoned");
    let n = conn.execute(
        "UPDATE agents SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    Ok(n > 0)
}

pub fn update_agent_system_prompt(db: &Db, id: &str, prompt: &str) -> Result<bool> {
    let conn = db.lock().expect("db lock poisoned");
    let n = conn.execute(
        "UPDATE agents SET system_prompt = ?1 WHERE id = ?2",
        params![prompt, id],
    )?;
    Ok(n > 0)
}

/// Associate a set of tool IDs with an agent (upsert).
pub fn attach_tools_to_agent(db: &Db, agent_id: &str, tool_ids: &[String]) -> Result<()> {
    let conn = db.lock().expect("db lock poisoned");
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
    let conn = db.lock().expect("db lock poisoned");
    let mut stmt = conn.prepare("SELECT tool_id FROM agent_tools WHERE agent_id = ?1")?;
    let rows = stmt.query_map(params![agent_id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Return (tool_id, tool_name) pairs for all tools attached to an agent.
pub fn get_agent_tools_with_names(db: &Db, agent_id: &str) -> Result<Vec<(String, String)>> {
    let conn = db.lock().expect("db lock poisoned");
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
    let conn = db.lock().expect("db lock poisoned");
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

