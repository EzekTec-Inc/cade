use super::*;

pub fn upsert_memory_block(
    db: &Db,
    agent_id: &str,
    label: &str,
    value: &str,
    description: Option<&str>,
    max_chars: Option<usize>,
) -> Result<()> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;

    // Fetch existing block linked to this agent with this label
    let existing: Option<(String, String, Option<usize>)> = conn
        .query_row(
            "SELECT b.id, b.value, b.max_chars FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
            params![agent_id, label],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?.map(|n| n as usize),
                ))
            },
        )
        .optional()?;

    // Effective limit: prefer caller-supplied, else stored, else none.
    let effective_limit = max_chars.or_else(|| existing.as_ref().and_then(|(_, _, mc)| *mc));

    // Apply size limit to the incoming value.
    let final_value: String = if let Some(limit) = effective_limit {
        let char_count = value.chars().count();
        if char_count > limit {
            return Err(crate::server::Error::custom(format!(
                "Memory block '{}' exceeds character limit ({} > {}). Please edit or summarize to fit.",
                label, char_count, limit
            )));
        }
        value.to_string()
    } else {
        value.to_string()
    };

    let ts = now_ts();
    // Get the agent's current turn counter so we can stamp last_turn on the block.
    let current_turn: i64 = conn
        .query_row(
            "SELECT COALESCE(memory_turn_counter, 0) FROM agents WHERE id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

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
                "UPDATE shared_memory_blocks
                 SET value = ?1, description = ?2, max_chars = ?3, updated_at = ?4,
                     last_turn = ?5,
                     tier = CASE WHEN tier = 'pinned' THEN 'pinned' ELSE 'short' END
                 WHERE id = ?6",
                params![
                    final_value,
                    desc,
                    max_chars.map(|n| n as i64),
                    ts,
                    current_turn,
                    block_id
                ],
            )?;
        } else {
            conn.execute(
                "UPDATE shared_memory_blocks
                 SET value = ?1, updated_at = ?2, last_turn = ?3,
                     tier = CASE WHEN tier = 'pinned' THEN 'pinned' ELSE 'short' END
                 WHERE id = ?4",
                params![final_value, ts, current_turn, block_id],
            )?;
        }
    } else {
        // Create a new shared block and link it to the agent
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO shared_memory_blocks (id, label, value, description, max_chars, updated_at, tier, last_turn)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'short', ?7)",
            params![id, label, final_value, description.unwrap_or(""),
                    max_chars.map(|n| n as i64), ts, current_turn],
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
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    conn.execute(
        "INSERT OR IGNORE INTO agent_memory_blocks (agent_id, block_id) VALUES (?1, ?2)",
        params![agent_id, block_id],
    )?;
    Ok(())
}

pub fn delete_memory_block(db: &Db, agent_id: &str, label: &str) -> Result<bool> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    // We only remove the link, not the shared block itself (to avoid orphan issues if shared)
    // Actually, CADE docs imply it's removed from the agent's view.
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
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT b.label, b.value, b.description FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 ORDER BY b.label",
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
pub fn get_memory_blocks_with_ts(
    db: &Db,
    agent_id: &str,
) -> Result<Vec<(String, String, String, i64)>> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT b.label, b.value, b.description, b.updated_at FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 ORDER BY b.updated_at DESC",
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

// -- Three-tier memory functions

/// Increment the agent's user-message turn counter and return the new value.
/// Call once per non-tool-return message (never for tool result turns).
pub fn increment_turn_counter(db: &Db, agent_id: &str) -> Result<i64> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    conn.execute(
        "UPDATE agents SET memory_turn_counter = memory_turn_counter + 1 WHERE id = ?1",
        params![agent_id],
    )?;
    let n: i64 = conn
        .query_row(
            "SELECT COALESCE(memory_turn_counter, 0) FROM agents WHERE id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(n)
}

/// Read the current turn counter without incrementing.
pub fn get_turn_counter(db: &Db, agent_id: &str) -> Result<i64> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let n: i64 = conn
        .query_row(
            "SELECT COALESCE(memory_turn_counter, 0) FROM agents WHERE id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(n)
}

/// Promote 'short' blocks idle for >= threshold turns to 'long'.
/// 'pinned' blocks are never promoted. Returns number of blocks promoted.
pub fn promote_stale_blocks(
    db: &Db,
    agent_id: &str,
    current_turn: i64,
    threshold: i64,
) -> Result<u64> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let n = conn.execute(
        "UPDATE shared_memory_blocks SET tier = 'long'
         WHERE tier = 'short'
           AND (? - last_turn) >= ?
           AND id IN (
               SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?
           )",
        params![current_turn, threshold, agent_id],
    )?;
    Ok(n as u64)
}

/// Fetch pinned + short-term blocks, pinned first then short by last_turn DESC.
/// Returns (label, value, description, tier, last_turn).
pub fn get_active_blocks(
    db: &Db,
    agent_id: &str,
) -> Result<Vec<(String, String, String, String, i64)>> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT b.label, b.value, b.description, b.tier, b.last_turn
         FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.tier IN ('pinned', 'short')
         ORDER BY CASE b.tier WHEN 'pinned' THEN 0 ELSE 1 END ASC, b.last_turn DESC",
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2).unwrap_or_default(),
            row.get::<_, String>(3)
                .unwrap_or_else(|_| "short".to_string()),
            row.get::<_, i64>(4).unwrap_or(0),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Fetch long-term blocks: label + first 80 chars of value, ordered by last_turn DESC.
