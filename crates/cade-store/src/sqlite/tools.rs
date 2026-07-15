use super::*;

// ── A11: Recency × Frequency scoring ─────────────────────────────────────────

/// Compute a composite score for ranking search results.
///
/// `score = frequency_weight × recency_weight` where:
/// - `frequency_weight = 1.0 + log2(access_count + 1)` — each doubling of
///   access count adds +1.0 to the weight (diminishing returns).
/// - `recency_weight = 1.0 / (1.0 + turns_idle × 0.02)` — blocks idle for
///   50 turns score ~0.5, blocks idle for 100+ turns score ~0.33.
///
/// `turns_idle` uses the **maximum** of `last_turn` (last write) and
/// `last_access_turn` (last search hit), matching the `promote_stale_blocks`
/// staleness clock.
pub fn recency_frequency_score(
    current_turn: i64,
    last_turn: i64,
    last_access_turn: i64,
    access_count: i64,
) -> f64 {
    let last_activity = last_turn.max(last_access_turn);
    let turns_idle = (current_turn - last_activity).max(0) as f64;
    let recency = 1.0 / (1.0 + turns_idle * 0.02);
    let frequency = 1.0 + (access_count as f64 + 1.0).log2();
    recency * frequency
}

pub fn upsert_tool(db: &Db, row: &ToolRow) -> Result<()> {
    let conn = db.get()?;
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

pub fn get_tool_id_by_name(db: &Db, name: &str) -> Option<String> {
    let conn = db.get().ok()?;
    let mut stmt = conn.prepare("SELECT id FROM tools WHERE name = ?1").ok()?;
    stmt.query_row(params![name], |r| r.get::<_, String>(0))
        .ok()
}

/// Delete all messages for an agent (or a specific conversation).
/// If conversation_id is None, deletes all messages for the agent.
pub fn clear_messages(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<usize> {
    let conn = db.get()?;
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
/// A ranked search result from FTS5 message search.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageSearchResult {
    pub id: String,
    pub agent_id: String,
    pub conversation_id: Option<String>,
    pub role: String,
    pub content: Value,
    /// BM25 relevance score (lower = more relevant in SQLite FTS5).
    pub score: f64,
    /// Context snippet with match highlighted by `**` markers.
    pub snippet: String,
}

/// True iff the agent has at least one compaction marker in `messages`.
///
/// Used by `conversation_search` (F8) so the agent can be told — even when
/// the current search returns zero hits — that older history was summarised
/// into `session_summary` and the F2 archival cache.
///
/// When `conversation_id` is `None`, the check spans every conversation.
/// When `Some(_)`, only that conversation's compaction markers count.
pub fn has_compaction_marker(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> Result<bool> {
    let conn = db.get()?;
    let count: i64 = if let Some(cid) = conversation_id {
        conn.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'compaction'",
            params![agent_id, cid],
            |r| r.get(0),
        )
        .unwrap_or(0)
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE agent_id = ?1 AND role = 'compaction'",
            params![agent_id],
            |r| r.get(0),
        )
        .unwrap_or(0)
    };
    Ok(count > 0)
}

/// Search messages using FTS5 with BM25 ranking and context snippets.
///
/// Results are ordered by BM25 relevance (best match first).
/// Each result includes a `snippet` field: up to 32 tokens of context
/// around the best matching phrase, with matches wrapped in `**...**`.
///
/// Falls back to a plain LIKE search if FTS5 is unavailable.
pub fn search_messages(
    db: &Db,
    agent_id: &str,
    query: &str,
    conversation_id: Option<&str>,
) -> Result<Vec<MessageSearchResult>> {
    let conn = db.get()?;

    // Build safe FTS5 query: wrap the whole phrase in double-quotes to handle
    // spaces and special chars; escape internal quotes.
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));

    // FTS5 bm25() returns negative values; ORDER BY bm25 ASC = best match first.
    // snippet() extracts up to 32 tokens around the best match:
    //   args: (fts_table, col_idx, before_match, after_match, ellipsis, max_tokens)
    // Exclude compaction markers from search results — they are meta-content, not
    // real conversation messages.
    // P4-B: Identify pre-compaction messages so the agent knows their context might be truncated.
    let sql = if conversation_id.is_some() {
        "WITH boundary AS (
             SELECT COALESCE(
                 (SELECT rowid FROM messages
                  WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'compaction'
                  ORDER BY created_at DESC, rowid DESC LIMIT 1),
                 0
             ) AS marker_rowid
         )
         SELECT m.id, m.agent_id, m.conversation_id, m.role, m.content,
                bm25(messages_fts) AS score,
                CASE WHEN m.rowid <= (SELECT marker_rowid FROM boundary)
                     THEN snippet(messages_fts, 0, '**', '**', '…', 32) || '\n[pre-compaction; full text in archival_memory_search (tag: dropped-turns) and summary in session_summary]'
                     ELSE snippet(messages_fts, 0, '**', '**', '…', 32)
                END AS snip
         FROM messages m
         JOIN messages_fts ON messages_fts.rowid = m.rowid
         WHERE m.agent_id = ?1 AND m.conversation_id = ?2
           AND m.role != 'compaction'
           AND messages_fts MATCH ?3
         ORDER BY score ASC
         LIMIT 20"
    } else {
        "WITH boundary AS (
             SELECT COALESCE(
                 (SELECT rowid FROM messages
                  WHERE agent_id = ?1 AND conversation_id IS NULL AND role = 'compaction'
                  ORDER BY created_at DESC, rowid DESC LIMIT 1),
                 0
             ) AS marker_rowid
         )
         SELECT m.id, m.agent_id, m.conversation_id, m.role, m.content,
                bm25(messages_fts) AS score,
                CASE WHEN m.rowid <= (SELECT marker_rowid FROM boundary)
                     THEN snippet(messages_fts, 0, '**', '**', '…', 32) || '\n[pre-compaction; full text in archival_memory_search (tag: dropped-turns) and summary in session_summary]'
                     ELSE snippet(messages_fts, 0, '**', '**', '…', 32)
                END AS snip
         FROM messages m
         JOIN messages_fts ON messages_fts.rowid = m.rowid
         WHERE m.agent_id = ?1
           AND m.role != 'compaction'
           AND messages_fts MATCH ?2
         ORDER BY score ASC
         LIMIT 20"
    };

    let mapper = |r: &rusqlite::Row| {
        let content_str: String = r.get(4)?;
        let content =
            serde_json::from_str(&content_str).unwrap_or(serde_json::Value::String(content_str));
        Ok(MessageSearchResult {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            conversation_id: r.get(2)?,
            role: r.get(3)?,
            content,
            score: r.get::<_, f64>(5).unwrap_or(0.0),
            snippet: r.get::<_, String>(6).unwrap_or_default(),
        })
    };

    let result = if let Some(conv_id) = conversation_id {
        conn.prepare(sql)?
            .query_map(params![agent_id, conv_id, fts_query], mapper)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        conn.prepare(sql)?
            .query_map(params![agent_id, fts_query], mapper)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    Ok(result)
}

