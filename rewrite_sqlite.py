import re

with open("src/server/storage/sqlite.rs", "r") as f:
    content = f.read()

# Replace run_migrations and apply_schema
pattern = re.compile(r"fn run_migrations\(conn: &Connection\) -> Result<\(\)> \{.*?\n\}\n\nfn apply_schema\(conn: &Connection\) -> Result<\(\)> \{.*?\n\}", re.DOTALL)

new_code = r"""struct Migration {
    version: i64,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: "
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
            INSERT OR IGNORE INTO memory_blocks_new
                SELECT id, agent_id, label, value, updated_at FROM (
                    SELECT *, ROW_NUMBER() OVER (
                        PARTITION BY agent_id, label ORDER BY updated_at DESC
                    ) AS rn FROM memory_blocks
                ) WHERE rn = 1;
            DROP TABLE memory_blocks;
            ALTER TABLE memory_blocks_new RENAME TO memory_blocks;
            COMMIT;
        ",
    },
    Migration {
        version: 2,
        sql: "ALTER TABLE memory_blocks ADD COLUMN description TEXT NOT NULL DEFAULT '';",
    },
    Migration {
        version: 3,
        sql: "ALTER TABLE memory_blocks ADD COLUMN max_chars INTEGER;",
    },
    Migration {
        version: 4,
        sql: "
            CREATE TABLE IF NOT EXISTS memory_history (
                id         TEXT PRIMARY KEY,
                block_id   TEXT NOT NULL,
                value      TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memory_history_block_id ON memory_history(block_id, updated_at DESC);
        ",
    },
    Migration {
        version: 5,
        sql: "
            ALTER TABLE messages ADD COLUMN conversation_id TEXT;
            CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(agent_id, conversation_id);
        ",
    },
    Migration {
        version: 6,
        sql: "
            BEGIN;
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
            INSERT OR IGNORE INTO shared_memory_blocks (id, label, value, description, max_chars, updated_at)
                SELECT id, label, value, description, max_chars, updated_at FROM memory_blocks;
            INSERT OR IGNORE INTO agent_memory_blocks (agent_id, block_id)
                SELECT agent_id, id FROM memory_blocks;
            COMMIT;
        ",
    },
    Migration {
        version: 7,
        sql: "
            ALTER TABLE agents ADD COLUMN memory_turn_counter INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE shared_memory_blocks ADD COLUMN last_turn INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE shared_memory_blocks ADD COLUMN tier TEXT NOT NULL DEFAULT 'short';
        ",
    },
    Migration {
        version: 8,
        sql: "
            BEGIN;
            CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                content,
                content='messages',
                content_rowid='rowid'
            );
            INSERT OR IGNORE INTO messages_fts(rowid, content) SELECT rowid, content FROM messages;
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
            COMMIT;
        ",
    },
];

fn detect_legacy_version(conn: &Connection) -> i64 {
    let has_fts: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages_fts'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if has_fts { return 8; }
    let has_tier: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('shared_memory_blocks') WHERE name='tier'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if has_tier { return 7; }
    let has_shared: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='shared_memory_blocks'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if has_shared { return 6; }
    let has_conv: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('messages') WHERE name='conversation_id'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if has_conv { return 5; }
    let has_hist: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memory_history'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if has_hist { return 4; }
    let has_max: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('memory_blocks') WHERE name='max_chars'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if has_max { return 3; }
    let has_desc: bool = conn.query_row("SELECT COUNT(*) FROM pragma_table_info('memory_blocks') WHERE name='description'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if has_desc { return 2; }
    let has_unique: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE tbl_name='memory_blocks' AND ((type='index' AND sql IS NOT NULL AND (sql LIKE '%agent_id%label%' OR sql LIKE '%label%agent_id%')) OR (type='table' AND sql LIKE '%UNIQUE%agent_id%label%'))", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
    if has_unique { return 1; }
    0
}

fn run_migrations(conn: &Connection) -> Result<()> {
    let mut current_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);

    if current_version == 0 {
        let has_agents: bool = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agents'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) > 0;
        if has_agents {
            current_version = detect_legacy_version(conn);
        }
    }

    for migration in MIGRATIONS {
        if current_version < migration.version {
            tracing::info!("Running migration version {}", migration.version);
            if let Err(e) = conn.execute_batch(migration.sql) {
                tracing::error!("Migration {} failed: {}", migration.version, e);
                return Err(anyhow::anyhow!("Migration {} failed: {}", migration.version, e));
            }
            current_version = migration.version;
            conn.execute(&format!("PRAGMA user_version = {}", current_version), [])?;
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
            role            TEXT NOT NULL,
            content         TEXT NOT NULL,
            created_at      INTEGER NOT NULL,
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
}"""

if pattern.search(content):
    content = pattern.sub(new_code, content)
    with open("src/server/storage/sqlite.rs", "w") as f:
        f.write(content)
    print("Successfully replaced apply_schema and run_migrations.")
else:
    print("Pattern not found!")
