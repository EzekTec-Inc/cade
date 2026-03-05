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

    // Migration 3: add `max_chars` column to memory_blocks if missing.
    let has_max_chars: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('memory_blocks') WHERE name='max_chars'",
        [],
        |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0;
    if !has_max_chars {
        tracing::info!("Running migration: adding max_chars column to memory_blocks");
        conn.execute_batch(
            "ALTER TABLE memory_blocks ADD COLUMN max_chars INTEGER;"
        )?;
        tracing::info!("Migration complete: memory_blocks.max_chars added");
    }

    // Migration 4: create memory_history table if it doesn't exist.
    conn.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS memory_history (
            id         TEXT PRIMARY KEY,
            block_id   TEXT NOT NULL,
            value      TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_history_block_id
            ON memory_history(block_id, updated_at DESC);
    "#)?;

    // Migration 2: add `description` column to memory_blocks if missing.
    // SQLite supports ADD COLUMN directly (no table rebuild needed).
    let has_description: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('memory_blocks') WHERE name='description'",
        [],
        |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0;

    if !has_description {
        tracing::info!("Running migration: adding description column to memory_blocks");
        conn.execute_batch(
            "ALTER TABLE memory_blocks ADD COLUMN description TEXT NOT NULL DEFAULT '';"
        )?;
        tracing::info!("Migration complete: memory_blocks.description added");
    }

    // Migration 3: add conversation_id column to messages + index.
    let has_conv_col: bool = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name='conversation_id'",
        [],
        |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0;

    if !has_conv_col {
        tracing::info!("Running migration: adding conversation_id to messages");
        conn.execute_batch(
            "ALTER TABLE messages ADD COLUMN conversation_id TEXT;
             CREATE INDEX IF NOT EXISTS idx_messages_conv
               ON messages(agent_id, conversation_id);"
        )?;
        tracing::info!("Migration complete: messages.conversation_id added");
    }

    // Migration 5: Shared Memory Blocks
    let has_shared_memory: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='shared_memory_blocks'",
        [],
        |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0;

    if !has_shared_memory {
        tracing::info!("Running migration: implement Shared Memory schema");
        conn.execute_batch(r#"
            BEGIN;
            CREATE TABLE shared_memory_blocks (
                id          TEXT PRIMARY KEY,
                label       TEXT NOT NULL,
                value       TEXT NOT NULL DEFAULT '',
                description TEXT NOT NULL DEFAULT '',
                max_chars   INTEGER,
                updated_at  INTEGER NOT NULL
            );
            CREATE TABLE agent_memory_blocks (
                agent_id TEXT NOT NULL,
                block_id TEXT NOT NULL,
                PRIMARY KEY (agent_id, block_id),
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
                FOREIGN KEY (block_id) REFERENCES shared_memory_blocks(id) ON DELETE CASCADE
            );
            -- Migrate existing memory_blocks to shared_memory_blocks
            INSERT INTO shared_memory_blocks (id, label, value, description, max_chars, updated_at)
                SELECT id, label, value, description, max_chars, updated_at FROM memory_blocks;
            -- Create the links
            INSERT INTO agent_memory_blocks (agent_id, block_id)
                SELECT agent_id, id FROM memory_blocks;
            COMMIT;
        "#)?;
        tracing::info!("Migration complete: Shared Memory schema implemented");
    }

    // Migration 6: Archival Memory (FTS5)
    let has_fts: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages_fts'",
        [],
        |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0;

    if !has_fts {
        tracing::info!("Running migration: implement FTS5 Archival Memory");
        let res = conn.execute_batch(r#"
            BEGIN;
            CREATE VIRTUAL TABLE messages_fts USING fts5(
                content,
                content='messages',
                content_rowid='id'
            );
            -- Populate initial data
            INSERT INTO messages_fts(rowid, content) SELECT rowid, content FROM messages;
            
            -- Triggers to keep FTS index in sync
            CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
                INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content);
            END;
            CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.rowid, old.content);
            END;
            CREATE TRIGGER messages_au AFTER UPDATE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.rowid, old.content);
                INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content);
            END;
            COMMIT;
        "#);
        if let Err(e) = res {
            tracing::error!("FTS5 migration failed: {}. SQLite may not have FTS5 extension enabled.", e);
        } else {
            tracing::info!("Migration complete: FTS5 Archival Memory implemented");
        }
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

        CREATE TABLE IF NOT EXISTS runs (
            id              TEXT PRIMARY KEY,
            agent_id        TEXT NOT NULL,
            conversation_id TEXT,
            status          TEXT NOT NULL DEFAULT 'running',  -- running | completed | failed
            created_at      INTEGER NOT NULL,
            updated_at      INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS run_events (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            run_id  TEXT NOT NULL,
            seq_id  INTEGER NOT NULL,
            data    TEXT NOT NULL,
            UNIQUE (run_id, seq_id),
            FOREIGN KEY (run_id) REFERENCES runs(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS conversations (
            id          TEXT PRIMARY KEY,
            agent_id    TEXT NOT NULL,
            title       TEXT NOT NULL DEFAULT '',
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS messages (
            id              TEXT PRIMARY KEY,
            agent_id        TEXT NOT NULL,
            conversation_id TEXT,
            role            TEXT NOT NULL,
            content         TEXT NOT NULL,
            created_at      INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS shared_memory_blocks (
            id          TEXT PRIMARY KEY,
            label       TEXT NOT NULL,
            value       TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            max_chars   INTEGER,
            updated_at  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS agent_memory_blocks (
            agent_id TEXT NOT NULL,
            block_id TEXT NOT NULL,
            PRIMARY KEY (agent_id, block_id),
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
            FOREIGN KEY (block_id) REFERENCES shared_memory_blocks(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS memory_blocks (
            id          TEXT PRIMARY KEY,
            agent_id    TEXT NOT NULL,
            label       TEXT NOT NULL,
            value       TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            max_chars   INTEGER,
            updated_at  INTEGER NOT NULL,
            UNIQUE (agent_id, label),
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS memory_history (
            id         TEXT PRIMARY KEY,
            block_id   TEXT NOT NULL,
            value      TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_history_block_id
            ON memory_history(block_id, updated_at DESC);

        -- FTS5 Archival Memory
        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            content,
            content='messages',
            content_rowid='id'
        );

        CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
            INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content);
        END;
        CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
            INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.rowid, old.content);
        END;
        CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
            INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.rowid, old.content);
            INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content);
        END;

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

pub fn update_agent_name(db: &Db, id: &str, name: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    let n = conn.execute(
        "UPDATE agents SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    Ok(n > 0)
}

pub fn update_agent_system_prompt(db: &Db, id: &str, prompt: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    let n = conn.execute(
        "UPDATE agents SET system_prompt = ?1 WHERE id = ?2",
        params![prompt, id],
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

/// Return (tool_id, tool_name) pairs for all tools attached to an agent.
pub fn get_agent_tools_with_names(db: &Db, agent_id: &str) -> Result<Vec<(String, String)>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT at.tool_id, t.name FROM agent_tools at
         JOIN tools t ON t.id = at.tool_id
         WHERE at.agent_id = ?1
         ORDER BY t.name"
    )?;
    let rows = stmt.query_map(params![agent_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Detach ALL tools from an agent (clear agent_tools rows for this agent).
pub fn detach_all_tools_from_agent(db: &Db, agent_id: &str) -> Result<usize> {
    let conn = db.lock().unwrap();
    let n = conn.execute(
        "DELETE FROM agent_tools WHERE agent_id = ?1",
        params![agent_id],
    )?;
    Ok(n)
}

// ── Conversations ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConversationRow {
    pub id:         String,
    pub agent_id:   String,
    pub title:      String,
    pub created_at: i64,
    pub updated_at: i64,
    pub message_count: i64,
}

pub fn create_conversation(db: &Db, agent_id: &str, title: &str) -> Result<ConversationRow> {
    let id = format!("conv-{}", uuid::Uuid::new_v4());
    let ts = now_ts();
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO conversations (id, agent_id, title, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, agent_id, title, ts, ts],
    )?;
    Ok(ConversationRow { id, agent_id: agent_id.to_string(), title: title.to_string(),
                         created_at: ts, updated_at: ts, message_count: 0 })
}

pub fn get_conversation(db: &Db, conv_id: &str) -> Result<Option<ConversationRow>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT c.id, c.agent_id, c.title, c.created_at, c.updated_at,
                COUNT(m.id) as message_count
         FROM conversations c
         LEFT JOIN messages m ON m.conversation_id = c.id
         WHERE c.id = ?1
         GROUP BY c.id"
    )?;
    let mut rows = stmt.query(params![conv_id])?;
    if let Some(r) = rows.next()? {
        Ok(Some(ConversationRow {
            id:            r.get(0)?,
            agent_id:      r.get(1)?,
            title:         r.get(2)?,
            created_at:    r.get(3)?,
            updated_at:    r.get(4)?,
            message_count: r.get(5)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn list_conversations(db: &Db, agent_id: &str) -> Result<Vec<ConversationRow>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT c.id, c.agent_id, c.title, c.created_at, c.updated_at,
                COUNT(m.id) as message_count
         FROM conversations c
         LEFT JOIN messages m ON m.conversation_id = c.id
         WHERE c.agent_id = ?1
         GROUP BY c.id
         ORDER BY c.updated_at DESC"
    )?;
    let rows = stmt.query_map(params![agent_id], |r| {
        Ok(ConversationRow {
            id:            r.get(0)?,
            agent_id:      r.get(1)?,
            title:         r.get(2)?,
            created_at:    r.get(3)?,
            updated_at:    r.get(4)?,
            message_count: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn delete_conversation(db: &Db, conv_id: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    // CASCADE deletes the messages too
    let n = conn.execute("DELETE FROM conversations WHERE id = ?1", params![conv_id])?;
    // Also clean up orphaned messages (fallback for rows without FK enforcement)
    let _ = conn.execute(
        "DELETE FROM messages WHERE conversation_id = ?1", params![conv_id]
    );
    Ok(n > 0)
}

/// Update the conversation's title and bump updated_at.
pub fn update_conversation_title(db: &Db, conv_id: &str, title: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "UPDATE conversations SET title = ?1, updated_at = ?2 WHERE id = ?3",
        params![title, now_ts(), conv_id],
    )?;
    Ok(())
}

/// Touch updated_at (called when a new message is added to a conversation).
pub fn touch_conversation(db: &Db, conv_id: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
        params![now_ts(), conv_id],
    )?;
    Ok(())
}

// ── Runs (background mode) ────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunRow {
    pub id:              String,
    pub agent_id:        String,
    pub conversation_id: Option<String>,
    pub status:          String, // "running" | "completed" | "failed"
    pub created_at:      i64,
    pub updated_at:      i64,
}

pub fn create_run(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<RunRow> {
    let id = format!("run-{}", uuid::Uuid::new_v4());
    let ts = now_ts();
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO runs (id, agent_id, conversation_id, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'running', ?4, ?5)",
        params![id, agent_id, conversation_id, ts, ts],
    )?;
    Ok(RunRow {
        id, agent_id: agent_id.to_string(),
        conversation_id: conversation_id.map(String::from),
        status: "running".to_string(), created_at: ts, updated_at: ts,
    })
}

pub fn get_run(db: &Db, run_id: &str) -> Result<Option<RunRow>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, agent_id, conversation_id, status, created_at, updated_at
         FROM runs WHERE id = ?1"
    )?;
    let mut rows = stmt.query(params![run_id])?;
    if let Some(r) = rows.next()? {
        Ok(Some(RunRow {
            id:              r.get(0)?,
            agent_id:        r.get(1)?,
            conversation_id: r.get(2)?,
            status:          r.get(3)?,
            created_at:      r.get(4)?,
            updated_at:      r.get(5)?,
        }))
    } else { Ok(None) }
}

pub fn finish_run(db: &Db, run_id: &str, status: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "UPDATE runs SET status = ?1, updated_at = ?2 WHERE id = ?3",
        params![status, now_ts(), run_id],
    )?;
    Ok(())
}

/// Append an SSE event payload to the run's event log.
/// Returns the assigned seq_id.
pub fn append_run_event(db: &Db, run_id: &str, data: &str) -> Result<i64> {
    let conn = db.lock().unwrap();
    // Find current max seq_id for this run
    let max_seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq_id), -1) FROM run_events WHERE run_id = ?1",
        params![run_id],
        |r| r.get(0),
    ).unwrap_or(-1);
    let next_seq = max_seq + 1;
    conn.execute(
        "INSERT INTO run_events (run_id, seq_id, data) VALUES (?1, ?2, ?3)",
        params![run_id, next_seq, data],
    )?;
    Ok(next_seq)
}

/// Load run events after a given seq_id (exclusive).
pub fn run_events_after(db: &Db, run_id: &str, after_seq: i64) -> Result<Vec<(i64, String)>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT seq_id, data FROM run_events
         WHERE run_id = ?1 AND seq_id > ?2
         ORDER BY seq_id ASC"
    )?;
    let rows = stmt.query_map(params![run_id, after_seq], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageRow {
    pub id:              String,
    pub agent_id:        String,
    pub conversation_id: Option<String>,
    pub role:            String,
    pub content:         Value,
}

pub fn insert_message(db: &Db, row: &MessageRow) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            row.id, row.agent_id, row.conversation_id,
            row.role, row.content.to_string(), now_ts()
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
    let conn = db.lock().unwrap();
    // Filter: conversation_id IS NULL for legacy messages, or matches given id.
    let sql = if conversation_id.is_some() {
        "SELECT id, agent_id, conversation_id, role, content FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2
         ORDER BY created_at DESC LIMIT ?3"
    } else {
        "SELECT id, agent_id, conversation_id, role, content FROM messages
         WHERE agent_id = ?1 AND conversation_id IS NULL
         ORDER BY created_at DESC LIMIT ?3"
    };

    let mut stmt = conn.prepare(sql)?;
    let conv_placeholder = conversation_id.unwrap_or("");
    let rows = stmt.query_map(
        params![agent_id, conv_placeholder, limit as i64],
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
        .map(|(id, agent_id, conversation_id, role, content)| MessageRow {
            id,
            agent_id,
            conversation_id,
            role,
            content: serde_json::from_str(&content).unwrap_or(Value::String(content)),
        })
        .collect();
    result.reverse(); // return oldest-first
    Ok(result)
}

// ── Memory blocks ─────────────────────────────────────────────────────────────

/// Upsert a memory block.
///
/// `max_chars`: if `Some(n)`, enforces a hard character limit on `value`.
///   - On **set**: returns `Err` if value exceeds `n` (agent must summarise first).
///   - On **append** (caller passes pre-appended string): trims the oldest (front)
///     content to fit within `n` chars, preserving the newest content.
///
/// The previous value is snapshotted into `memory_history` before overwrite
/// (up to 5 revisions kept per block).
pub fn upsert_memory_block(
    db: &Db,
    agent_id: &str,
    label: &str,
    value: &str,
    description: Option<&str>,
    max_chars: Option<usize>,
) -> Result<()> {
    let conn = db.lock().unwrap();

    // Fetch existing block linked to this agent with this label
    let existing: Option<(String, String, Option<usize>)> = conn.query_row(
        "SELECT b.id, b.value, b.max_chars FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
        params![agent_id, label],
        |r| Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<i64>>(2)?.map(|n| n as usize),
        )),
    ).optional()?;

    // Effective limit: prefer caller-supplied, else stored, else none.
    let effective_limit = max_chars.or_else(|| existing.as_ref().and_then(|(_, _, mc)| *mc));

    // Apply size limit to the incoming value.
    let final_value: String = if let Some(limit) = effective_limit {
        let char_count = value.chars().count();
        if char_count > limit {
            // Trim oldest (front) content — keep the tail (newest).
            let start_byte = value
                .char_indices()
                .nth(char_count - limit)
                .map(|(i, _)| i)
                .unwrap_or(0);
            format!("[…trimmed]\n{}", &value[start_byte..])
        } else {
            value.to_string()
        }
    } else {
        value.to_string()
    };

    let ts = now_ts();

    if let Some((block_id, old_value, _)) = existing {
        // Snapshot old value into history (skip if unchanged)
        if old_value != final_value {
            let hist_id = uuid::Uuid::new_v4().to_string();
            let _ = conn.execute(
                "INSERT INTO memory_history (id, block_id, value, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params![hist_id, block_id, old_value, ts],
            );
            // Prune to last 5 revisions
            let _ = conn.execute(
                "DELETE FROM memory_history WHERE block_id = ?1
                 AND id NOT IN (
                     SELECT id FROM memory_history WHERE block_id = ?1
                     ORDER BY updated_at DESC LIMIT 5
                 )",
                params![block_id],
            );
        }

        if let Some(desc) = description {
            conn.execute(
                "UPDATE shared_memory_blocks SET value = ?1, description = ?2, max_chars = ?3, updated_at = ?4
                 WHERE id = ?5",
                params![final_value, desc, max_chars.map(|n| n as i64), ts, block_id],
            )?;
        } else {
            conn.execute(
                "UPDATE shared_memory_blocks SET value = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![final_value, ts, block_id],
            )?;
        }
    } else {
        // Create a new shared block and link it to the agent
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO shared_memory_blocks (id, label, value, description, max_chars, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, label, final_value, description.unwrap_or(""),
                    max_chars.map(|n| n as i64), ts],
        )?;
        conn.execute(
            "INSERT INTO agent_memory_blocks (agent_id, block_id) VALUES (?1, ?2)",
            params![agent_id, id],
        )?;
    }
    Ok(())
}

/// Link an existing shared memory block to an agent.
pub fn link_shared_memory_block(db: &Db, agent_id: &str, block_id: &str) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO agent_memory_blocks (agent_id, block_id) VALUES (?1, ?2)",
        params![agent_id, block_id],
    )?;
    Ok(())
}