/// Returns (label, excerpt, turns_idle) where turns_idle = current_turn - last_turn.
pub fn get_long_term_excerpts(
    db: &Db,
    agent_id: &str,
    current_turn: i64,
) -> Result<Vec<(String, String, i64)>> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT b.label, b.value, b.last_turn
         FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.tier = 'long'
         ORDER BY b.last_turn DESC",
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        let label: String = row.get(0)?;
        let value: String = row.get(1).unwrap_or_default();
        let last_turn: i64 = row.get(2).unwrap_or(0);
        // Take first 80 chars as excerpt
        let excerpt: String = value.chars().take(80).collect();
        let excerpt = if value.chars().count() > 80 {
            format!("{excerpt}…")
        } else {
            excerpt
        };
        Ok((label, excerpt, last_turn))
    })?;
    let rows: Vec<(String, String, i64)> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|(l, e, lt)| (l, e, current_turn - lt))
        .collect())
}

/// Explicitly set a block's tier and optionally reset last_turn to current_turn.
pub fn set_memory_tier(
    db: &Db,
    agent_id: &str,
    label: &str,
    tier: &str,
    reset_turn: bool,
) -> Result<bool> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let current_turn: i64 = conn
        .query_row(
            "SELECT COALESCE(memory_turn_counter, 0) FROM agents WHERE id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let n = if reset_turn {
        conn.execute(
            "UPDATE shared_memory_blocks SET tier = ?1, last_turn = ?2
             WHERE label = ?3 AND id IN (
                 SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?4
             )",
            params![tier, current_turn, label, agent_id],
        )?
    } else {
        conn.execute(
            "UPDATE shared_memory_blocks SET tier = ?1
             WHERE label = ?2 AND id IN (
                 SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?3
             )",
            params![tier, label, agent_id],
        )?
    };
    Ok(n > 0)
}

/// Returns (label, value, description, tier) for all blocks, ordered by tier priority then label.
/// Used by the API get_memory endpoint to expose tier information.
pub fn get_memory_blocks_full(
    db: &Db,
    agent_id: &str,
) -> Result<Vec<(String, String, String, String)>> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let mut stmt = conn.prepare(
        "SELECT b.label, b.value, b.description, b.tier
         FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1
         ORDER BY CASE b.tier WHEN 'pinned' THEN 0 WHEN 'short' THEN 1 ELSE 2 END, b.last_turn DESC"
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2).unwrap_or_default(),
            row.get::<_, String>(3)
                .unwrap_or_else(|_| "short".to_string()),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Returns the last N revisions of a memory block: (id, value, updated_at).
