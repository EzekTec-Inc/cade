use crate::error::Result;
use r2d2::Pool;

#[cfg(feature = "rig-compat")]
pub mod rig_store;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::time::Duration;

// -- Provider row

/// A provider row as stored in the SQLite `providers` table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProviderRow {
    /// Display / lookup name for the provider.
    pub name: String,
    /// Provider kind: `"anthropic"`, `"openai"`, `"gemini"`, `"ollama"`,
    /// or `"openai-compatible"`.
    pub kind: String,
    /// Optional API key (encrypted at rest when encryption is enabled).
    pub api_key: Option<String>,
    /// Optional base URL override (e.g. for self-hosted / compatible APIs).
    pub base_url: Option<String>,
    /// Whether this provider is currently active.
    pub enabled: bool,
}

/// Thread-safe SQLite handle backed by an r2d2 connection pool.
///
/// Each call to [`Db::get`] checks out an idle [`Connection`] from the pool
/// or, if all are in use, waits up to `connection_timeout` (default 30s).
/// Connections are returned to the pool when their `PooledConnection` is dropped.
///
/// Migration note: previously this was `Arc<Mutex<Connection>>` with an
/// infallible `lock()` helper. The new API is `db.get()?` and is fallible
/// (pool exhaustion / connection timeout surface as an error).
pub type Db = Pool<SqliteConnectionManager>;

/// Maximum pool size for file-backed databases.
const DEFAULT_MAX_POOL_SIZE: u32 = 8;

/// Connection timeout when checking out from the pool.
const POOL_CONNECTION_TIMEOUT_SECS: u64 = 30;

