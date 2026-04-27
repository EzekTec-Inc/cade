use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Shared / standalone block types & operations
// ─────────────────────────────────────────────────────────────────────────────

/// Info about a memory block, independent of any agent attachment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlockInfo {
    pub id: String,
    pub label: String,
    pub value: String,
    pub description: String,
    pub tier: String,
    pub max_chars: Option<usize>,
    pub updated_at: i64,
}

/// Create a standalone block with NO agent attachment.
/// Returns the new block ID (UUID).
pub fn create_standalone_block(
    db: &Db,
    label: &str,
    value: &str,
    description: Option<&str>,
    max_chars: Option<usize>,
) -> Result<String> {
    let conn = db.lock();
    let id = uuid::Uuid::new_v4().to_string();
    let ts = now_ts();
    conn.execute(
        "INSERT INTO shared_memory_blocks (id, label, value, description, max_chars, updated_at, tier)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'short')",
        params![id, label, value, description.unwrap_or(""), max_chars.map(|n| n as i64), ts],
    )?;
    Ok(id)
}

/// Fetch a block by its primary-key ID, regardless of agent attachment.
pub fn get_block_by_id(db: &Db, block_id: &str) -> Result<Option<BlockInfo>> {
    let conn = db.lock();
    conn.query_row(
        "SELECT id, label, value, description, tier, max_chars, updated_at
         FROM shared_memory_blocks WHERE id = ?1",
        params![block_id],
        |r| {
            Ok(BlockInfo {
                id: r.get(0)?,
                label: r.get(1)?,
                value: r.get(2)?,
                description: r.get::<_, String>(3).unwrap_or_default(),
                tier: r.get::<_, String>(4).unwrap_or_else(|_| "short".to_string()),
                max_chars: r.get::<_, Option<i64>>(5)?.map(|n| n as usize),
                updated_at: r.get(6)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// List all blocks in the system. Optional exact-match label filter.
/// Ordered by updated_at DESC.
pub fn list_all_blocks(db: &Db, label_filter: Option<&str>) -> Result<Vec<BlockInfo>> {
    let conn = db.lock();
    if let Some(label) = label_filter {
        let mut stmt = conn.prepare(
            "SELECT id, label, value, description, tier, max_chars, updated_at
             FROM shared_memory_blocks WHERE label = ?1 ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map(params![label], block_info_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, label, value, description, tier, max_chars, updated_at
             FROM shared_memory_blocks ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], block_info_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Permanently delete a block from the system. FK CASCADE removes all
/// agent_memory_blocks junction rows automatically.
pub fn delete_block_permanently(db: &Db, block_id: &str) -> Result<bool> {
    let conn = db.lock();
    let n = conn.execute(
        "DELETE FROM shared_memory_blocks WHERE id = ?1",
        params![block_id],
    )?;
    Ok(n > 0)
}

/// Remove the link between an agent and a block (by block_id).
/// Does NOT delete the block itself.
pub fn unlink_shared_memory_block(db: &Db, agent_id: &str, block_id: &str) -> Result<bool> {
    let conn = db.lock();
    let n = conn.execute(
        "DELETE FROM agent_memory_blocks WHERE agent_id = ?1 AND block_id = ?2",
        params![agent_id, block_id],
    )?;
    Ok(n > 0)
}

/// Return the list of agent IDs that have this block attached.
pub fn list_agents_for_block(db: &Db, block_id: &str) -> Result<Vec<String>> {
    let conn = db.lock();
    let mut stmt = conn.prepare(
        "SELECT agent_id FROM agent_memory_blocks WHERE block_id = ?1 ORDER BY agent_id",
    )?;
    let rows = stmt.query_map(params![block_id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Like `get_memory_blocks` but includes the block_id as the first tuple element.
/// Returns (block_id, label, value, description) ordered by label.
pub fn get_memory_blocks_with_ids(
    db: &Db,
    agent_id: &str,
) -> Result<Vec<(String, String, String, String)>> {
    let conn = db.lock();
    let mut stmt = conn.prepare(
        "SELECT b.id, b.label, b.value, b.description FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 ORDER BY b.label",
    )?;
    let rows = stmt.query_map(params![agent_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3).unwrap_or_default(),
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Row mapper shared by list_all_blocks queries.
fn block_info_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<BlockInfo> {
    Ok(BlockInfo {
        id: r.get(0)?,
        label: r.get(1)?,
        value: r.get(2)?,
        description: r.get::<_, String>(3).unwrap_or_default(),
        tier: r.get::<_, String>(4).unwrap_or_else(|_| "short".to_string()),
        max_chars: r.get::<_, Option<i64>>(5)?.map(|n| n as usize),
        updated_at: r.get(6)?,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent-scoped memory operations (existing)
// ─────────────────────────────────────────────────────────────────────────────

pub fn upsert_memory_block(
    db: &Db,
    agent_id: &str,
    label: &str,
    value: &str,
    description: Option<&str>,
    max_chars: Option<usize>,
) -> Result<()> {
    let conn = db
        .lock();

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
            return Err(crate::error::Error::custom(format!(
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

    // M1: auto-pin `active_goal` the first time it receives a non-empty value.
    // The block is seeded as `short` (see DEFAULT_MEMORY_BLOCKS) so it can
    // age out when the agent moves on to a new task, but once the agent has
    // written real task state the block must survive `promote_stale_blocks`
    // until consolidation explicitly manages it. Without this, a long session
    // (≥80 idle turns between active_goal writes) would archive the block
    // before `consolidate_agent` could pin it.
    let is_nonempty_active_goal = label == "active_goal" && !final_value.trim().is_empty();

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

        // Tier transition rule for UPDATE:
        //   * already pinned → stay pinned
        //   * active_goal with non-empty value → pinned (M1)
        //   * else → short
        let tier_sql = if is_nonempty_active_goal {
            "'pinned'"
        } else {
            "CASE WHEN tier = 'pinned' THEN 'pinned' ELSE 'short' END"
        };

        if let Some(desc) = description {
            let sql = format!(
                "UPDATE shared_memory_blocks
                 SET value = ?1, description = ?2, max_chars = ?3, updated_at = ?4,
                     last_turn = ?5,
                     tier = {tier_sql}
                 WHERE id = ?6"
            );
            conn.execute(
                &sql,
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
            let sql = format!(
                "UPDATE shared_memory_blocks
                 SET value = ?1, updated_at = ?2, last_turn = ?3,
                     tier = {tier_sql}
                 WHERE id = ?4"
            );
            conn.execute(
                &sql,
                params![final_value, ts, current_turn, block_id],
            )?;
        }
    } else {
        // Create a new shared block and link it to the agent.
        // INSERT tier: `pinned` for non-empty active_goal (M1), else `short`.
        let insert_tier = if is_nonempty_active_goal { "pinned" } else { "short" };
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO shared_memory_blocks (id, label, value, description, max_chars, updated_at, tier, last_turn)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, label, final_value, description.unwrap_or(""),
                    max_chars.map(|n| n as i64), ts, insert_tier, current_turn],
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
        .lock();
    conn.execute(
        "INSERT OR IGNORE INTO agent_memory_blocks (agent_id, block_id) VALUES (?1, ?2)",
        params![agent_id, block_id],
    )?;
    Ok(())
}

pub fn delete_memory_block(db: &Db, agent_id: &str, label: &str) -> Result<bool> {
    let conn = db
        .lock();
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

/// Confidence threshold above which a block resists archival demotion.
/// Blocks with confidence >= this value stay in 'short' tier even when
/// chronologically stale.
pub const CONFIDENCE_RETENTION_THRESHOLD: f64 = 1.5;

/// Confidence increment applied each time a block is returned by search_memory.
pub const CONFIDENCE_BOOST_PER_HIT: f64 = 0.15;

/// Increment the confidence score for a memory block (called on search hit).
pub fn boost_confidence(db: &Db, agent_id: &str, label: &str) -> Result<bool> {
    let conn = db
        .lock();
    let n = conn.execute(
        "UPDATE shared_memory_blocks
         SET confidence = confidence + ?1
         WHERE label = ?2
           AND id IN (SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?3)",
        params![CONFIDENCE_BOOST_PER_HIT, label, agent_id],
    )?;
    Ok(n > 0)
}

/// Read the current confidence value for a memory block (used in tests).
pub fn get_block_confidence(db: &Db, agent_id: &str, label: &str) -> Result<f64> {
    let conn = db
        .lock();
    let confidence: f64 = conn.query_row(
        "SELECT b.confidence FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = ?2",
        params![agent_id, label],
        |row| row.get(0),
    )?;
    Ok(confidence)
}

/// Returns (label, value, description) tuples ordered by label.
pub fn get_memory_blocks(db: &Db, agent_id: &str) -> Result<Vec<(String, String, String)>> {
    let conn = db
        .lock();
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
        .lock();
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
        .lock();
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
        .lock();
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
        .lock();
    let n = conn.execute(
        "UPDATE shared_memory_blocks SET tier = 'long'
         WHERE tier = 'short'
           AND (? - last_turn) >= ?
           AND confidence < ?
           AND id IN (
               SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?
           )",
        params![
            current_turn,
            threshold,
            CONFIDENCE_RETENTION_THRESHOLD,
            agent_id
        ],
    )?;
    Ok(n as u64)
}

/// Fetch pinned + short-term blocks, pinned first then short by last_turn DESC.
/// Returns (label, value, description, tier, last_turn).
#[allow(clippy::type_complexity)]
pub fn get_active_blocks(
    db: &Db,
    agent_id: &str,
) -> Result<Vec<(String, String, String, String, i64)>> {
    let conn = db
        .lock();
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
        .lock();
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
        .lock();
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
        .lock();
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
        .lock();
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
        .lock();
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

// ─────────────────────────────────────────────────────────────────────────────
// Phase B — export memory to a directory indexable by cade-rag-mcp
// ─────────────────────────────────────────────────────────────────────────────
//
// cade-rag-mcp is an external MCP server the user may have attached; it walks
// a directory tree and builds a semantic index with fastembed. By writing each
// memory block as a markdown file with YAML front-matter, we get semantic
// recall over memory for free — without pulling the embedding stack into CADE
// itself, and without losing anything if cade-rag-mcp isn't running.
//
// The export is **one-way** (SQLite → filesystem). The filesystem copy is a
// read-only index surface. Re-imports are not supported — the DB remains the
// source of truth.

/// Escape a label for use as a filename stem. Strips path separators and any
/// control characters; replaces with `_`.
fn safe_stem(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Write `contents` to `path` atomically (write to `.tmp`, fsync, rename).
/// Creates parent directories if needed.
fn atomic_write(path: &std::path::Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::error::Error::custom(format!("create_dir_all {}: {e}", parent.display()))
        })?;
    }
    let tmp = path.with_extension("md.tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(|e| {
            crate::error::Error::custom(format!("create {}: {e}", tmp.display()))
        })?;
        f.write_all(contents.as_bytes()).map_err(|e| {
            crate::error::Error::custom(format!("write {}: {e}", tmp.display()))
        })?;
        f.sync_all().ok(); // best-effort durability
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        crate::error::Error::custom(format!(
            "rename {} → {}: {e}",
            tmp.display(),
            path.display()
        ))
    })?;
    Ok(())
}

/// Summary of one export run.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MemoryExportReport {
    pub blocks_written: usize,
    pub archival_written: usize,
    pub out_dir: String,
}

/// Export every memory block and archival entry for `agent_id` to `out_dir`
/// as markdown files with YAML front-matter. Call after consolidation, or on
/// demand via `/memory export`. Existing files for the agent are **removed**
/// first so deleted labels don't linger in the index.
///
/// Layout:
///   <out_dir>/blocks/<label>.md         — one per memory block
///   <out_dir>/archival/<archival_id>.md — one per archival entry
pub fn export_memory_to_rag_dir(
    db: &Db,
    agent_id: &str,
    out_dir: &std::path::Path,
) -> Result<MemoryExportReport> {
    // Clear any previous export so stale files don't outlive their blocks.
    let blocks_dir = out_dir.join("blocks");
    let archival_dir = out_dir.join("archival");
    if blocks_dir.exists() {
        let _ = std::fs::remove_dir_all(&blocks_dir);
    }
    if archival_dir.exists() {
        let _ = std::fs::remove_dir_all(&archival_dir);
    }
    std::fs::create_dir_all(&blocks_dir).map_err(|e| {
        crate::error::Error::custom(format!("mkdir {}: {e}", blocks_dir.display()))
    })?;
    std::fs::create_dir_all(&archival_dir).map_err(|e| {
        crate::error::Error::custom(format!("mkdir {}: {e}", archival_dir.display()))
    })?;

    let mut report = MemoryExportReport {
        blocks_written: 0,
        archival_written: 0,
        out_dir: out_dir.display().to_string(),
    };

    // -- Memory blocks -----------------------------------------------------
    let full = get_memory_blocks_full(db, agent_id)?;
    for (label, value, desc, tier) in full {
        let stem = safe_stem(&label);
        if stem.is_empty() {
            continue;
        }
        let fm = format!(
            "---\nlabel: {}\ntier: {}\nagent_id: {}\nexported_at: {}\n{}---\n\n",
            label,
            tier,
            agent_id,
            now_ts(),
            if desc.is_empty() {
                String::new()
            } else {
                format!("description: {}\n", desc.replace('\n', " "))
            }
        );
        let body = format!("{fm}{value}\n");
        let path = blocks_dir.join(format!("{stem}.md"));
        atomic_write(&path, &body)?;
        report.blocks_written += 1;
    }

    // -- Archival entries --------------------------------------------------
    //
    // We read the archival_memory FTS5 virtual table directly — no existing
    // helper lists all rows for an agent, and we don't want one in the hot
    // path. Expected volume here is small-ish (hundreds to low thousands).
    let conn = db
        .lock();
    let mut stmt = conn.prepare(
        "SELECT id, content, tags, created_at FROM archival_memory WHERE agent_id = ?1",
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    for row in rows.filter_map(|r| r.ok()) {
        let (id, content, tags_json, created_at) = row;
        let stem = safe_stem(&id);
        if stem.is_empty() {
            continue;
        }
        let fm = format!(
            "---\nid: {id}\ntier: archival\nagent_id: {agent_id}\ncreated_at: {created_at}\ntags: {tags_json}\n---\n\n"
        );
        let body = format!("{fm}{content}\n");
        let path = archival_dir.join(format!("{stem}.md"));
        atomic_write(&path, &body)?;
        report.archival_written += 1;
    }

    Ok(report)
}

// region:    --- Tests

#[cfg(test)]
mod tests;

// endregion: --- Tests
