use crate::server::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::sync::{Arc, Mutex};

// -- Provider row

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderRow {
    pub name: String,
    pub kind: String, // "anthropic" | "openai" | "gemini" | "ollama" | "openai-compatible"
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub enabled: bool,
}

/// Thread-safe SQLite handle
pub type Db = Arc<Mutex<Connection>>;

pub fn open(path: &str) -> Result<Db> {
    if let Some(parent) = std::path::Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)
        .map_err(|e| crate::server::error::Error::custom(format!("open SQLite at {path}: {e}")))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    apply_schema(&conn)?;
    run_migrations(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

/// Apply initial schema or update existing ones.
fn apply_schema(conn: &Connection) -> Result<()> {
    // 1. Create all tables to their latest structure for fresh databases.
    conn.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS agents (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            model       TEXT NOT NULL,
            description TEXT,
            system_prompt TEXT,
            created_at  INTEGER NOT NULL,
            memory_turn_counter INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS runs (
            id              TEXT PRIMARY KEY,
            agent_id        TEXT NOT NULL,
            conversation_id TEXT,
            status          TEXT NOT NULL DEFAULT 'running',
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
            char_count      INTEGER NOT NULL DEFAULT 0,
            parent_id       TEXT,
            branch_id       TEXT NOT NULL DEFAULT 'main',
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(agent_id, conversation_id);
        CREATE INDEX IF NOT EXISTS idx_messages_branch ON messages(agent_id, branch_id);

        CREATE TABLE IF NOT EXISTS shared_memory_blocks (
            id          TEXT PRIMARY KEY,
            label       TEXT NOT NULL,
            value       TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            max_chars   INTEGER,
            updated_at  INTEGER NOT NULL,
            last_turn   INTEGER NOT NULL DEFAULT 0,
            tier        TEXT NOT NULL DEFAULT 'short',
            memory_type TEXT NOT NULL DEFAULT 'generic',
            confidence  REAL NOT NULL DEFAULT 1.0,
            source_msg_id TEXT,
            source_te_id  TEXT,
            tags_json   TEXT NOT NULL DEFAULT '[]',
            expires_at  INTEGER
        );

        CREATE TABLE IF NOT EXISTS agent_memory_blocks (
            agent_id TEXT NOT NULL,
            block_id TEXT NOT NULL,
            PRIMARY KEY (agent_id, block_id),
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
            FOREIGN KEY (block_id) REFERENCES shared_memory_blocks(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS memory_history (
            id         TEXT PRIMARY KEY,
            block_id   TEXT NOT NULL,
            value      TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_history_block_id ON memory_history(block_id, updated_at DESC);

        CREATE TABLE IF NOT EXISTS memory_evidence (
            id         TEXT PRIMARY KEY,
            block_id   TEXT NOT NULL,
            kind       TEXT NOT NULL,
            reference  TEXT NOT NULL,
            excerpt    TEXT,
            confidence REAL NOT NULL DEFAULT 1.0,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (block_id) REFERENCES shared_memory_blocks(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_evidence_block ON memory_evidence(block_id);

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

        CREATE TABLE IF NOT EXISTS checkpoints (
            id              TEXT PRIMARY KEY,
            agent_id        TEXT NOT NULL,
            conversation_id TEXT,
            branch_id       TEXT NOT NULL DEFAULT 'main',
            label           TEXT,
            description     TEXT,
            created_at      INTEGER NOT NULL,
            git_stash_ref   TEXT,
            git_commit_hash TEXT,
            parent_id       TEXT,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_checkpoints_agent  ON checkpoints(agent_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_checkpoints_parent ON checkpoints(parent_id);

        CREATE TABLE IF NOT EXISTS artifacts (
            id           TEXT PRIMARY KEY,
            agent_id     TEXT NOT NULL,
            run_id       TEXT,
            tool_call_id TEXT,
            kind         TEXT NOT NULL,
            content_type TEXT NOT NULL,
            data_text    TEXT,
            data_blob    BLOB,
            metadata_json TEXT NOT NULL DEFAULT '{}',
            size_bytes   INTEGER NOT NULL DEFAULT 0,
            created_at   INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_artifacts_agent ON artifacts(agent_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_artifacts_run   ON artifacts(run_id);
        CREATE INDEX IF NOT EXISTS idx_artifacts_kind  ON artifacts(kind);

        CREATE TABLE IF NOT EXISTS tool_executions (
            id              TEXT PRIMARY KEY,
            run_id          TEXT,
            agent_id        TEXT NOT NULL,
            conversation_id TEXT,
            checkpoint_id   TEXT,
            tool_name       TEXT NOT NULL,
            arguments_json  TEXT NOT NULL,
            output          TEXT,
            is_error        INTEGER NOT NULL DEFAULT 0,
            duration_ms     INTEGER,
            created_at      INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_tool_exec_agent      ON tool_executions(agent_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_tool_exec_run        ON tool_executions(run_id);
        CREATE INDEX IF NOT EXISTS idx_tool_exec_checkpoint ON tool_executions(checkpoint_id);

        CREATE TABLE IF NOT EXISTS eval_tasks (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            description   TEXT,
            prompt        TEXT NOT NULL,
            expected_json TEXT,
            tags_json     TEXT NOT NULL DEFAULT '[]',
            created_at    INTEGER NOT NULL
        );
        
        CREATE TABLE IF NOT EXISTS eval_runs (
            id            TEXT PRIMARY KEY,
            task_id       TEXT NOT NULL,
            agent_id      TEXT,
            model         TEXT,
            status        TEXT NOT NULL DEFAULT 'pending',
            score         REAL,
            pass_criteria TEXT,
            result_json   TEXT,
            tool_calls_n  INTEGER NOT NULL DEFAULT 0,
            tokens_in     INTEGER NOT NULL DEFAULT 0,
            tokens_out    INTEGER NOT NULL DEFAULT 0,
            duration_ms   INTEGER,
            created_at    INTEGER NOT NULL,
            completed_at  INTEGER,
            FOREIGN KEY (task_id) REFERENCES eval_tasks(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_eval_runs_task  ON eval_runs(task_id);
        CREATE INDEX IF NOT EXISTS idx_eval_runs_model ON eval_runs(model);

        CREATE TABLE IF NOT EXISTS reflection_log (
            id             TEXT PRIMARY KEY,
            agent_id       TEXT NOT NULL,
            trigger        TEXT NOT NULL,
            model          TEXT,
            blocks_created INTEGER NOT NULL DEFAULT 0,
            blocks_updated INTEGER NOT NULL DEFAULT 0,
            blocks_deleted INTEGER NOT NULL DEFAULT 0,
            summary        TEXT,
            duration_ms    INTEGER,
            created_at     INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_reflection_log_agent ON reflection_log(agent_id, created_at DESC);
    "#)?;

    // FTS5 Tables
    let _ = conn.execute_batch(r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            content,
            content='messages',
            content_rowid='rowid'
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

        CREATE VIRTUAL TABLE IF NOT EXISTS archival_memory USING fts5(
            id UNINDEXED,
            agent_id UNINDEXED,
            content,
            tags,
            created_at UNINDEXED
        );
    "#);

    Ok(())
}

/// Run sequential, structured migrations using PRAGMA user_version.
fn run_migrations(conn: &Connection) -> Result<()> {
    let current_version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);

    // Version 1: Legacy catch-up
    // For databases created before user_version was used, we apply the old idempotent ALTER TABLEs.
    // For brand new databases, these will safely do nothing because apply_schema already built the columns.
    if current_version < 1 {
        tracing::info!("Running Migration 1: Legacy schema catch-up");

        // 1. Add unique constraint to memory_blocks (legacy) and migrate to shared_memory_blocks
        let has_shared: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='shared_memory_blocks'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
        if !has_shared {
            let _ = conn.execute_batch(r#"
                CREATE TABLE IF NOT EXISTS memory_blocks (
                    id TEXT PRIMARY KEY, agent_id TEXT, label TEXT, value TEXT, description TEXT, max_chars INTEGER, updated_at INTEGER
                );
                CREATE TABLE shared_memory_blocks (
                    id          TEXT PRIMARY KEY, label TEXT NOT NULL, value TEXT NOT NULL DEFAULT '',
                    description TEXT NOT NULL DEFAULT '', max_chars INTEGER, updated_at INTEGER NOT NULL
                );
                CREATE TABLE agent_memory_blocks (
                    agent_id TEXT NOT NULL, block_id TEXT NOT NULL, PRIMARY KEY (agent_id, block_id)
                );
                INSERT INTO shared_memory_blocks (id, label, value, description, max_chars, updated_at)
                    SELECT id, label, value, description, max_chars, updated_at FROM memory_blocks;
                INSERT INTO agent_memory_blocks (agent_id, block_id)
                    SELECT agent_id, id FROM memory_blocks;
            "#);
        }

        // 2. Add columns silently (fails safely if they exist)
        let _ = conn.execute("ALTER TABLE messages ADD COLUMN conversation_id TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE messages ADD COLUMN char_count INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute("ALTER TABLE messages ADD COLUMN parent_id TEXT", []);
        let _ = conn.execute(
            "ALTER TABLE messages ADD COLUMN branch_id TEXT NOT NULL DEFAULT 'main'",
            [],
        );

        let _ = conn.execute(
            "ALTER TABLE agents ADD COLUMN memory_turn_counter INTEGER NOT NULL DEFAULT 0",
            [],
        );

        let _ = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN last_turn INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN tier TEXT NOT NULL DEFAULT 'short'",
            [],
        );
        let _ = conn.execute("ALTER TABLE shared_memory_blocks ADD COLUMN memory_type TEXT NOT NULL DEFAULT 'generic'", []);
        let _ = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN source_msg_id TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN source_te_id TEXT",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN tags_json TEXT NOT NULL DEFAULT '[]'",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN expires_at INTEGER",
            [],
        );

        // 3. Remove stale providers with undecryptable keys
        if let Ok(mut stmt) = conn.prepare(
            "SELECT name, api_key FROM providers WHERE api_key IS NOT NULL AND api_key != ''",
        ) && let Ok(mapped) =
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        {
            let stale: Vec<String> = mapped
                .filter_map(|r| r.ok())
                .filter(|(_, enc)| crate::server::crypto::decrypt(enc).is_err())
                .map(|(name, _)| name)
                .collect();
            for name in stale {
                let _ = conn.execute("DELETE FROM providers WHERE name = ?1", params![name]);
            }
        }

        conn.execute("PRAGMA user_version = 1", [])?;
    }

    // Future migrations go here:
    // if current_version < 2 {
    //     ...
    //     conn.execute("PRAGMA user_version = 2", [])?;
    // }

    Ok(())
}

// -- Helpers

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// -- Agents

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentRow {
    pub id: String,
    pub name: String,
    pub model: String,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    /// Unix timestamp (seconds) when the agent was created.
    pub created_at: Option<i64>,
}

pub mod agents;
pub mod conversations;
pub mod evidence;
pub mod memory;
pub mod messages;
pub mod providers;
pub mod runs;
pub mod tools;

pub use agents::*;
pub use conversations::*;
pub use evidence::*;
pub use memory::*;
pub use messages::*;
pub use providers::*;
pub use runs::*;
pub use tools::*;