/// Open a SQLite database and return a pooled handle.
///
/// `path` may be a filesystem path or the special string `":memory:"`. For
/// in-memory databases the pool is restricted to a single connection so all
/// callers share the same database (otherwise each fresh connection would
/// produce an isolated, empty DB).
///
/// On the first connection the `apply_schema` + `run_migrations` routines
/// run. Every subsequent connection produced by the pool is initialised with
/// `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`.
pub fn open(path: &str) -> Result<Db> {
    if let Some(parent) = std::path::Path::new(path).parent()
        && !parent.as_os_str().is_empty()
        && path != ":memory:"
    {
        std::fs::create_dir_all(parent)?;
    }

    let in_memory = path == ":memory:";
    let manager = if in_memory {
        SqliteConnectionManager::memory()
    } else {
        SqliteConnectionManager::file(path)
    }
    .with_init(|c| {
        // Applied to every connection handed out by the pool.
        c.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
        )
    });

    let max_size = if in_memory { 1 } else { DEFAULT_MAX_POOL_SIZE };
    let pool = Pool::builder()
        .max_size(max_size)
        .connection_timeout(Duration::from_secs(POOL_CONNECTION_TIMEOUT_SECS))
        .build(manager)
        .map_err(|e| crate::error::Error::custom(format!("build SQLite pool at {path}: {e}")))?;

    // Run schema + migrations once on a freshly checked-out connection.
    {
        let conn = pool
            .get()
            .map_err(|e| crate::error::Error::custom(format!("get conn at {path}: {e}")))?;
        apply_schema(&conn)?;
        run_migrations(&conn)?;
    }

    Ok(pool)
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
            memory_turn_counter INTEGER NOT NULL DEFAULT 0,
            parent_id   TEXT,
            FOREIGN KEY (parent_id) REFERENCES agents(id) ON DELETE SET NULL
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
            expires_at  INTEGER,
            -- F7 (Migration 9): activity-weighted aging
            access_count       INTEGER NOT NULL DEFAULT 0,
            last_access_turn   INTEGER NOT NULL DEFAULT 0,
            -- A3 (Migration 12): provenance tracking
            source_turn        INTEGER,
            -- A.1 (Migration 14): explicit provenance tracking
            source_turn_id     TEXT,
            source_tool_id     TEXT
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

        -- A5 (Migration 13): Semantic chunks for large memory blocks.
        CREATE TABLE IF NOT EXISTS memory_chunks (
            id          TEXT PRIMARY KEY,
            block_id    TEXT NOT NULL,
            chunk_index INTEGER NOT NULL,
            content     TEXT NOT NULL,
            char_count  INTEGER NOT NULL,
            embedding   BLOB,
            FOREIGN KEY (block_id) REFERENCES shared_memory_blocks(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_chunks_block ON memory_chunks(block_id, chunk_index);

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
            output_chars    INTEGER NOT NULL DEFAULT 0,
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

        CREATE TABLE IF NOT EXISTS event_log (
            id              TEXT PRIMARY KEY,
            agent_id        TEXT NOT NULL,
            conversation_id TEXT,
            event_type      TEXT NOT NULL,
            content         TEXT NOT NULL,
            created_at      INTEGER NOT NULL,
            FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_event_log_agent ON event_log(agent_id, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_event_log_type ON event_log(event_type);

        CREATE TABLE IF NOT EXISTS agent_skill_blacklist (
            agent_id    TEXT NOT NULL,
            skill_id    TEXT NOT NULL,
            PRIMARY KEY (agent_id, skill_id)
        );
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

        CREATE VIRTUAL TABLE IF NOT EXISTS event_log_fts USING fts5(
            id UNINDEXED,
            agent_id UNINDEXED,
            event_type UNINDEXED,
            content,
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
                .filter(|(_, enc)| crate::crypto::decrypt(enc).is_err())
                .map(|(name, _)| name)
                .collect();
            for name in stale {
                let _ = conn.execute("DELETE FROM providers WHERE name = ?1", params![name]);
            }
        }

        conn.execute("PRAGMA user_version = 1", [])?;
    }

    // Future migrations go here:
    if current_version < 2 {
        // P1-C: Add optional compaction_model column to agents table.
        // When set, Sleeptime consolidation uses this (cheaper) model instead
        // of the agent's main model.
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN compaction_model TEXT", []);
        conn.execute("PRAGMA user_version = 2", [])?;
    }

    if current_version < 3 {
        let _ = conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS event_log (
                id              TEXT PRIMARY KEY,
                agent_id        TEXT NOT NULL,
                conversation_id TEXT,
                event_type      TEXT NOT NULL,
                content         TEXT NOT NULL,
                created_at      INTEGER NOT NULL,
                FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_event_log_agent ON event_log(agent_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_event_log_type ON event_log(event_type);
            
            CREATE VIRTUAL TABLE IF NOT EXISTS event_log_fts USING fts5(
                id UNINDEXED, agent_id UNINDEXED, event_type UNINDEXED, content, created_at UNINDEXED
            );
        "#);
        conn.execute("PRAGMA user_version = 3", [])?;
    }

    if current_version < 4 {
        // Phase 5: persist theme name per agent so GUI `/theme` survives reload.
        // Stored as nullable TEXT holding the theme name (e.g. "dark", "tokyo-night",
        // or a user theme file stem). NULL = inherit the global setting.
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN theme TEXT", []);
        conn.execute("PRAGMA user_version = 4", [])?;
    }

    if current_version < 5 {
        // Migration 5: agent_skill_blacklist — per-agent skill disable feature (Phase B).
        // `IF NOT EXISTS` is safe for new DBs (apply_schema already created it).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_skill_blacklist (
                agent_id    TEXT NOT NULL,
                skill_id    TEXT NOT NULL,
                PRIMARY KEY (agent_id, skill_id)
            );",
        )?;
        conn.execute("PRAGMA user_version = 5", [])?;
    }

    if current_version < 6 {
        // Migration 6 (P8): tool_executions.output_chars for per-call cost
        // observability without scanning the output blob.  Idempotent — pre-existing
        // databases get the column added; new DBs already have it from apply_schema.
        let _ = conn.execute(
            "ALTER TABLE tool_executions ADD COLUMN output_chars INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Backfill output_chars for rows persisted before the migration.
        let _ = conn.execute(
            "UPDATE tool_executions SET output_chars = LENGTH(output) WHERE output_chars = 0 AND output IS NOT NULL",
            [],
        );
        conn.execute("PRAGMA user_version = 6", [])?;
    }

    // ── Migration 7: P1 — observations table ─────────────────────────────────
    if current_version < 7 {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS observations (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                turn INTEGER NOT NULL DEFAULT 0,
                tool_name TEXT NOT NULL,
                observation_type TEXT NOT NULL DEFAULT 'tool_call',
                summary TEXT NOT NULL,
                files TEXT NOT NULL DEFAULT '[]',
                concepts TEXT NOT NULL DEFAULT '[]',
                importance INTEGER NOT NULL DEFAULT 3,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_observations_agent_turn
             ON observations (agent_id, turn DESC)",
            [],
        )?;
        conn.execute("PRAGMA user_version = 7", [])?;
    }

    // Adds two columns to `shared_memory_blocks` so `promote_stale_blocks`
    // can extend the retention window for memory blocks the agent has read
    // recently or frequently:
    //   - access_count       : total intentional reads (bumped by search_memory)
    //   - last_access_turn   : turn counter at the most recent intentional read
    //
    // Without these columns, the only signal the aging pass had was
    // `last_turn` (= last *write* turn), so a heavily consulted block that
    // was written once at turn 0 would be archived on turn 80 even if the
    // agent had searched for it 50 times in between.
    if current_version < 9 {
        let r1 = conn.execute(
            "ALTER TABLE shared_memory_blocks
             ADD COLUMN access_count INTEGER NOT NULL DEFAULT 0",
            [],
        );
        let r2 = conn.execute(
            "ALTER TABLE shared_memory_blocks
             ADD COLUMN last_access_turn INTEGER NOT NULL DEFAULT 0",
            [],
        );
        if let Err(e) = r1.and(r2) {
            // ALTER TABLE is idempotent only when the column doesn't exist —
            // if a previous partial run left one of the columns in place we
            // tolerate "duplicate column" errors and continue.
            let msg = e.to_string();
            if !msg.contains("duplicate column name") {
                tracing::warn!("Migration 9 (F7) ALTER TABLE failed: {e}");
            }
        }
        conn.execute("PRAGMA user_version = 9", [])?;
    }

    // ── Migration 10 (WI-SEMANTIC Phase 1): memory_blocks_fts ─────────────
    //
    // Adds an FTS5 virtual table for keyword search over memory blocks.
    // The previous code in `embedding::search_memory_blocks_fts` queried
    // `messages_fts` (conversation history FTS) by mistake, returning no
    // memory hits — see PLAN.md WI-SEMANTIC.
    //
    // Architecture:
    //   * external-content FTS5 (`content=shared_memory_blocks`) — no
    //     duplicate storage of label/value text.
    //   * `content_rowid=rowid` — uses SQLite's auto rowid on
    //     shared_memory_blocks (the table is NOT declared WITHOUT ROWID).
    //   * Triggers keep the FTS index in sync on INSERT/UPDATE/DELETE.
    //   * One-time backfill at end via `INSERT INTO memory_blocks_fts(...)
    //     VALUES('rebuild')` so existing rows are indexed.
    //
    // The semantic-search feature flag is NOT required for this migration:
    // FTS5 is part of bundled SQLite and benefits keyword search even when
    // the embedding stack is disabled.
    if current_version < 10 {
        let r = conn.execute_batch(
            r#"
            CREATE VIRTUAL TABLE IF NOT EXISTS memory_blocks_fts USING fts5(
                label,
                value,
                content='shared_memory_blocks',
                content_rowid='rowid'
            );

            CREATE TRIGGER IF NOT EXISTS shared_memory_blocks_ai
            AFTER INSERT ON shared_memory_blocks BEGIN
                INSERT INTO memory_blocks_fts(rowid, label, value)
                VALUES (new.rowid, new.label, new.value);
            END;

            CREATE TRIGGER IF NOT EXISTS shared_memory_blocks_ad
            AFTER DELETE ON shared_memory_blocks BEGIN
                INSERT INTO memory_blocks_fts(memory_blocks_fts, rowid, label, value)
                VALUES ('delete', old.rowid, old.label, old.value);
            END;

            CREATE TRIGGER IF NOT EXISTS shared_memory_blocks_au
            AFTER UPDATE ON shared_memory_blocks BEGIN
                INSERT INTO memory_blocks_fts(memory_blocks_fts, rowid, label, value)
                VALUES ('delete', old.rowid, old.label, old.value);
                INSERT INTO memory_blocks_fts(rowid, label, value)
                VALUES (new.rowid, new.label, new.value);
            END;

            INSERT INTO memory_blocks_fts(memory_blocks_fts) VALUES('rebuild');
            "#,
        );
        if let Err(e) = r {
            // Tolerate re-run when artefacts already exist (idempotent migration).
            let msg = e.to_string();
            if !(msg.contains("already exists")
                || msg.contains("trigger") && msg.contains("exists"))
            {
                tracing::warn!("Migration 10 (WI-SEMANTIC) memory_blocks_fts setup failed: {e}");
            }
        }
        conn.execute("PRAGMA user_version = 10", [])?;
    }

    // ── Migration 11 (WI-SEMANTIC Phase 2): embedding BLOB column ─────────
    //
    // Adds an optional `embedding` BLOB column on shared_memory_blocks for
    // storing per-block embedding vectors as packed little-endian f32 bytes.
    //
    // The column is always present (regardless of the `semantic-search`
    // feature flag) so the schema is portable: a default-feature build can
    // open and read a DB written by a feature-enabled build, the embedding
    // bytes are simply ignored. Writes are gated on having an Embedder.
    if current_version < 11 {
        let r = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN embedding BLOB",
            [],
        );
        if let Err(e) = r {
            let msg = e.to_string();
            if !msg.contains("duplicate column name") {
                tracing::warn!("Migration 11 (WI-SEMANTIC) ADD COLUMN embedding failed: {e}");
            }
        }
        conn.execute("PRAGMA user_version = 11", [])?;
    }

    // ── Migration 12: A3 provenance — add source_turn column ─────────────────
    // `source_te_id` (TEXT, already exists from migration 2) is repurposed as
    // `source_tool_call_id` — it stores the tool_call_id that triggered the
    // memory write.  `source_turn` (INTEGER) records the agent's turn counter
    // at write time so we can trace *when* a fact was established.
    if current_version < 12 {
        let r = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN source_turn INTEGER",
            [],
        );
        if let Err(e) = r {
            let msg = e.to_string();
            if !msg.contains("duplicate column name") {
                tracing::warn!("Migration 12 (A3-provenance) ADD COLUMN source_turn failed: {e}");
            }
        }
        conn.execute("PRAGMA user_version = 12", [])?;
    }

    // ── Migration 13: A5 semantic chunks table ───────────────────────────────
    if current_version < 13 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory_chunks (
                 id          TEXT PRIMARY KEY,
                 block_id    TEXT NOT NULL,
                 chunk_index INTEGER NOT NULL,
                 content     TEXT NOT NULL,
                 char_count  INTEGER NOT NULL,
                 embedding   BLOB,
                 FOREIGN KEY (block_id) REFERENCES shared_memory_blocks(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_chunks_block ON memory_chunks(block_id, chunk_index);",
        )?;
        conn.execute("PRAGMA user_version = 13", [])?;
    }

    // ── Migration 14: A.1 Schema Migration (explicit provenance) ────────────────
    if current_version < 14 {
        let r1 = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN source_turn_id TEXT",
            [],
        );
        if let Err(e) = r1 {
            let msg = e.to_string();
            if !msg.contains("duplicate column name") {
                tracing::warn!(
                    "Migration 14 (A.1-provenance) ADD COLUMN source_turn_id failed: {e}"
                );
            }
        }
        let r2 = conn.execute(
            "ALTER TABLE shared_memory_blocks ADD COLUMN source_tool_id TEXT",
            [],
        );
        if let Err(e) = r2 {
            let msg = e.to_string();
            if !msg.contains("duplicate column name") {
                tracing::warn!(
                    "Migration 14 (A.1-provenance) ADD COLUMN source_tool_id failed: {e}"
                );
            }
        }
        conn.execute("PRAGMA user_version = 14", [])?;
    }

    if current_version < 15 {
        let r = conn.execute("ALTER TABLE agents ADD COLUMN active_plan_json TEXT", []);
        if let Err(e) = r {
            let msg = e.to_string();
            if !msg.contains("duplicate column name") {
                tracing::warn!("Migration 15 ADD COLUMN active_plan_json failed: {e}");
            }
        }
        conn.execute("PRAGMA user_version = 15", [])?;
    }

    if current_version < 16 {
        let r1 = conn.execute(
            "CREATE TABLE IF NOT EXISTS knowledge_edges (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                entity TEXT NOT NULL,
                relation TEXT NOT NULL,
                target TEXT NOT NULL,
                embedding BLOB,
                created_at INTEGER NOT NULL
            )",
            [],
        );
        let r2 = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_knowledge_edges_entity ON knowledge_edges(entity)",
            [],
        );
        if let Err(e) = r1.and(r2) {
            tracing::warn!("Migration 16 CREATE TABLE knowledge_edges failed: {e}");
        }
        conn.execute("PRAGMA user_version = 16", [])?;
    }

    if current_version < 17 {
        let r = conn.execute(
            "CREATE TABLE IF NOT EXISTS run_checkpoints (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                conversation_id TEXT,
                current_iteration INTEGER NOT NULL,
                serialized_state TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )",
            [],
        );
        if let Err(e) = r {
            tracing::warn!("Migration 17 CREATE TABLE run_checkpoints failed: {e}");
        }
        conn.execute("PRAGMA user_version = 17", [])?;
    }

    if current_version < 18 {
        let r1 = conn.execute(
            "CREATE TABLE IF NOT EXISTS pending_approvals (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                subagent_id TEXT,
                tool_name TEXT NOT NULL,
                arguments TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL
            )",
            [],
        );
        let r2 = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_pending_approvals_status ON pending_approvals(status)",
            [],
        );
        if let Err(e) = r1.and(r2) {
            tracing::warn!("Migration 18 CREATE TABLE pending_approvals failed: {e}");
        }
        conn.execute("PRAGMA user_version = 18", [])?;
    }

    if current_version < 19 {
        let _ = conn.execute("ALTER TABLE agents ADD COLUMN parent_id TEXT", []);
        conn.execute("PRAGMA user_version = 19", [])?;
    }

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
    /// Optional cheaper model used for Sleeptime consolidation summaries.
    /// When `None`, consolidation falls back to `model`.
    pub compaction_model: Option<String>,
    /// Optional theme name (built-in or user-defined) last set via `/theme`.
    /// Persisted so GUI restores the theme across page reloads.
    /// `None` → inherit from global settings.
    #[serde(default)]
    pub theme: Option<String>,
    /// Optional active plan serialized as JSON.
    #[serde(default)]
    pub active_plan_json: Option<String>,
    /// Optional parent agent ID.
    #[serde(default)]
    pub parent_id: Option<String>,
}

pub mod agents;
pub mod approvals;
pub mod conversations;
pub mod embedding;
pub mod event_log;
pub mod evidence;
pub mod horizon;
pub mod knowledge;
pub mod memory;
pub mod messages;
pub mod observations;
pub mod providers;
pub mod run_checkpoints;
pub mod runs;
pub mod skills;
pub mod tools;

pub use agents::*;
pub use approvals::*;
pub use conversations::*;
pub use evidence::*;
pub use horizon::*;
pub use knowledge::*;
pub use memory::*;
pub use messages::*;
pub use observations::*;
pub use providers::*;
pub use run_checkpoints::*;
pub use runs::*;
pub use tools::*;
