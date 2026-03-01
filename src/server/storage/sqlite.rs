use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::sync::{Arc, Mutex};

// ── Provider row ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderRow {
    pub name:     String,
    pub kind:     String,          // "anthropic" | "openai" | "gemini" | "ollama" | "openai-compatible"
    pub api_key:  Option<String>,
    pub base_url: Option<String>,
    pub enabled:  bool,
}

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
    run_migrations(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// Idempotent migrations — run after apply_schema on every startup.
fn run_migrations(conn: &Connection) -> Result<()> {
    // Migration 1: add UNIQUE(agent_id, label) to memory_blocks if missing.
    // SQLite doesn't support ALTER TABLE ADD CONSTRAINT, so we rebuild the table.
    // Detect UNIQUE(agent_id, label) specifically.
    // Note: autoindices for PRIMARY KEY have sql=NULL — exclude them with sql IS NOT NULL.
    // A user-defined UNIQUE constraint generates an autoindex whose sql is also NULL,
    // so we check the index name pattern instead.
    let has_unique: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE tbl_name='memory_blocks'
         AND (
           (type='index' AND sql IS NOT NULL
            AND (sql LIKE '%agent_id%label%' OR sql LIKE '%label%agent_id%'))
           OR
           (type='table' AND sql LIKE '%UNIQUE%agent_id%label%')
         )",
        [],
        |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0;

    if !has_unique {
        tracing::info!("Running migration: adding UNIQUE(agent_id, label) to memory_blocks");
        conn.execute_batch(r#"
            BEGIN;
            CREATE TABLE IF NOT EXISTS memory_blocks_new (
                id         TEXT PRIMARY KEY,
                agent_id   TEXT NOT NULL,
                label      TEXT NOT NULL,
                value      TEXT NOT NULL DEFAULT '',
                updated_at INTEGER NOT NULL,
                UNIQUE (agent_id, label),
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
            );
            -- Copy keeping only the latest row per (agent_id, label)
            INSERT OR IGNORE INTO memory_blocks_new
                SELECT id, agent_id, label, value, updated_at FROM (
                    SELECT *, ROW_NUMBER() OVER (
                        PARTITION BY agent_id, label ORDER BY updated_at DESC
                    ) AS rn FROM memory_blocks
                ) WHERE rn = 1;
            DROP TABLE memory_blocks;
            ALTER TABLE memory_blocks_new RENAME TO memory_blocks;
            COMMIT;
        "#)?;
        tracing::info!("Migration complete: memory_blocks UNIQUE constraint added");
    }
    Ok(())
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
            UNIQUE (agent_id, label),
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

        CREATE TABLE IF NOT EXISTS providers (
            name       TEXT PRIMARY KEY,
            kind       TEXT NOT NULL,
            api_key    TEXT,
            base_url   TEXT,
            enabled    INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
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

/// Update the model used by an agent. Returns false if the agent was not found.
pub fn update_agent_model(db: &Db, id: &str, model: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    let n = conn.execute(
        "UPDATE agents SET model = ?1 WHERE id = ?2",
        params![model, id],
    )?;
    Ok(n > 0)
}

/// Associate a set of tool IDs with an agent (upsert).
pub fn attach_tools_to_agent(db: &Db, agent_id: &str, tool_ids: &[String]) -> Result<()> {
    let conn = db.lock().unwrap();
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
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT tool_id FROM agent_tools WHERE agent_id = ?1"
    )?;
    let rows = stmt.query_map(params![agent_id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
    // Use SELECT + UPDATE/INSERT instead of ON CONFLICT so this works regardless
    // of whether the UNIQUE(agent_id, label) constraint exists on the table.
    let existing: Option<String> = conn.query_row(
        "SELECT id FROM memory_blocks WHERE agent_id = ?1 AND label = ?2",
        params![agent_id, label],
        |r| r.get(0),
    ).optional()?;

    if existing.is_some() {
        conn.execute(
            "UPDATE memory_blocks SET value = ?1, updated_at = ?2
             WHERE agent_id = ?3 AND label = ?4",
            params![value, now_ts(), agent_id, label],
        )?;
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO memory_blocks (id, agent_id, label, value, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, agent_id, label, value, now_ts()],
        )?;
    }
    Ok(())
}

pub fn delete_memory_block(db: &Db, agent_id: &str, label: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    let n = conn.execute(
        "DELETE FROM memory_blocks WHERE agent_id = ?1 AND label = ?2",
        params![agent_id, label],
    )?;
    Ok(n > 0)
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

/// Check whether all tool results have been received for the current turn.
///
/// Returns `(received, expected)`.
/// - If not in a tool-call turn: (0, 0)
/// - If waiting for more results: received < expected
/// - If all received: received == expected
/// Delete all messages for an agent (context-window clear).
pub fn clear_messages(db: &Db, agent_id: &str) -> Result<usize> {
    let conn = db.lock().unwrap();
    let n = conn.execute(
        "DELETE FROM messages WHERE agent_id = ?1",
        params![agent_id],
    )?;
    Ok(n)
}

/// Full-text search over message content for an agent.
pub fn search_messages(db: &Db, agent_id: &str, query: &str) -> Result<Vec<MessageRow>> {
    let conn = db.lock().unwrap();
    let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, role, content FROM messages \
         WHERE agent_id = ?1 AND content LIKE ?2 ESCAPE '\\' \
         ORDER BY rowid DESC LIMIT 50",
    )?;
    let rows = stmt.query_map(params![agent_id, pattern], |r| {
        let content_str: String = r.get(3)?;
        let content = serde_json::from_str(&content_str)
            .unwrap_or(serde_json::Value::String(content_str));
        Ok(MessageRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            role: r.get(2)?,
            content,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn pending_tool_results(db: &Db, agent_id: &str) -> Result<(usize, usize)> {
    let messages = list_messages(db, agent_id, 20)?;

    let mut tool_results_received: usize = 0;
    let mut expected: usize = 0;

    // Walk backwards through recent messages
    for msg in messages.iter().rev() {
        match msg.role.as_str() {
            "tool" => {
                tool_results_received += 1;
            }
            "assistant" => {
                if let Some(arr) = msg.content["tool_calls"].as_array() {
                    let non_empty: Vec<_> = arr.iter()
                        .filter(|tc| tc.get("id").and_then(|v| v.as_str()).is_some())
                        .collect();
                    if !non_empty.is_empty() {
                        expected = non_empty.len();
                        break; // found the assistant turn that issued tool calls
                    }
                }
                // Assistant message without tool_calls = not in a tool-call turn
                break;
            }
            _ => break, // user/system — not in tool-call turn
        }
    }

    Ok((tool_results_received, expected))
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

// ── Providers ─────────────────────────────────────────────────────────────────

pub fn upsert_provider(db: &Db, row: &ProviderRow) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO providers (name, kind, api_key, base_url, enabled, created_at)
         VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(name) DO UPDATE SET
           kind    = excluded.kind,
           api_key = excluded.api_key,
           base_url= excluded.base_url,
           enabled = excluded.enabled",
        params![
            row.name,
            row.kind,
            row.api_key,
            row.base_url,
            row.enabled as i64,
            now_ts(),
        ],
    )?;
    Ok(())
}

pub fn list_providers(db: &Db) -> Result<Vec<ProviderRow>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT name, kind, api_key, base_url, enabled FROM providers ORDER BY name"
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ProviderRow {
            name:     r.get(0)?,
            kind:     r.get(1)?,
            api_key:  r.get(2)?,
            base_url: r.get(3)?,
            enabled:  r.get::<_, i64>(4)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn delete_provider(db: &Db, name: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    let n = conn.execute("DELETE FROM providers WHERE name = ?1", params![name])?;
    Ok(n > 0)
}