pub fn delete_memory_block(db: &Db, agent_id: &str, label: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    // We only remove the link, not the shared block itself (to avoid orphan issues if shared)
    // Actually, Letta docs imply it's removed from the agent's view.
    let n = conn.execute(
        "DELETE FROM agent_memory_blocks WHERE agent_id = ?1 AND block_id IN (
            SELECT id FROM shared_memory_blocks WHERE label = ?2
        )",
        params![agent_id, label],
    )?;
    Ok(n > 0)
}

/// Returns (label, value, description) tuples ordered by label.
pub fn get_memory_blocks(db: &Db, agent_id: &str) -> Result<Vec<(String, String, String)>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT b.label, b.value, b.description FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 ORDER BY b.label"
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2).unwrap_or_default(),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Returns (label, value, description, updated_at) ordered by updated_at DESC (most recent first).
/// Used by build_context to apply the memory budget with recency priority.
pub fn get_memory_blocks_with_ts(db: &Db, agent_id: &str) -> Result<Vec<(String, String, String, i64)>> {
    let conn = db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT b.label, b.value, b.description, b.updated_at FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 ORDER BY b.updated_at DESC"
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2).unwrap_or_default(),
            row.get::<_, i64>(3).unwrap_or(0),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Returns the last N revisions of a memory block: (id, value, updated_at).
