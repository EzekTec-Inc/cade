use super::*;

#[allow(clippy::too_many_arguments)]
pub fn upsert_memory_block_typed(
    db: &Db,
    agent_id: &str,
    label: &str,
    value: &str,
    description: Option<&str>,
    max_chars: Option<usize>,
    memory_type: Option<&str>,
    confidence: Option<f64>,
) -> Result<()> {
    // Core upsert first
    upsert_memory_block(db, agent_id, label, value, description, max_chars)?;

    // Update typed columns (safe — ALTER TABLE already ran in migration 14)
    if memory_type.is_some() || confidence.is_some() {
        let conn = db.lock().expect("db lock poisoned");
        if let Some(mt) = memory_type {
            let _ = conn.execute(
                "UPDATE shared_memory_blocks SET memory_type = ?1 WHERE label = ?2",
                params![mt, label],
            );
        }
        if let Some(c) = confidence {
            let _ = conn.execute(
                "UPDATE shared_memory_blocks SET confidence = ?1 WHERE label = ?2",
                params![c, label],
            );
        }
    }
    Ok(())
}

/// Insert a memory evidence entry for a block.
pub fn insert_memory_evidence(
    db: &Db,
    agent_id: &str,
    label: &str,
    kind: &str,
    reference: &str,
    excerpt: Option<&str>,
    confidence: f64,
) -> Result<String> {
    let conn = db.lock().expect("db lock poisoned");

    // Find the block_id
    let block_id: Option<String> = conn
        .query_row(
            "SELECT b.id FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2 LIMIT 1",
            params![agent_id, label],
            |r| r.get(0),
        )
        .optional()?;

    let Some(block_id) = block_id else {
        return Err(crate::server::Error::custom(format!(
            "Memory block '{label}' not found for agent {agent_id}"
        )));
    };

    let id = format!("ev-{}", uuid::Uuid::new_v4());
    conn.execute(
        "INSERT OR IGNORE INTO memory_evidence (id, block_id, kind, reference, excerpt, confidence, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, block_id, kind, reference, excerpt, confidence, now_ts()],
    )?;
    Ok(id)
}

