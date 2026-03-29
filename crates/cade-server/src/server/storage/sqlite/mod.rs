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

/// Idempotent migrations — run after apply_schema on every startup.
fn run_migrations(conn: &Connection) -> Result<()> {
    // Migration 1: add UNIQUE(agent_id, label) to memory_blocks if missing.
    // SQLite doesn't support ALTER TABLE ADD CONSTRAINT, so we rebuild the table.
    // Detect UNIQUE(agent_id, label) specifically.
    // Note: autoindices for PRIMARY KEY have sql=NULL — exclude them with sql IS NOT NULL.
    // A user-defined UNIQUE constraint generates an autoindex whose sql is also NULL,
    // so we check the index name pattern instead.
    let has_unique: bool = conn
        .query_row(
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
        )
        .unwrap_or(0)
        > 0;

    if !has_unique {
        tracing::info!("Running migration: adding UNIQUE(agent_id, label) to memory_blocks");
        conn.execute_batch(
            r#"
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
        "#,
        )?;
        tracing::info!("Migration complete: memory_blocks UNIQUE constraint added");
    }

    // Migration 3: add `max_chars` column to memory_blocks if missing.
    let has_max_chars: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memory_blocks') WHERE name='max_chars'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_max_chars {
        tracing::info!("Running migration: adding max_chars column to memory_blocks");
        conn.execute_batch("ALTER TABLE memory_blocks ADD COLUMN max_chars INTEGER;")?;
        tracing::info!("Migration complete: memory_blocks.max_chars added");
    }

    // Migration 4: create memory_history table if it doesn't exist.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS memory_history (
            id         TEXT PRIMARY KEY,
            block_id   TEXT NOT NULL,
            value      TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_history_block_id
            ON memory_history(block_id, updated_at DESC);
    "#,
    )?;

    // Migration 2: add `description` column to memory_blocks if missing.
    // SQLite supports ADD COLUMN directly (no table rebuild needed).
    let has_description: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memory_blocks') WHERE name='description'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_description {
        tracing::info!("Running migration: adding description column to memory_blocks");
        conn.execute_batch(
            "ALTER TABLE memory_blocks ADD COLUMN description TEXT NOT NULL DEFAULT '';",
        )?;
        tracing::info!("Migration complete: memory_blocks.description added");
    }

    // Migration 3: add conversation_id column to messages + index.
    let has_conv_col: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name='conversation_id'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_conv_col {
        tracing::info!("Running migration: adding conversation_id to messages");
        conn.execute_batch(
            "ALTER TABLE messages ADD COLUMN conversation_id TEXT;
             CREATE INDEX IF NOT EXISTS idx_messages_conv
               ON messages(agent_id, conversation_id);",
        )?;
        tracing::info!("Migration complete: messages.conversation_id added");
    }

    // Migration 5: Shared Memory Blocks
    let has_shared_memory: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='shared_memory_blocks'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_shared_memory {
        tracing::info!("Running migration: implement Shared Memory schema");
        conn.execute_batch(
            r#"
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
        "#,
        )?;
        tracing::info!("Migration complete: Shared Memory schema implemented");
    }

    // Migration 7: Three-tier memory (pinned / short / long) + per-agent turn counter
    // Uses silent ALTER TABLE — errors are ignored on existing columns.
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

    // Migration 6: Archival Memory (FTS5)
    let has_fts: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages_fts'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_fts {
        tracing::info!("Running migration: implement FTS5 Archival Memory");
        let res = conn.execute_batch(r#"
            BEGIN;
            CREATE VIRTUAL TABLE messages_fts USING fts5(
                content,
                content='messages',
                content_rowid='rowid'
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
            tracing::error!(
                "FTS5 migration failed: {}. SQLite may not have FTS5 extension enabled.",
                e
            );
        } else {
            tracing::info!("Migration complete: FTS5 Archival Memory implemented");
        }
    }

    // Migration 8: Remove provider rows whose encrypted API key can no longer be
    // decrypted (stale keys from a previous .cade-db.key or machine).
    // These rows are already skipped at load time (list_providers logs a warning
    // and continues), so deleting them loses no recoverable data.
    {
        let mut stmt = conn.prepare(
            "SELECT name, api_key FROM providers WHERE api_key IS NOT NULL AND api_key != ''",
        )?;
        let stale: Vec<String> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .filter(|(_, encrypted)| crate::server::crypto::decrypt(encrypted).is_err())
            .map(|(name, _)| name)
            .collect();

        if !stale.is_empty() {
            tracing::info!(
                "Migration 8: removing {} provider(s) with undecryptable API keys: {}",
                stale.len(),
                stale.join(", ")
            );
            for name in &stale {
                conn.execute("DELETE FROM providers WHERE name = ?1", params![name])?;
            }
            tracing::info!("Migration 8 complete: stale providers removed");
        }
    }

    // Migration 9: Archival Memory
    let has_archival: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='archival_memory'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_archival {
        tracing::info!("Running migration: implement FTS5 Archival Memory table");
        let res = conn.execute_batch(
            r#"
            BEGIN;
            CREATE VIRTUAL TABLE archival_memory USING fts5(
                id UNINDEXED,
                agent_id UNINDEXED,
                content,
                tags,
                created_at UNINDEXED
            );
            COMMIT;
        "#,
        );
        if let Err(e) = res {
            tracing::error!("FTS5 archival_memory migration failed: {}", e);
        } else {
            tracing::info!("Migration complete: FTS5 archival_memory implemented");
        }
    }

    // Migration 10: Conversation branching + checkpoints
    {
        let has_branch_id: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name='branch_id'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_branch_id {
            tracing::info!("Migration 10a: adding branch_id / parent_id to messages");
            let _ = conn.execute_batch(
                "ALTER TABLE messages ADD COLUMN parent_id TEXT;
                 ALTER TABLE messages ADD COLUMN branch_id TEXT NOT NULL DEFAULT 'main';
                 CREATE INDEX IF NOT EXISTS idx_messages_branch ON messages(agent_id, branch_id);",
            );
        }
        let has_checkpoints: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='checkpoints'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_checkpoints {
            tracing::info!("Migration 10b: creating checkpoints table");
            let _ = conn.execute_batch(r#"
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
            "#);
        }
    }

    // Migration 11: Artifact store
    {
        let has_artifacts: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='artifacts'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_artifacts {
            tracing::info!("Migration 11: creating artifacts table");
            let _ = conn.execute_batch(r#"
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
            "#);
        }
    }

    // Migration 12: Tool execution log
    {
        let has_te: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tool_executions'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_te {
            tracing::info!("Migration 12: creating tool_executions table");
            let _ = conn.execute_batch(r#"
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
            "#);
        }
    }

    // Migration 13: Eval harness
    {
        let has_eval: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='eval_tasks'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_eval {
            tracing::info!("Migration 13: creating eval tables");
            let _ = conn.execute_batch(
                r#"
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
            "#,
            );
        }
    }

    // Migration 14: Typed memory + provenance
    {
        let has_type: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('shared_memory_blocks') WHERE name='memory_type'",
            [], |r| r.get::<_, i64>(0),
        ).unwrap_or(0) > 0;
        if !has_type {
            tracing::info!("Migration 14: adding memory_type and provenance fields");
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
        }
        let has_evidence: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_evidence'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_evidence {
            let _ = conn.execute_batch(
                r#"
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
            "#,
            );
        }
    }

    // Migration 15: Reflection log
    {
        let has_reflection: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='reflection_log'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;
        if !has_reflection {
            tracing::info!("Migration 15: creating reflection_log table");
            let _ = conn.execute_batch(r#"
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
            "#);
        }
    }

    // Migration 9: add char_count to messages if missing
    let has_char_count: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name='char_count'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;

    if !has_char_count {
        tracing::info!("Running migration: adding char_count column to messages");
        conn.execute_batch(
            r#"
            ALTER TABLE messages ADD COLUMN char_count INTEGER NOT NULL DEFAULT 0;
            "#,
        )?;
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
            char_count      INTEGER NOT NULL DEFAULT 0,
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
pub mod runs;
pub mod messages;
pub mod memory;
pub mod tools;
pub mod providers;
pub mod evidence;

pub use agents::*;
pub use conversations::*;
pub use runs::*;
pub use messages::*;
pub use memory::*;
pub use tools::*;
pub use providers::*;
pub use evidence::*;