pub fn get_memory_history(
    db: &Db,
    agent_id: &str,
    label: &str,
    limit: usize,
) -> Result<Vec<(String, String, i64)>> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let block_id: Option<String> = conn
        .query_row(
            "SELECT b.id FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
            params![agent_id, label],
            |r| r.get(0),
        )
        .optional()?;
    let Some(block_id) = block_id else {
        return Ok(vec![]);
    };
    let mut stmt = conn.prepare(
        "SELECT id, value, updated_at FROM memory_history
         WHERE block_id = ?1 ORDER BY updated_at DESC LIMIT ?2",
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
pub fn restore_memory_from_history(
    db: &Db,
    agent_id: &str,
    label: &str,
    hist_id: &str,
) -> Result<bool> {
    let conn = db
        .lock()
        .map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
    let block_id: Option<String> = conn
        .query_row(
            "SELECT b.id FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
            params![agent_id, label],
            |r| r.get(0),
        )
        .optional()?;
    let Some(block_id) = block_id else {
        return Ok(false);
    };
    let hist_value: Option<String> = conn
        .query_row(
            "SELECT value FROM memory_history WHERE id = ?1 AND block_id = ?2",
            params![hist_id, block_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(hist_value) = hist_value else {
        return Ok(false);
    };
    conn.execute(
        "UPDATE shared_memory_blocks SET value = ?1, updated_at = ?2 WHERE id = ?3",
        params![hist_value, now_ts(), block_id],
    )?;
    Ok(true)
}

// -- Tools

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub source_code: Option<String>,
    pub json_schema: Option<Value>,
    pub tags: Vec<String>,
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

    fn make_agent(db: &Db, id: &str) -> Result<()> {
        agents::create_agent(
            db,
            &AgentRow {
                id: id.into(),
                name: "A".into(),
                model: "m".into(),
                description: None,
                system_prompt: None,
                created_at: None,
            },
        )?;
        Ok(())
    }

    #[test]
    fn test_upsert_and_get_memory_block() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "project", "Rust app", Some("about"), None)?;

        let blocks = get_memory_blocks(&db, "a1")?;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, "project"); // label
        assert_eq!(blocks[0].1, "Rust app"); // value
        assert_eq!(blocks[0].2, "about"); // description
        Ok(())
    }

    #[test]
    fn test_upsert_memory_block_update() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "project", "v1", None, None)?;
        upsert_memory_block(&db, "a1", "project", "v2", Some("updated"), None)?;

        let blocks = get_memory_blocks(&db, "a1")?;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].1, "v2");
        assert_eq!(blocks[0].2, "updated");
        Ok(())
    }

    #[test]
    fn test_delete_memory_block() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "project", "data", None, None)?;
        assert!(delete_memory_block(&db, "a1", "project")?);
        assert!(get_memory_blocks(&db, "a1")?.is_empty());
        assert!(!delete_memory_block(&db, "a1", "nope")?);
        Ok(())
    }

    #[test]
    fn test_get_memory_blocks_with_ts() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "block1", "value1", None, None)?;
        let blocks = get_memory_blocks_with_ts(&db, "a1")?;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, "block1"); // label
        assert_eq!(blocks[0].1, "value1"); // value
        assert!(blocks[0].3 > 0); // updated_at timestamp
        Ok(())
    }

    #[test]
    fn test_increment_and_get_turn_counter() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        assert_eq!(get_turn_counter(&db, "a1")?, 0);
        let t1 = increment_turn_counter(&db, "a1")?;
        assert_eq!(t1, 1);
        let t2 = increment_turn_counter(&db, "a1")?;
        assert_eq!(t2, 2);
        assert_eq!(get_turn_counter(&db, "a1")?, 2);
        Ok(())
    }

    #[test]
    fn test_set_memory_tier() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "block1", "data", None, None)?;

        // Set tier to 'long'
        let ok = set_memory_tier(&db, "a1", "block1", "long", false)?;
        assert!(ok);

        // Verify via get_memory_blocks_full
        let full = get_memory_blocks_full(&db, "a1")?;
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].3, "long"); // tier

        // Set tier for missing label
        let ok = set_memory_tier(&db, "a1", "nope", "long", false)?;
        assert!(!ok);
        Ok(())
    }

    #[test]
    fn test_get_active_blocks() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "active1", "val1", None, None)?;
        upsert_memory_block(&db, "a1", "active2", "val2", None, None)?;

        // Default tier is 'short' — both should show as active
        let active = get_active_blocks(&db, "a1")?;
        assert_eq!(active.len(), 2);
        Ok(())
    }

    #[test]
    fn test_get_long_term_excerpts() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "block1", "some long data here", None, None)?;
        set_memory_tier(&db, "a1", "block1", "long", false)?;

        let turn = get_turn_counter(&db, "a1")?;
        let excerpts = get_long_term_excerpts(&db, "a1", turn)?;
        assert_eq!(excerpts.len(), 1);
        assert_eq!(excerpts[0].0, "block1"); // label
        Ok(())
    }

    #[test]
    fn test_promote_stale_blocks() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "block1", "data", None, None)?;

        // Advance turn counter way past the block's last_turn
        for _ in 0..50 {
            increment_turn_counter(&db, "a1")?;
        }
        let current_turn = get_turn_counter(&db, "a1")?;

        // Promote blocks that are 40+ turns stale
        let promoted = promote_stale_blocks(&db, "a1", current_turn, 40)?;
        assert!(promoted >= 1);

        // Verify block is now 'long' tier
        let full = get_memory_blocks_full(&db, "a1")?;
        assert_eq!(full[0].3, "long");
        Ok(())
    }

    #[test]
    fn test_memory_history() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Upsert creates history entries
        upsert_memory_block(&db, "a1", "project", "v1", None, None)?;
        upsert_memory_block(&db, "a1", "project", "v2", None, None)?;
        upsert_memory_block(&db, "a1", "project", "v3", None, None)?;

        let history = get_memory_history(&db, "a1", "project", 10)?;
        // Should have at least the update entries
        assert!(!history.is_empty());
        Ok(())
    }

    #[test]
    fn test_restore_memory_from_history() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "project", "original", None, None)?;
        upsert_memory_block(&db, "a1", "project", "modified", None, None)?;

        // Get history — the first entry should be the "original" value
        let history = get_memory_history(&db, "a1", "project", 10)?;
        if !history.is_empty() {
            let hist_id = &history[history.len() - 1].0; // oldest entry
            let ok = restore_memory_from_history(&db, "a1", "project", hist_id)?;
            assert!(ok);
        }
        Ok(())
    }

    #[test]
    fn test_get_memory_blocks_full() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        upsert_memory_block(&db, "a1", "b1", "val1", Some("desc1"), None)?;
        upsert_memory_block(&db, "a1", "b2", "val2", Some("desc2"), None)?;

        let full = get_memory_blocks_full(&db, "a1")?;
        assert_eq!(full.len(), 2);
        // Each tuple is (label, value, description, tier)
        let labels: Vec<&str> = full.iter().map(|f| f.0.as_str()).collect();
        assert!(labels.contains(&"b1"));
        assert!(labels.contains(&"b2"));
        Ok(())
    }
}

// endregion: --- Tests