/// List evidence entries for a memory block.
pub fn list_memory_evidence(
    db: &Db,
    agent_id: &str,
    label: &str,
) -> Result<Vec<(String, String, String, Option<String>, f64, i64)>> {
    let conn = db.lock().expect("db lock poisoned");
    let mut stmt = conn.prepare(
        "SELECT e.id, e.kind, e.reference, e.excerpt, e.confidence, e.created_at
         FROM memory_evidence e
         JOIN shared_memory_blocks b ON b.id = e.block_id
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2
         ORDER BY e.created_at DESC LIMIT 20",
    )?;
    let rows = stmt.query_map(params![agent_id, label], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, f64>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Insert a reflection log entry.
#[allow(clippy::too_many_arguments)]
pub fn insert_reflection_log(
    db: &Db,
    id: &str,
    agent_id: &str,
    trigger: &str,
    blocks_created: usize,
    blocks_updated: usize,
    summary: &str,
    duration_ms: u128,
) -> Result<()> {
    let conn = db.lock().expect("db lock poisoned");
    conn.execute(
        "INSERT OR IGNORE INTO reflection_log
         (id, agent_id, trigger, blocks_created, blocks_updated, summary, duration_ms, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            agent_id,
            trigger,
            blocks_created as i64,
            blocks_updated as i64,
            summary,
            duration_ms as i64,
            now_ts()
        ],
    )?;
    Ok(())
}

/// List reflection log entries for an agent.
pub fn list_reflection_log(db: &Db, agent_id: &str) -> Result<Vec<serde_json::Value>> {
    let conn = db.lock().expect("db lock poisoned");
    let mut stmt = conn.prepare(
        "SELECT id, trigger, blocks_created, blocks_updated, summary, duration_ms, created_at
         FROM reflection_log WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT 50",
    )?;
    let rows = stmt.query_map(params![agent_id], |r| {
        Ok(serde_json::json!({
            "id":             r.get::<_, String>(0)?,
            "trigger":        r.get::<_, String>(1)?,
            "blocks_created": r.get::<_, i64>(2)?,
            "blocks_updated": r.get::<_, i64>(3)?,
            "summary":        r.get::<_, Option<String>>(4)?,
            "duration_ms":    r.get::<_, Option<i64>>(5)?,
            "created_at":     r.get::<_, i64>(6)?,
        }))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

// endregion: --- Typed memory + provenance + reflection helpers

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

    use super::*;
    use serde_json::json;

    fn setup_mem_db() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        apply_schema(&conn)?;
        run_migrations(&conn)?;
        Ok(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn test_sqlite_shared_memory() -> Result<()> {
        let db = setup_mem_db()?;
        let agent1 = "agent-1";
        let agent2 = "agent-2";

        // Create agents
        create_agent(
            &db,
            &AgentRow {
                id: agent1.to_string(),
                name: "A1".to_string(),
                model: "m".to_string(),
                description: None,
                system_prompt: None,
                created_at: None,
            },
        )?;
        create_agent(
            &db,
            &AgentRow {
                id: agent2.to_string(),
                name: "A2".to_string(),
                model: "m".to_string(),
                description: None,
                system_prompt: None,
                created_at: None,
            },
        )?;

        // 1. Agent 1 creates a block
        upsert_memory_block(&db, agent1, "shared_fact", "Initial value", None, None)?;

        // Find the block ID
        let block_id: String = {
            let conn = db.lock().unwrap(); // Keep this one unwrap() as it's a Mutex poison error which is fine, or use lock().map_err(|e| e.to_string())?
            conn.query_row(
                "SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?1",
                params![agent1],
                |r| r.get(0),
            )?
        };

        // 2. Link Agent 2 to the same block
        link_shared_memory_block(&db, agent2, &block_id)?;

        // 3. Verify both see the same value
        let b1 = get_memory_blocks(&db, agent1)?;
        let b2 = get_memory_blocks(&db, agent2)?;
        assert_eq!(b1[0].1, "Initial value");
        assert_eq!(b2[0].1, "Initial value");

        // 4. Agent 2 updates the block
        upsert_memory_block(&db, agent2, "shared_fact", "Updated by A2", None, None)?;

        // 5. Verify Agent 1 sees the update
        let b1_new = get_memory_blocks(&db, agent1)?;
        assert_eq!(b1_new[0].1, "Updated by A2");

        Ok(())
    }

    #[test]
    fn test_sqlite_archival_memory_fts() -> Result<()> {
        let db = setup_mem_db()?;
        let agent_id = "agent-fts";

        create_agent(
            &db,
            &AgentRow {
                id: agent_id.to_string(),
                name: "A".to_string(),
                model: "m".to_string(),
                description: None,
                system_prompt: None,
                created_at: None,
            },
        )?;

        insert_message(
            &db,
            &MessageRow {
                id: "m1".to_string(),
                agent_id: agent_id.to_string(),
                conversation_id: None,
                role: "user".to_string(),
                content: json!("Rust is a systems programming language"),
                char_count: 0,
            },
        )?;

        insert_message(
            &db,
            &MessageRow {
                id: "m2".to_string(),
                agent_id: agent_id.to_string(),
                conversation_id: None,
                role: "assistant".to_string(),
                content: json!("I agree, Rust is safe and fast."),
                char_count: 0,
            },
        )?;

        // Search for "systems"
        let res = search_messages(&db, agent_id, "systems", None)?;
        assert_eq!(res.len(), 1);
        assert!(
            res[0]
                .content
                .as_str()
                .ok_or("not string")?
                .contains("systems")
        );

        // Search for "safe"
        let res2 = search_messages(&db, agent_id, "safe", None)?;
        assert_eq!(res2.len(), 1);
        assert!(
            res2[0]
                .content
                .as_str()
                .ok_or("not string")?
                .contains("fast")
        );

        Ok(())
    }

    #[test]
    fn test_sqlite_migration_8_removes_stale_providers() -> Result<()> {
        // Build a DB with schema but WITHOUT running migrations yet
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        apply_schema(&conn)?;

        // Insert a provider with a valid encrypted key
        let valid_key = crate::server::crypto::encrypt("sk-real-key")?;
        conn.execute(
            "INSERT INTO providers (name, kind, api_key, base_url, enabled, created_at)
             VALUES ('good', 'anthropic', ?1, NULL, 1, 0)",
            params![valid_key],
        )?;

        // Insert a provider with garbage that cannot be decrypted
        conn.execute(
            "INSERT INTO providers (name, kind, api_key, base_url, enabled, created_at)
             VALUES ('stale', 'openai', 'not-a-real-encrypted-value', NULL, 1, 0)",
            params![],
        )?;

        // Insert a provider with NULL api_key (e.g. ollama) — should survive
        conn.execute(
            "INSERT INTO providers (name, kind, api_key, base_url, enabled, created_at)
             VALUES ('ollama', 'ollama', NULL, 'http://localhost:11434', 1, 0)",
            params![],
        )?;

        // Verify 3 rows before migration
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM providers", [], |r| r.get(0))?;
        assert_eq!(count, 3);

        // Run migrations — migration 8 should remove 'stale'
        run_migrations(&conn)?;

        // Verify: 'stale' removed, 'good' and 'ollama' survive
        let remaining: Vec<String> = {
            let mut stmt = conn.prepare("SELECT name FROM providers ORDER BY name")?;
            stmt.query_map([], |r| r.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(remaining, vec!["good", "ollama"]);

        Ok(())
    }
}

// endregion: --- Tests
