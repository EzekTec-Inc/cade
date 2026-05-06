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

/// A3: Rich archived excerpt with retrieval hints.
#[derive(Debug, Clone)]
pub struct LongTermExcerpt {
    pub label: String,
    pub excerpt: String,      // 250 chars
    pub keywords: Vec<String>, // top 5 distinctive terms
    pub char_count: usize,    // total chars in original value
    pub turns_idle: i64,
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

/// A3: Extract distinctive keywords from text using simple TF-IDF-like scoring.
/// Splits text on non-alphanumeric chars, lowercases, filters stop words,
/// scores by len × frequency, and returns top `max` unique words.
fn extract_keywords(text: &str, max: usize) -> Vec<String> {
    let stop_words: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "in", "on", "at", "to", "for", "of", "with",
        "and", "or", "but", "not", "this", "that", "it", "be", "as", "by", "from", "has", "had",
        "have", "which", "their", "they", "we", "you", "he", "she", "if", "do", "my", "no", "so",
        "up", "out", "all", "use", "can", "will", "one", "when", "than", "each", "its", "been",
        "who", "into", "may", "would", "could", "should", "some", "such", "also", "then", "just",
        "like", "other", "more", "about", "these", "those", "only", "very", "how", "after", "new",
        "any", "most", "what", "both", "did", "let", "get", "our", "his", "her"
    ];

    let mut word_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    
    // Split on non-alphanumeric chars, lowercase, filter stop words
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| !w.is_empty() && !stop_words.contains(&w.as_str()))
    {
        *word_counts.entry(word).or_insert(0) += 1;
    }

    // Score by len * frequency and take top `max`
    let mut scored: Vec<(String, usize)> = word_counts
        .into_iter()
        .map(|(word, freq)| (word.clone(), word.len() * freq))
        .collect();
    
    scored.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by score desc
    scored.into_iter().take(max).map(|(word, _)| word).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Agent-scoped memory operations (existing)
// ─────────────────────────────────────────────────────────────────────────────

/// Result of a memory block write operation (A2: write-ahead verification).
///
/// Callers can inspect `was_truncated` / `stored_chars` / `requested_chars`
/// to warn the agent that content was silently clipped, preventing
/// hallucination from partial data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteResult {
    /// `true` when the stored value is shorter than the requested value
    /// because it hit the `max_chars` limit (currently an error, but this
    /// struct is forward-compatible for auto-trim flows).
    pub was_truncated: bool,
    /// Number of chars actually persisted.
    pub stored_chars: usize,
    /// Number of chars the caller originally requested to write.
    pub requested_chars: usize,
}

