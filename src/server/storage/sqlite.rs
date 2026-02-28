use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// Thread-safe SQLite handle
pub type Db = Arc<Mutex<Connection>>;

pub fn open(path: &str) -> Result<Db> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let conn = Connection::open(path)
        .with_context(|| format!("open SQLite at {path}"))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    apply_schema(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

fn apply_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS agents (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            model       TEXT NOT NULL,
            description TEXT,
            system_prompt TEXT,
            created_at  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS messages (
            id          TEXT PRIMARY KEY,
            agent_id    TEXT NOT NULL,
            role        TEXT NOT NULL,
            content     TEXT NOT NULL,
            created_at  INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS memory_blocks (
            id          TEXT PRIMARY KEY,
            agent_id    TEXT NOT NULL,
            label       TEXT NOT NULL,
            value       TEXT NOT NULL DEFAULT '',
            updated_at  INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS tools (
            id          TEXT PRIMARY KEY,
            name        TEXT UNIQUE NOT NULL,
            description TEXT,
            source_code TEXT,
            json_schema TEXT,
            tags        TEXT DEFAULT '[]',
            created_at  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS agent_tools (
            agent_id TEXT NOT NULL,
            tool_id  TEXT NOT NULL,
            PRIMARY KEY (agent_id, tool_id),
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
            FOREIGN KEY (tool_id)  REFERENCES tools(id)  ON DELETE CASCADE
        );
    "#)?;
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ── Agents ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub model: String,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
}

pub fn create_agent(db: &Db, row: &AgentRow) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO agents (id, name, model, description, system_prompt, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![row.id, row.name, row.model, row.description, row.system_prompt, now_ts()],
    )?;
    Ok(())
}

pub fn get_agent(db: &Db, id: &str) -> Result<Option<AgentRow>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, name, model, description, system_prompt FROM agents WHERE id = ?1"
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(AgentRow {
            id:            row.get(0)?,
            name:          row.get(1)?,
            model:         row.get(2)?,
            description:   row.get(3)?,
            system_prompt: row.get(4)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn list_agents(db: &Db) -> Result<Vec<AgentRow>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, name, model, description, system_prompt FROM agents ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(AgentRow {
            id:            row.get(0)?,
            name:          row.get(1)?,
            model:         row.get(2)?,
            description:   row.get(3)?,
            system_prompt: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn delete_agent(db: &Db, id: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    let n = conn.execute("DELETE FROM agents WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageRow {
    pub id: String,
    pub agent_id: String,
    pub role: String,
    pub content: Value,
}

pub fn insert_message(db: &Db, row: &MessageRow) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO messages (id, agent_id, role, content, created_at) VALUES (?1,?2,?3,?4,?5)",
        params![row.id, row.agent_id, row.role, row.content.to_string(), now_ts()],
    )?;
    Ok(())
}

/// Load the last `limit` messages for an agent (oldest-first order)
pub fn list_messages(db: &Db, agent_id: &str, limit: usize) -> Result<Vec<MessageRow>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, role, content FROM messages
         WHERE agent_id = ?1
         ORDER BY created_at DESC LIMIT ?2"
    )?;
    let rows = stmt.query_map(params![agent_id, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut result: Vec<MessageRow> = rows
        .filter_map(|r| r.ok())
        .map(|(id, agent_id, role, content)| MessageRow {
            id,
            agent_id,
            role,
            content: serde_json::from_str(&content).unwrap_or(Value::String(content)),
        })
        .collect();
    result.reverse(); // return oldest-first
    Ok(result)
}

// ── Memory blocks ─────────────────────────────────────────────────────────────

pub fn upsert_memory_block(db: &Db, agent_id: &str, label: &str, value: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO memory_blocks (id, agent_id, label, value, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (agent_id) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![id, agent_id, label, value, now_ts()],
    )?;
    Ok(())
}

pub fn get_memory_blocks(db: &Db, agent_id: &str) -> Result<Vec<(String, String)>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT label, value FROM memory_blocks WHERE agent_id = ?1 ORDER BY label"
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ── Tools ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub source_code: Option<String>,
    pub json_schema: Option<Value>,
    pub tags: Vec<String>,
}

pub fn upsert_tool(db: &Db, row: &ToolRow) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO tools (id, name, description, source_code, json_schema, tags, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(name) DO UPDATE SET
           description = excluded.description,
           source_code = excluded.source_code,
           json_schema = excluded.json_schema,
           tags = excluded.tags",
        params![
            row.id,
            row.name,
            row.description,
            row.source_code,
            row.json_schema.as_ref().map(|v| v.to_string()),
            serde_json::to_string(&row.tags).unwrap_or_default(),
            now_ts()
        ],
    )?;
    Ok(())
}

pub fn list_tools(db: &Db) -> Result<Vec<ToolRow>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, name, description, source_code, json_schema, tags FROM tools ORDER BY name"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    Ok(rows
        .filter_map(|r| r.ok())
        .map(|(id, name, description, source_code, schema_str, tags_str)| ToolRow {
            id,
            name,
            description,
            source_code,
            json_schema: schema_str.and_then(|s| serde_json::from_str(&s).ok()),
            tags: serde_json::from_str(&tags_str).unwrap_or_default(),
        })
        .collect())
}