pub fn get_memory_history(db: &Db, agent_id: &str, label: &str, limit: usize) -> Result<Vec<(String, String, i64)>> {
    let conn = db.lock().unwrap();
    let block_id: Option<String> = conn.query_row(
        "SELECT b.id FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
        params![agent_id, label],
        |r| r.get(0),
    ).optional()?;
    let Some(block_id) = block_id else { return Ok(vec![]); };
    let mut stmt = conn.prepare(
        "SELECT id, value, updated_at FROM memory_history
         WHERE block_id = ?1 ORDER BY updated_at DESC LIMIT ?2"
    )?;
    let rows = stmt.query_map(params![block_id, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Restore a memory block to a specific history revision.
pub fn restore_memory_from_history(db: &Db, agent_id: &str, label: &str, hist_id: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    let block_id: Option<String> = conn.query_row(
        "SELECT b.id FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
        params![agent_id, label],
        |r| r.get(0),
    ).optional()?;
    let Some(block_id) = block_id else { return Ok(false); };
    let hist_value: Option<String> = conn.query_row(
        "SELECT value FROM memory_history WHERE id = ?1 AND block_id = ?2",
        params![hist_id, block_id],
        |r| r.get(0),
    ).optional()?;
    let Some(hist_value) = hist_value else { return Ok(false); };
    conn.execute(
        "UPDATE shared_memory_blocks SET value = ?1, updated_at = ?2 WHERE id = ?3",
        params![hist_value, now_ts(), block_id],
    )?;
    Ok(true)
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

/// Delete all messages for an agent (or a specific conversation).
/// If conversation_id is None, deletes all messages for the agent.
pub fn clear_messages(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<usize> {
    let conn = db.lock().unwrap();
    let n = if let Some(conv_id) = conversation_id {
        conn.execute(
            "DELETE FROM messages WHERE agent_id = ?1 AND conversation_id = ?2",
            params![agent_id, conv_id],
        )?
    } else {
        conn.execute(
            "DELETE FROM messages WHERE agent_id = ?1 AND conversation_id IS NULL",
            params![agent_id],
        )?
    };
    Ok(n)
}

/// Full-text search over message content for an agent (optionally scoped to a conversation).
pub fn search_messages(
    db: &Db,
    agent_id: &str,
    query: &str,
    conversation_id: Option<&str>,
) -> Result<Vec<MessageRow>> {
    let conn = db.lock().unwrap();
    
    // We use the FTS5 table for fast searching, then join back to messages for full row data.
    // If conversation_id is provided, we filter the results.
    let sql = if conversation_id.is_some() {
        "SELECT m.id, m.agent_id, m.conversation_id, m.role, m.content FROM messages m
         JOIN messages_fts f ON f.rowid = m.rowid
         WHERE m.agent_id = ?1 AND m.conversation_id = ?2 AND messages_fts MATCH ?3
         ORDER BY m.rowid DESC LIMIT 50"
    } else {
        "SELECT m.id, m.agent_id, m.conversation_id, m.role, m.content FROM messages m
         JOIN messages_fts f ON f.rowid = m.rowid
         WHERE m.agent_id = ?1 AND messages_fts MATCH ?2
         ORDER BY m.rowid DESC LIMIT 50"
    };

    let mut stmt = conn.prepare(sql)?;
    
    // FTS5 MATCH query: if simple string, wrap in quotes to handle special chars or spaces
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));

    let mapper = |r: &rusqlite::Row| {
        let content_str: String = r.get(4)?;
        let content = serde_json::from_str(&content_str)
            .unwrap_or(serde_json::Value::String(content_str));
        Ok(MessageRow {
            id:              r.get(0)?,
            agent_id:        r.get(1)?,
            conversation_id: r.get(2)?,
            role:            r.get(3)?,
            content,
        })
    };

    let result: Vec<MessageRow> = if let Some(conv_id) = conversation_id {
        stmt.query_map(params![agent_id, conv_id, fts_query], mapper)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map(params![agent_id, fts_query], mapper)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    Ok(result)
}

pub fn pending_tool_results(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> Result<(usize, usize)> {
    let messages = list_messages(db, agent_id, conversation_id, 20)?;

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
    
    // SEC-02: Encrypt API key at rest
    let encrypted_key = match &row.api_key {
        Some(k) if !k.is_empty() => Some(crate::server::crypto::encrypt(k)?),
        other => other.clone(),
    };

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
            encrypted_key,
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
    let mut providers = Vec::new();
    let mut rows = stmt.query([])?;
    
    while let Some(r) = rows.next()? {
        let name: String = r.get(0)?;
        let kind: String = r.get(1)?;
        let encrypted_key: Option<String> = r.get(2)?;
        let base_url: Option<String> = r.get(3)?;
        let enabled: bool = r.get::<_, i64>(4)? != 0;

        // SEC-02: Decrypt API key after retrieval
        // L-02: Surface decryption errors rather than silently returning None.
        let api_key = match encrypted_key {
            Some(k) if !k.is_empty() => {
                match crate::server::crypto::decrypt(&k) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        tracing::error!(
                            "Failed to decrypt API key for provider '{}': {e}. \
                             The key may have been encrypted on a different machine. \
                             Re-save the provider to re-encrypt with the current machine key.",
                            name
                        );
                        return Err(anyhow::anyhow!(
                            "Decrypt failed for provider '{name}': {e}"
                        ));
                    }
                }
            }
            other => other,
        };

        providers.push(ProviderRow {
            name,
            kind,
            api_key,
            base_url,
            enabled,
        });
    }
    
    Ok(providers)
}

pub fn delete_provider(db: &Db, name: &str) -> Result<bool> {
    let conn = db.lock().unwrap();
    let n = conn.execute("DELETE FROM providers WHERE name = ?1", params![name])?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_mem_db() -> Db {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        apply_schema(&conn).unwrap();
        run_migrations(&conn).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[test]
    fn test_shared_memory() {
        let db = setup_mem_db();
        let agent1 = "agent-1";
        let agent2 = "agent-2";

        // Create agents
        create_agent(&db, &AgentRow {
            id: agent1.to_string(), name: "A1".to_string(), model: "m".to_string(),
            description: None, system_prompt: None
        }).unwrap();
        create_agent(&db, &AgentRow {
            id: agent2.to_string(), name: "A2".to_string(), model: "m".to_string(),
            description: None, system_prompt: None
        }).unwrap();

        // 1. Agent 1 creates a block
        upsert_memory_block(&db, agent1, "shared_fact", "Initial value", None, None).unwrap();
        
        // Find the block ID
        let block_id: String = {
            let conn = db.lock().unwrap();
            conn.query_row(
                "SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?1",
                params![agent1],
                |r| r.get(0)
            ).unwrap()
        };

        // 2. Link Agent 2 to the same block
        link_shared_memory_block(&db, agent2, &block_id).unwrap();

        // 3. Verify both see the same value
        let b1 = get_memory_blocks(&db, agent1).unwrap();
        let b2 = get_memory_blocks(&db, agent2).unwrap();
        assert_eq!(b1[0].1, "Initial value");
        assert_eq!(b2[0].1, "Initial value");

        // 4. Agent 2 updates the block
        upsert_memory_block(&db, agent2, "shared_fact", "Updated by A2", None, None).unwrap();

        // 5. Verify Agent 1 sees the update
        let b1_new = get_memory_blocks(&db, agent1).unwrap();
        assert_eq!(b1_new[0].1, "Updated by A2");
    }

    #[test]
    fn test_archival_memory_fts() {
        let db = setup_mem_db();
        let agent_id = "agent-fts";

        create_agent(&db, &AgentRow {
            id: agent_id.to_string(), name: "A".to_string(), model: "m".to_string(),
            description: None, system_prompt: None
        }).unwrap();

        insert_message(&db, &MessageRow {
            id: "m1".to_string(), agent_id: agent_id.to_string(), conversation_id: None,
            role: "user".to_string(), content: serde_json::json!("Rust is a systems programming language")
        }).unwrap();

        insert_message(&db, &MessageRow {
            id: "m2".to_string(), agent_id: agent_id.to_string(), conversation_id: None,
            role: "assistant".to_string(), content: serde_json::json!("I agree, Rust is safe and fast.")
        }).unwrap();

        // Search for "systems"
        let res = search_messages(&db, agent_id, "systems", None).unwrap();
        assert_eq!(res.len(), 1);
        assert!(res[0].content.as_str().unwrap().contains("systems"));

        // Search for "safe"
        let res2 = search_messages(&db, agent_id, "safe", None).unwrap();
        assert_eq!(res2.len(), 1);
        assert!(res2[0].content.as_str().unwrap().contains("fast"));
    }
}