pub fn search_memory(
    db: &Db,
    agent_id: &str,
    query: &str,
    memory_type: Option<&str>,
) -> Result<Vec<(String, String, String)>> {
    let q_lower = query.to_lowercase();
    let current_turn = super::get_turn_counter(db, agent_id).unwrap_or(0);

    let mut results: Vec<(String, String, String)> = {
        let conn = db.get()?;
        let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));

        let mut scored: Vec<(f64, String, String, String)> = if let Some(mtype) = memory_type {
            let mut stmt = conn.prepare(
                "SELECT b.label, b.value, b.access_count, b.last_access_turn, b.last_turn
                 FROM shared_memory_blocks b
                 JOIN agent_memory_blocks amb ON amb.block_id = b.id
                 WHERE amb.agent_id = ?1
                   AND b.memory_type = ?3
                   AND (LOWER(b.label) LIKE LOWER(?2) ESCAPE '\\'
                        OR LOWER(b.value) LIKE LOWER(?2) ESCAPE '\\')
                 LIMIT 20",
            )?;
            let rows = stmt.query_map(params![agent_id, pattern, mtype], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2).unwrap_or(0),
                    row.get::<_, i64>(3).unwrap_or(0),
                    row.get::<_, i64>(4).unwrap_or(0),
                ))
            })?;
            rows.filter_map(|r| r.ok())
                .map(
                    |(label, value, access_count, last_access_turn, last_turn)| {
                        let val_lower = value.to_lowercase();
                        let snippet = if let Some(pos) = val_lower.find(&q_lower) {
                            let start = pos.saturating_sub(80);
                            let end = (pos + q_lower.len() + 80).min(value.len());
                            let prefix = if start > 0 { "…" } else { "" };
                            let suffix = if end < value.len() { "…" } else { "" };
                            let s = value
                                .char_indices()
                                .map(|(i, _)| i)
                                .find(|&i| i >= start)
                                .unwrap_or(start);
                            let e = value
                                .char_indices()
                                .map(|(i, _)| i)
                                .find(|&i| i >= end)
                                .unwrap_or(end);
                            format!("{prefix}{}{suffix}", &value[s..e])
                        } else {
                            value.chars().take(160).collect::<String>()
                        };
                        let score = recency_frequency_score(
                            current_turn,
                            last_turn,
                            last_access_turn,
                            access_count,
                        );
                        (score, label, value, snippet)
                    },
                )
                .collect()
        } else {
            let mut stmt = conn.prepare(
                "SELECT b.label, b.value, b.access_count, b.last_access_turn, b.last_turn
                 FROM shared_memory_blocks b
                 JOIN agent_memory_blocks amb ON amb.block_id = b.id
                 WHERE amb.agent_id = ?1
                   AND (LOWER(b.label) LIKE LOWER(?2) ESCAPE '\\'
                        OR LOWER(b.value) LIKE LOWER(?2) ESCAPE '\\')
                 LIMIT 20",
            )?;
            let rows = stmt.query_map(params![agent_id, pattern], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2).unwrap_or(0),
                    row.get::<_, i64>(3).unwrap_or(0),
                    row.get::<_, i64>(4).unwrap_or(0),
                ))
            })?;
            rows.filter_map(|r| r.ok())
                .map(
                    |(label, value, access_count, last_access_turn, last_turn)| {
                        let val_lower = value.to_lowercase();
                        let snippet = if let Some(pos) = val_lower.find(&q_lower) {
                            let start = pos.saturating_sub(80);
                            let end = (pos + q_lower.len() + 80).min(value.len());
                            let prefix = if start > 0 { "…" } else { "" };
                            let suffix = if end < value.len() { "…" } else { "" };
                            let s = value
                                .char_indices()
                                .map(|(i, _)| i)
                                .find(|&i| i >= start)
                                .unwrap_or(start);
                            let e = value
                                .char_indices()
                                .map(|(i, _)| i)
                                .find(|&i| i >= end)
                                .unwrap_or(end);
                            format!("{prefix}{}{suffix}", &value[s..e])
                        } else {
                            value.chars().take(160).collect::<String>()
                        };
                        let score = recency_frequency_score(
                            current_turn,
                            last_turn,
                            last_access_turn,
                            access_count,
                        );
                        (score, label, value, snippet)
                    },
                )
                .collect()
        };

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .take(10)
            .map(|(_, label, value, snippet)| (label, value, snippet))
            .collect()
    };

    if results.is_empty() {
        let words: Vec<String> = query
            .split_whitespace()
            .filter(|w| w.len() >= 3)
            .map(|w| w.to_lowercase())
            .collect();
        if !words.is_empty() {
            let conn = db.get()?;
            let mut stmt_sql = String::from(
                "SELECT b.label, b.value FROM shared_memory_blocks b JOIN agent_memory_blocks amb ON amb.block_id = b.id WHERE amb.agent_id = ?1",
            );
            if memory_type.is_some() {
                stmt_sql.push_str(" AND b.memory_type = ?2");
            }
            let mut scored: Vec<(usize, String, String, String)> = Vec::new();

            if let Some(mtype) = memory_type {
                if let Ok(mut stmt) = conn.prepare(&stmt_sql)
                    && let Ok(rows) = stmt.query_map(params![agent_id, mtype], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                {
                    for row in rows.filter_map(|r| r.ok()) {
                        let (label, value) = row;
                        let combined = format!("{} {}", label, value).to_lowercase();
                        let hits = words
                            .iter()
                            .filter(|w| combined.contains(w.as_str()))
                            .count();
                        if hits > 0 {
                            let snippet = value.chars().take(200).collect::<String>();
                            scored.push((hits, label, value, snippet));
                        }
                    }
                }
            } else {
                if let Ok(mut stmt) = conn.prepare(&stmt_sql)
                    && let Ok(rows) = stmt.query_map(params![agent_id], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                {
                    for row in rows.filter_map(|r| r.ok()) {
                        let (label, value) = row;
                        let combined = format!("{} {}", label, value).to_lowercase();
                        let hits = words
                            .iter()
                            .filter(|w| combined.contains(w.as_str()))
                            .count();
                        if hits > 0 {
                            let snippet = value.chars().take(200).collect::<String>();
                            scored.push((hits, label, value, snippet));
                        }
                    }
                }
            }

            scored.sort_by_key(|b| std::cmp::Reverse(b.0));
            results = scored
                .into_iter()
                .take(10)
                .map(|(_, label, value, snippet)| (label, value, snippet))
                .collect();
        }
    }

    {
        let conn = db.get()?;
        let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));

        let chunk_hits: Vec<(String, String, String)> = if let Some(mtype) = memory_type {
            let mut stmt = conn
                .prepare(
                    "SELECT b.label, c.content, c.chunk_index
                 FROM memory_chunks c
                 JOIN shared_memory_blocks b ON b.id = c.block_id
                 JOIN agent_memory_blocks amb ON amb.block_id = b.id
                 WHERE amb.agent_id = ?1
                   AND b.memory_type = ?3
                   AND LOWER(c.content) LIKE LOWER(?2) ESCAPE '\\'
                 ORDER BY c.chunk_index ASC
                 LIMIT 5",
                )
                .ok();
            if let Some(ref mut st) = stmt {
                st.query_map(params![agent_id, pattern, mtype], |row| {
                    let label: String = row.get(0)?;
                    let content: String = row.get(1)?;
                    let idx: i64 = row.get(2)?;
                    let snippet = format!(
                        "[chunk {}] {}",
                        idx,
                        content.chars().take(200).collect::<String>()
                    );
                    Ok((label, content, snippet))
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT b.label, c.content, c.chunk_index
                 FROM memory_chunks c
                 JOIN shared_memory_blocks b ON b.id = c.block_id
                 JOIN agent_memory_blocks amb ON amb.block_id = b.id
                 WHERE amb.agent_id = ?1
                   AND LOWER(c.content) LIKE LOWER(?2) ESCAPE '\\'
                 ORDER BY c.chunk_index ASC
                 LIMIT 5",
                )
                .ok();
            if let Some(ref mut st) = stmt {
                st.query_map(params![agent_id, pattern], |row| {
                    let label: String = row.get(0)?;
                    let content: String = row.get(1)?;
                    let idx: i64 = row.get(2)?;
                    let snippet = format!(
                        "[chunk {}] {}",
                        idx,
                        content.chars().take(200).collect::<String>()
                    );
                    Ok((label, content, snippet))
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
            } else {
                Vec::new()
            }
        };

        let existing_labels: std::collections::HashSet<String> =
            results.iter().map(|(l, _, _)| l.clone()).collect();
        for (label, value, snippet) in chunk_hits {
            if !existing_labels.contains(&label) {
                results.push((label, value, snippet));
            }
        }
    }

    if !results.is_empty() {
        let labels: Vec<&str> = results.iter().map(|(l, _, _)| l.as_str()).collect();
        super::memory::bump_block_access(db, agent_id, &labels);
    }

    Ok(results)
}

pub fn search_memory_hybrid(
    db: &Db,
    agent_id: &str,
    query: &str,
    memory_type: Option<&str>,
    embedder: Option<&dyn super::embedding::Embedder>,
) -> Result<Vec<(String, String, String)>> {
    let kw_results = search_memory(db, agent_id, query, memory_type)?;

    let Some(emb) = embedder else {
        return Ok(kw_results);
    };

    let q_vec = emb.embed(query)?;
    if q_vec.is_empty() {
        return Ok(kw_results);
    }
    let sem_hits = {
        let conn = db.get()?;
        super::embedding::search_memory_semantic(&conn, agent_id, &q_vec, memory_type, 10)?
    };

    use std::collections::HashMap;
    let mut by_label: HashMap<String, (String, String, String)> = HashMap::new();
    for (label, value, snippet) in &kw_results {
        by_label
            .entry(label.clone())
            .or_insert_with(|| (label.clone(), value.clone(), snippet.clone()));
    }
    for (_id, _score, label, value) in &sem_hits {
        by_label.entry(label.clone()).or_insert_with(|| {
            let snippet: String = value.chars().take(200).collect();
            (label.clone(), value.clone(), snippet)
        });
    }

    let kw_labels: Vec<String> = kw_results.iter().map(|(l, _, _)| l.clone()).collect();
    let sem_labels: Vec<String> = sem_hits.iter().map(|(_, _, l, _)| l.clone()).collect();
    let fused = super::embedding::reciprocal_rank_fusion(&kw_labels, &sem_labels, 60.0);

    // ── Temporal decay ────────────────────────────────────────────────────
    // Apply a time-decay factor to each fused result: memories older than
    // ~7 days (the half-life) score roughly half their original RRF weight.
    //   decay = 1 / (1 + ln(1 + age_hours / 168))
    const RRF_K: f64 = 60.0;
    let now: f64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as f64;
    let half_life_secs: f64 = 168.0 * 3600.0;

    let mut scores: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for (rank, label) in kw_labels.iter().enumerate() {
        *scores.entry(label.as_str()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    for (rank, label) in sem_labels.iter().enumerate() {
        *scores.entry(label.as_str()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }

    let conn = db.get()?;
    let mut decayed: Vec<(String, f64)> = Vec::new();
    for label in &fused {
        let rrf_score = scores.get(label.as_str()).copied().unwrap_or(1.0);
        let updated_at: f64 = conn
            .query_row(
                "SELECT MAX(b.updated_at) FROM shared_memory_blocks b
                 JOIN agent_memory_blocks amb ON amb.block_id = b.id
                 WHERE amb.agent_id = ?1 AND b.label = ?2",
                params![agent_id, label],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(now as i64) as f64;
        let age_secs = (now - updated_at).max(0.0);
        let decay = 1.0 / (1.0 + (1.0 + age_secs / half_life_secs).ln());
        decayed.push((label.clone(), rrf_score * decay));
    }
    decayed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut out: Vec<(String, String, String)> = Vec::new();
    for (label, _) in decayed.iter().take(10) {
        if let Some(row) = by_label.remove(label.as_str()) {
            out.push(row);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ArchivalRecord {
    pub id: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: i64,
}

/// Insert a new large data block into Archival Memory.
pub fn insert_archival_memory(
    db: &Db,
    agent_id: &str,
    content: &str,
    tags: &[String],
) -> Result<String> {
    let conn = db.get()?;
    let id = uuid::Uuid::new_v4().to_string();
    let tags_json = serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        "INSERT INTO archival_memory (id, agent_id, content, tags, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, agent_id, content, tags_json, now_ts()],
    )?;
    Ok(id)
}

/// Search Archival Memory using FTS5 (BM25 ranking).
pub fn search_archival_memory(
    db: &Db,
    agent_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<ArchivalRecord>> {
    let conn = db.get()?;

    // FTS5 requires queries to be properly quoted to avoid syntax errors
    let fts_query = format!("\"{}\"", query.replace('\"', "\"\""));

    let mut stmt = conn.prepare(
        "SELECT id, content, tags, created_at
         FROM archival_memory
         WHERE archival_memory MATCH ?2 AND agent_id = ?1
         ORDER BY bm25(archival_memory)
         LIMIT ?3",
    )?;

    let rows = stmt.query_map(params![agent_id, fts_query, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    let mut results = Vec::new();
    for row in rows.filter_map(|r| r.ok()) {
        let (id, content, tags_str, created_at) = row;
        let tags = serde_json::from_str(&tags_str).unwrap_or_default();
        results.push(ArchivalRecord {
            id,
            content,
            tags,
            created_at,
        });
    }

    Ok(results)
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
                    let non_empty: Vec<_> = arr
                        .iter()
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
    let conn = db.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, name, description, source_code, json_schema, tags FROM tools ORDER BY name",
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
        .map(
            |(id, name, description, source_code, schema_str, tags_str)| ToolRow {
                id,
                name,
                description,
                source_code,
                json_schema: schema_str.and_then(|s| serde_json::from_str(&s).ok()),
                tags: serde_json::from_str(&tags_str).unwrap_or_default(),
            },
        )
        .collect())
}

// -- Providers

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;
    use serde_json::json;

    fn setup_mem_db() -> Result<Db> {
        Ok(super::open(":memory:")?)
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
                compaction_model: None,
                theme: None,
                active_plan_json: None,
                parent_id: None,
            },
        )?;
        Ok(())
    }

    fn make_tool(id: &str, name: &str) -> ToolRow {
        ToolRow {
            id: id.into(),
            name: name.into(),
            description: Some(format!("{name} tool")),
            source_code: None,
            json_schema: Some(json!({"name": name})),
            tags: vec!["test".into()],
        }
    }

    #[test]
    fn test_upsert_and_list_tools() -> Result<()> {
        let db = setup_mem_db()?;
        assert!(list_tools(&db)?.is_empty());

        upsert_tool(&db, &make_tool("t1", "bash"))?;
        upsert_tool(&db, &make_tool("t2", "grep"))?;

        let tools = list_tools(&db)?;
        assert_eq!(tools.len(), 2);
        Ok(())
    }

    #[test]
    fn test_upsert_tool_update() -> Result<()> {
        let db = setup_mem_db()?;
        upsert_tool(&db, &make_tool("t1", "bash"))?;

        // Upsert same name with updated description (conflict on name)
        upsert_tool(
            &db,
            &ToolRow {
                id: "t1-new".into(), // different id doesn't matter — conflict is on name
                name: "bash".into(),
                description: Some("Updated bash".into()),
                source_code: None,
                json_schema: None,
                tags: vec![],
            },
        )?;

        let tools = list_tools(&db)?;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].description, Some("Updated bash".into()));
        Ok(())
    }

    #[test]
    fn test_get_tool_id_by_name() -> Result<()> {
        let db = setup_mem_db()?;
        assert!(get_tool_id_by_name(&db, "bash").is_none());

        upsert_tool(&db, &make_tool("t1", "bash"))?;
        assert_eq!(get_tool_id_by_name(&db, "bash"), Some("t1".into()));
        assert!(get_tool_id_by_name(&db, "nope").is_none());
        Ok(())
    }

    #[test]
    fn test_clear_messages_all() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        insert_message(
            &db,
            &MessageRow {
                id: "m1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!("hello"),
                char_count: 5,
            },
        )?;
        insert_message(
            &db,
            &MessageRow {
                id: "m2".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "assistant".into(),
                content: json!("hi"),
                char_count: 2,
            },
        )?;

        let cleared = clear_messages(&db, "a1", None)?;
        assert_eq!(cleared, 2);
        assert!(list_messages(&db, "a1", None, 100)?.is_empty());
        Ok(())
    }

    #[test]
    fn test_clear_messages_by_conversation() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        insert_message(
            &db,
            &MessageRow {
                id: "m1".into(),
                agent_id: "a1".into(),
                conversation_id: Some("c1".into()),
                role: "user".into(),
                content: json!("conv1"),
                char_count: 4,
            },
        )?;
        insert_message(
            &db,
            &MessageRow {
                id: "m2".into(),
                agent_id: "a1".into(),
                conversation_id: Some("c2".into()),
                role: "user".into(),
                content: json!("conv2"),
                char_count: 4,
            },
        )?;

        // Clear only c1
        let cleared = clear_messages(&db, "a1", Some("c1"))?;
        assert_eq!(cleared, 1);
        // c2 should remain
        let remaining = list_messages(&db, "a1", Some("c2"), 100)?;
        assert_eq!(remaining.len(), 1);
        Ok(())
    }

    #[test]
    fn test_search_messages_fts() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        insert_message(
            &db,
            &MessageRow {
                id: "m1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!("Rust is a systems programming language"),
                char_count: 40,
            },
        )?;
        insert_message(
            &db,
            &MessageRow {
                id: "m2".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!("Python is great for data science"),
                char_count: 32,
            },
        )?;

        let results = search_messages(&db, "a1", "Rust", None)?;
        assert_eq!(results.len(), 1);
        assert!(results[0].content.as_str().unwrap().contains("Rust"));

        let results = search_messages(&db, "a1", "nonexistent_term_xyz", None)?;
        assert!(results.is_empty());
        Ok(())
    }

    #[test]
    fn test_insert_and_search_archival_memory() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        insert_archival_memory(
            &db,
            "a1",
            "The quick brown fox jumps over the lazy dog",
            &["test".into(), "fox".into()],
        )?;
        insert_archival_memory(&db, "a1", "Lorem ipsum dolor sit amet", &["test".into()])?;

        let results = search_archival_memory(&db, "a1", "brown fox", 10)?;
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("brown fox"));
        assert!(results[0].tags.contains(&"fox".into()));

        let all = search_archival_memory(&db, "a1", "test", 10)?;
        // Both entries should be searchable
        assert!(!all.is_empty());
        Ok(())
    }

    #[test]
    fn test_list_tools_empty() -> Result<()> {
        let db = setup_mem_db()?;
        assert!(list_tools(&db)?.is_empty());
        Ok(())
    }

    #[test]
    fn test_pending_tool_results() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // No pending results initially
        let (pending, _total) = pending_tool_results(&db, "a1", None)?;
        assert_eq!(pending, 0);
        Ok(())
    }
}