pub fn upsert_memory_block(
    db: &Db,
    agent_id: &str,
    label: &str,
    value: &str,
    description: Option<&str>,
    max_chars: Option<usize>,
) -> Result<WriteResult> {
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

    // A1: Apply size limit — truncate instead of hard-erroring so the agent
    // always gets partial data + a `was_truncated` warning rather than nothing.
    let (final_value, was_truncated): (String, bool) = if let Some(limit) = effective_limit {
        let char_count = value.chars().count();
        if char_count > limit {
            let truncated: String = value.chars().take(limit).collect();
            (truncated, true)
        } else {
            (value.to_string(), false)
        }
    } else {
        (value.to_string(), false)
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

    // ── P6: Auto-type memory blocks on write ─────────────────────────────────
    // When no explicit memory_type is set, infer it from content heuristics.
    // This ensures durable knowledge (decisions, constraints, conventions) gets
    // the confidence boost that prevents the 80-turn archival cliff.
    auto_type_block_if_untyped(&conn, label);

    // ── Semantic search: compute and store embedding for this block ──────────
    // Semantic search feature removed (F5).

    let requested_chars = value.chars().count();
    let stored_chars = final_value.chars().count();
    Ok(WriteResult {
        was_truncated,
        stored_chars,
        requested_chars,
    })
}

/// Upsert a memory block and, if an embedder is provided, compute and
/// store its embedding as a packed little-endian f32 BLOB.
///
/// This is a thin wrapper over [`upsert_memory_block`] — it performs the
/// row write through the existing path (so all truncation, typing, and
/// access-tracking behaviour is unchanged) and then issues one extra
/// `UPDATE` to populate `shared_memory_blocks.embedding`.
///
/// `embedder = None` is equivalent to calling `upsert_memory_block`
/// directly; the embedding column stays NULL. This lets callers in the
/// default-feature build pass `None` and pay zero cost.
///
/// # Errors
///
/// Returns the same errors as [`upsert_memory_block`], plus any error
/// raised by the embedder or the BLOB UPDATE.
pub fn upsert_memory_block_with_embedder(
    db: &Db,
    agent_id: &str,
    label: &str,
    value: &str,
    description: Option<&str>,
    max_chars: Option<usize>,
    embedder: Option<&dyn crate::sqlite::embedding::Embedder>,
) -> Result<WriteResult> {
    let res = upsert_memory_block(db, agent_id, label, value, description, max_chars)?;

    if let Some(e) = embedder {
        let vec = e.embed(value)?;
        if !vec.is_empty() {
            let mut bytes: Vec<u8> = Vec::with_capacity(vec.len() * 4);
            for f in &vec {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
            let conn = db.lock();
            // Resolve the block id via the agent ↔ block link, then update.
            conn.execute(
                "UPDATE shared_memory_blocks
                 SET embedding = ?1
                 WHERE id = (SELECT b.id FROM shared_memory_blocks b
                             JOIN agent_memory_blocks amb ON amb.block_id = b.id
                             WHERE amb.agent_id = ?2 AND b.label = ?3)",
                params![bytes, agent_id, label],
            )?;
        }
    }

    Ok(res)
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

// ── A3: Provenance helpers ───────────────────────────────────────────────────

/// Stamp provenance metadata on a memory block after a write.
///
/// `source_turn` is the agent's turn counter at write time.
/// `source_tool_call_id` is the tool_call_id that triggered the write
/// (stored in the existing `source_te_id` column).
///
/// Both are optional — callers outside the agentic loop (e.g. consolidation,
/// agent creation) may not have a tool_call_id.
pub fn stamp_provenance(
    db: &Db,
    agent_id: &str,
    label: &str,
    source_turn: Option<i64>,
    source_tool_call_id: Option<&str>,
) {
    if source_turn.is_none() && source_tool_call_id.is_none() {
        return;
    }
    let conn = db.lock();
    let _ = conn.execute(
        "UPDATE shared_memory_blocks
         SET source_turn = COALESCE(?1, source_turn),
             source_te_id = COALESCE(?2, source_te_id)
         WHERE id = (
             SELECT b.id FROM shared_memory_blocks b
             JOIN agent_memory_blocks amb ON amb.block_id = b.id
             WHERE amb.agent_id = ?3 AND b.label = ?4
         )",
        params![source_turn, source_tool_call_id, agent_id, label],
    );
}

// ── A5: Semantic chunking ─────────────────────────────────────────────────────

/// Blocks shorter than this are not chunked (stored as a single unit).
pub const CHUNK_THRESHOLD: usize = 500;

/// Target chunk size in characters. Actual chunks may be slightly larger
/// due to sentence-boundary alignment.
const CHUNK_TARGET: usize = 300;

/// Overlap in characters between consecutive chunks so retrieval has
/// context from both sides of a chunk boundary.
const CHUNK_OVERLAP: usize = 50;

/// A single chunk produced by [`chunk_text`].
#[derive(Debug, Clone)]
pub struct TextChunk {
    pub index: usize,
    pub content: String,
}

/// Split `text` into overlapping chunks at sentence boundaries.
///
/// Sentences are detected by `. `, `! `, `? `, or `\n`. Each chunk
/// targets [`CHUNK_TARGET`] characters and overlaps the previous chunk
/// by [`CHUNK_OVERLAP`] characters. Short texts (below
/// [`CHUNK_THRESHOLD`]) produce a single chunk.
pub fn chunk_text(text: &str) -> Vec<TextChunk> {
    if text.chars().count() <= CHUNK_THRESHOLD {
        return vec![TextChunk {
            index: 0,
            content: text.to_string(),
        }];
    }

    // Collect sentence-end byte offsets.
    let delimiters = [". ", "! ", "? ", "\n"];
    let mut breaks: Vec<usize> = Vec::new();
    for delim in &delimiters {
        let mut start = 0;
        while let Some(pos) = text[start..].find(delim) {
            let abs = start + pos + delim.len();
            breaks.push(abs);
            start = abs;
        }
    }
    breaks.sort_unstable();
    breaks.dedup();
    // Ensure the end of text is always a break point.
    if breaks.last().copied() != Some(text.len()) {
        breaks.push(text.len());
    }

    let mut chunks: Vec<TextChunk> = Vec::new();
    let mut cursor: usize = 0;

    while cursor < text.len() {
        let target_end = (cursor + CHUNK_TARGET).min(text.len());

        // Find the nearest sentence break at or after target_end.
        let end = breaks
            .iter()
            .find(|&&b| b >= target_end)
            .copied()
            .unwrap_or(text.len());

        let chunk_str = &text[cursor..end];
        chunks.push(TextChunk {
            index: chunks.len(),
            content: chunk_str.to_string(),
        });

        if end >= text.len() {
            break;
        }

        // Next chunk starts OVERLAP chars before the end of this one.
        cursor = if end > CHUNK_OVERLAP {
            end - CHUNK_OVERLAP
        } else {
            end
        };
    }
    chunks
}

/// Replace all chunks for a block and insert new ones.
///
/// `block_id` is the `shared_memory_blocks.id` for the block that was
/// just written. This function:
/// 1. Deletes all existing chunks for the block.
/// 2. If `value` exceeds [`CHUNK_THRESHOLD`], splits it via [`chunk_text`]
///    and inserts the resulting rows into `memory_chunks`.
/// 3. If an embedder is provided, computes per-chunk embeddings.
///
/// Called automatically after `upsert_memory_block`.
pub fn rechunk_block(
    db: &Db,
    agent_id: &str,
    label: &str,
    value: &str,
    embedder: Option<&dyn crate::sqlite::embedding::Embedder>,
) {
    let conn = db.lock();

    // Resolve block_id.
    let block_id: Option<String> = conn
        .query_row(
            "SELECT b.id FROM shared_memory_blocks b
             JOIN agent_memory_blocks amb ON amb.block_id = b.id
             WHERE amb.agent_id = ?1 AND b.label = ?2",
            params![agent_id, label],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();

    let Some(block_id) = block_id else {
        return;
    };

    // Delete old chunks.
    let _ = conn.execute(
        "DELETE FROM memory_chunks WHERE block_id = ?1",
        params![block_id],
    );

    // Only chunk if above threshold.
    if value.chars().count() <= CHUNK_THRESHOLD {
        return;
    }

    let chunks = chunk_text(value);
    for chunk in &chunks {
        let id = uuid::Uuid::new_v4().to_string();
        let char_count = chunk.content.chars().count() as i64;

        // Compute embedding if available.
        let emb_blob: Option<Vec<u8>> = embedder.and_then(|e| {
            e.embed(&chunk.content).ok().and_then(|vec| {
                if vec.is_empty() {
                    None
                } else {
                    let mut bytes = Vec::with_capacity(vec.len() * 4);
                    for f in &vec {
                        bytes.extend_from_slice(&f.to_le_bytes());
                    }
                    Some(bytes)
                }
            })
        });

        let _ = conn.execute(
            "INSERT INTO memory_chunks (id, block_id, chunk_index, content, char_count, embedding)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, block_id, chunk.index as i64, chunk.content, char_count, emb_blob],
        );
    }
}

// ── A9: Proactive recall ──────────────────────────────────────────────────────

/// A recalled chunk with its parent block label and content.
#[derive(Debug, Clone)]
pub struct RecalledChunk {
    pub label: String,
    pub chunk_content: String,
    pub chunk_index: i64,
}

/// Search `memory_chunks` for the top-k chunks matching keywords extracted
/// from `query`.  Used by the proactive injection path (A9) to surface
/// relevant memory fragments before the LLM generates a response.
///
/// Returns at most `limit` chunks, ordered by the number of keyword hits
/// (most relevant first).  Chunks from the same block are deduplicated to
/// the highest-scoring one.
pub fn recall_chunks(
    db: &Db,
    agent_id: &str,
    query: &str,
    limit: usize,
) -> Vec<RecalledChunk> {
    // Extract keywords from the query (min 3 chars, skip stop words).
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect();

    if words.is_empty() {
        return Vec::new();
    }

    let conn = db.lock();

    // Build OR clauses for keyword matching.
    let conditions: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, _)| format!("LOWER(c.content) LIKE ?{}", i + 3))
        .collect();
    let where_clause = conditions.join(" OR ");

    let sql = format!(
        "SELECT b.label, c.content, c.chunk_index
         FROM memory_chunks c
         JOIN shared_memory_blocks b ON b.id = c.block_id
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.tier != 'long'
           AND ({where_clause})
         ORDER BY c.chunk_index ASC
         LIMIT ?2"
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    // Build params: agent_id, limit, then one pattern per word.
    let patterns: Vec<String> = words
        .iter()
        .map(|w| format!("%{w}%"))
        .collect();

    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(agent_id.to_string()));
    param_values.push(Box::new(limit as i64));
    for p in &patterns {
        param_values.push(Box::new(p.clone()));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();

    let results: Vec<RecalledChunk> = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(RecalledChunk {
                label: row.get(0)?,
                chunk_content: row.get(1)?,
                chunk_index: row.get(2)?,
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    // Deduplicate: keep only the first (best) chunk per label.
    let mut seen = std::collections::HashSet::new();
    results
        .into_iter()
        .filter(|r| seen.insert(r.label.clone()))
        .take(limit)
        .collect()
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

// ── P6: Auto-type memory blocks ──────────────────────────────────────────────

/// Confidence boost applied to auto-typed durable blocks (same as
/// `evidence.rs::TYPED_CONFIDENCE_BOOST`).
const AUTO_TYPE_CONFIDENCE_BOOST: f64 = 1.35;

/// Infer a memory_type from block content when none is explicitly set.
///
/// Runs inside `upsert_memory_block` on the already-held connection. Only
/// updates blocks whose `memory_type` is NULL (never overwrites an explicit
/// type set via `update_memory_typed`).
///
/// Pattern rules (checked in priority order):
///   1. Contains "decided"/"chosen"/"rejected"/"approved" → `decision`
///   2. Contains "must"/"always"/"never"/"rule"/"mandatory" → `constraint`
///   3. Contains "convention"/"pattern"/"style"/"naming" + file path → `convention`
///   4. Contains "user prefers"/"user wants"/"user likes" → `user_pref`
fn auto_type_block_if_untyped(
    conn: &parking_lot::MutexGuard<'_, rusqlite::Connection>,
    label: &str,
) {
    // Read current memory_type + value
    let row: Option<(Option<String>, String)> = conn
        .query_row(
            "SELECT memory_type, value FROM shared_memory_blocks WHERE label = ?1",
            params![label],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
        )
        .ok();

    let Some((existing_type, value)) = row else {
        return;
    };

    // Skip if already typed
    if existing_type.is_some() {
        return;
    }

    let lower = value.to_lowercase();
    let inferred = if contains_any(&lower, &["decided", "chosen", "rejected", "approved", "decision"]) {
        "decision"
    } else if contains_any(&lower, &["must ", "always ", "never ", " rule", "mandatory", "forbidden"]) {
        "constraint"
    } else if contains_any(&lower, &["convention", "pattern", "naming", "style guide"])
        && (lower.contains('/') || lower.contains('.'))
    {
        "convention"
    } else if contains_any(&lower, &["user prefers", "user wants", "user likes", "user asked"]) {
        "user_pref"
    } else {
        return; // No match — leave untyped
    };

    let _ = conn.execute(
        "UPDATE shared_memory_blocks SET memory_type = ?1, confidence = MAX(confidence, ?2) WHERE label = ?3",
        params![inferred, AUTO_TYPE_CONFIDENCE_BOOST, label],
    );
}

/// Check if `haystack` contains any of the `needles`.
fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
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

/// F7: bump `access_count` and stamp `last_access_turn` for the named blocks.
///
/// Call this whenever the agent intentionally reads memory (e.g.
/// `search_memory` returning a hit).  Bumping these counters extends the
/// retention window in `promote_stale_blocks` so frequently-consulted
/// blocks survive the 80-turn idle cliff.
///
/// The update is bounded to the agent's own blocks via `agent_memory_blocks`
/// so two agents reading the same shared block don't accumulate ghost
/// access counts for each other.
///
/// Errors are logged at trace level and swallowed: a missed access bump
/// is never worth failing a search call over.
pub fn bump_block_access(db: &Db, agent_id: &str, labels: &[&str]) {
    if labels.is_empty() {
        return;
    }
    let current_turn = get_turn_counter(db, agent_id).unwrap_or(0);
    let conn = db.lock();
    for label in labels {
        let res = conn.execute(
            "UPDATE shared_memory_blocks
             SET access_count     = access_count + 1,
                 last_access_turn = ?1
             WHERE label = ?2
               AND id IN (SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?3)",
            params![current_turn, label, agent_id],
        );
        if let Err(e) = res {
            tracing::trace!("bump_block_access [{agent_id}/{label}] skipped: {e}");
        }
    }
}

/// Promote 'short' blocks idle for >= threshold turns to 'long'.
/// 'pinned' blocks are never promoted. Returns number of blocks promoted.
///
/// F7 (activity-weighted aging): the staleness clock is the **maximum** of
/// `last_turn` (last write) and `last_access_turn` (last read via
/// `search_memory`). Additionally, the effective threshold is multiplied by
/// an access-frequency boost: each intentional read up to a cap of 10 adds
/// 20% to the retention window, so a block read 5 times survives 2× longer
/// than an unread block, and one read 10+ times survives 3× longer. This
/// prevents the 80-turn cliff from prematurely archiving blocks the agent
/// has been actively consulting.
pub fn promote_stale_blocks(
    db: &Db,
    agent_id: &str,
    current_turn: i64,
    threshold: i64,
) -> Result<u64> {
    let conn = db
        .lock();
    // Per-row threshold = base × (1 + min(access_count, 10) × 0.20)
    // Implemented in SQL as base * (5 + MIN(access_count, 10)) / 5 so we stay
    // in integer arithmetic and avoid round-off surprises across SQLite versions.
    let n = conn.execute(
        "UPDATE shared_memory_blocks SET tier = 'long'
         WHERE tier = 'short'
           AND (?1 - MAX(last_turn, last_access_turn)) >=
               (?2 * (5 + MIN(access_count, 10)) / 5)
           AND confidence < ?3
           AND id IN (
               SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?4
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

/// Fetch long-term blocks with rich excerpts (A3).
/// Returns LongTermExcerpt with 250-char excerpts, keywords, char counts, and idle turn count.
pub fn get_long_term_excerpts(
    db: &Db,
    agent_id: &str,
    current_turn: i64,
) -> Result<Vec<LongTermExcerpt>> {
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
        
        // A3: Take first 250 chars as excerpt (increased from 80)
        let char_count = value.chars().count();
        let excerpt: String = value.chars().take(250).collect();
        let excerpt = if char_count > 250 {
            format!("{excerpt}…")
        } else {
            excerpt
        };
        
        // A3: Extract top 5 distinctive keywords
        let keywords = extract_keywords(&value, 5);
        
        Ok(LongTermExcerpt {
            label,
            excerpt,
            keywords,
            char_count,
            turns_idle: current_turn - last_turn,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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

// ── A15: Subagent write-back ──────────────────────────────────────────────────

/// Labels that are ephemeral to the subagent session and should NOT be
/// written back to the parent agent (they either already exist or are
/// parent-seeded copies).
const WRITEBACK_EXCLUDE: &[&str] = &[
    "persona",
    "human",
    "project",
    "active_goal",
    "recent_edits",
    "session_summary",
    "session_index",
    "skills",
];

/// A fact extracted from the subagent's memory for write-back.
#[derive(Debug, Clone)]
pub struct WritebackFact {
    pub label: String,
    pub value: String,
    pub description: String,
}

/// Extract typed facts from a subagent's memory and write them to the
/// parent agent's memory, prefixed with `subagent:` so the parent can
/// distinguish inherited knowledge.
///
/// Called just before the ephemeral subagent DB row is deleted.
///
/// Returns the number of facts written back.
pub fn write_back_subagent_memory(
    db: &Db,
    subagent_id: &str,
    parent_agent_id: &str,
) -> usize {
    let blocks = get_memory_blocks(db, subagent_id).unwrap_or_default();

    let facts: Vec<WritebackFact> = blocks
        .into_iter()
        .filter(|(label, value, _)| {
            // Skip excluded labels.
            if WRITEBACK_EXCLUDE.contains(&label.as_str()) {
                return false;
            }
            // Skip skill blocks (parent already has them).
            if label.starts_with("skill:") {
                return false;
            }
            // REC-3: Skip blocks already prefixed with `subagent:` to
            // prevent cascading labels (subagent:subagent:subagent:…)
            // when subagents inherit write-back results from prior runs.
            if label.starts_with("subagent:") {
                return false;
            }
            // Skip empty values.
            if value.trim().is_empty() {
                return false;
            }
            true
        })
        .map(|(label, value, desc)| WritebackFact {
            label,
            value,
            description: desc,
        })
        .collect();

    let mut written = 0;
    for fact in &facts {
        // Prefix with `subagent:` to namespace it under the parent.
        let parent_label = format!("subagent:{}", fact.label);
        let desc = if fact.description.is_empty() {
            Some(format!("Written back from subagent {subagent_id}"))
        } else {
            Some(format!("{} (from subagent {subagent_id})", fact.description))
        };

        if upsert_memory_block(
            db,
            parent_agent_id,
            &parent_label,
            &fact.value,
            desc.as_deref(),
            None,
        )
        .is_ok()
        {
            written += 1;
        }
    }

    if written > 0 {
        tracing::debug!(
            "A15 write-back: {written}/{} facts from {subagent_id} → {parent_agent_id}",
            facts.len()
        );
    }

    written
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