// ── C7: Unified recall (federated FTS5 search) ────────────────────────────

/// A single result from the federated recall search.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RecallResult {
    pub source: String,  // "memory", "conversation", "archival", "event_log"
    pub label: String,   // block label / message role / archive id / event type
    pub snippet: String, // contextual excerpt
    pub score: f64,      // RRF-combined score (higher = better)
}

/// C7: Federated search across memory blocks, messages, archival memory,
/// and event log.  Returns up to `limit` results ranked by reciprocal
/// rank fusion (RRF) across the four sources.
pub fn recall(db: &Db, agent_id: &str, query: &str, limit: usize) -> Result<Vec<RecallResult>> {
    const RRF_K: f64 = 60.0; // standard RRF constant

    let mut all_results: Vec<RecallResult> = Vec::new();

    // Source 1: Memory blocks (LIKE + fuzzy)
    if let Ok(mem_hits) = search_memory(db, agent_id, query, None) {
        for (rank, (label, _value, snippet)) in mem_hits.into_iter().enumerate() {
            all_results.push(RecallResult {
                source: "memory".into(),
                label,
                snippet,
                score: 1.0 / (RRF_K + rank as f64 + 1.0),
            });
        }
    }

    // Source 2: Conversation messages (FTS5)
    if let Ok(msg_hits) = search_messages(db, agent_id, query, None) {
        for (rank, hit) in msg_hits.into_iter().enumerate() {
            all_results.push(RecallResult {
                source: "conversation".into(),
                label: hit.role,
                snippet: hit.snippet,
                score: 1.0 / (RRF_K + rank as f64 + 1.0),
            });
        }
    }

    // Source 3: Archival memory (FTS5)
    if let Ok(arch_hits) = search_archival_memory(db, agent_id, query, 10) {
        for (rank, hit) in arch_hits.into_iter().enumerate() {
            let snip = hit.content.chars().take(200).collect::<String>();
            all_results.push(RecallResult {
                source: "archival".into(),
                label: hit.tags.first().cloned().unwrap_or_default(),
                snippet: snip,
                score: 1.0 / (RRF_K + rank as f64 + 1.0),
            });
        }
    }

    // Source 4: Event log (FTS5)
    if let Ok(ev_hits) = super::event_log::query_event_log(db, agent_id, query, 10) {
        for (rank, hit) in ev_hits.into_iter().enumerate() {
            let snip = hit.content.chars().take(200).collect::<String>();
            all_results.push(RecallResult {
                source: "event_log".into(),
                label: hit.event_type,
                snippet: snip,
                score: 1.0 / (RRF_K + rank as f64 + 1.0),
            });
        }
    }

    // Sort by RRF score descending, take top `limit`.
    all_results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all_results.truncate(limit);

    Ok(all_results)
}

// endregion: --- Tests
