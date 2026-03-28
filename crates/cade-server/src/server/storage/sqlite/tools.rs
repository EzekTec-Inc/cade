use super::*;

pub fn upsert_tool(db: &Db, row: &ToolRow) -> Result<()> {
    let conn = db.lock().expect("db lock poisoned");
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
    let conn = db.lock().expect("db lock poisoned");
    let mut stmt = conn.prepare("SELECT id FROM tools WHERE name = ?1").ok()?;
    stmt.query_row(params![name], |r| r.get::<_, String>(0))
        .ok()
}

/// Delete all messages for an agent (or a specific conversation).
/// If conversation_id is None, deletes all messages for the agent.
pub fn clear_messages(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<usize> {
    let conn = db.lock().expect("db lock poisoned");
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
    let conn = db.lock().expect("db lock poisoned");

    // Build safe FTS5 query: wrap the whole phrase in double-quotes to handle
    // spaces and special chars; escape internal quotes.
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));

    // FTS5 bm25() returns negative values; ORDER BY bm25 ASC = best match first.
    // snippet() extracts up to 32 tokens around the best match:
    //   args: (fts_table, col_idx, before_match, after_match, ellipsis, max_tokens)
    let sql = if conversation_id.is_some() {
        "SELECT m.id, m.agent_id, m.conversation_id, m.role, m.content,
                bm25(messages_fts) AS score,
                snippet(messages_fts, 0, '**', '**', '…', 32) AS snip
         FROM messages m
         JOIN messages_fts ON messages_fts.rowid = m.rowid
         WHERE m.agent_id = ?1 AND m.conversation_id = ?2
           AND messages_fts MATCH ?3
         ORDER BY score ASC
         LIMIT 20"
    } else {
        "SELECT m.id, m.agent_id, m.conversation_id, m.role, m.content,
                bm25(messages_fts) AS score,
                snippet(messages_fts, 0, '**', '**', '…', 32) AS snip
         FROM messages m
         JOIN messages_fts ON messages_fts.rowid = m.rowid
         WHERE m.agent_id = ?1
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

/// Search memory blocks for an agent using case-insensitive LIKE.
/// Returns (label, value, snippet) where snippet is up to 200 chars around
/// the first match in the value.
///
/// Memory blocks are small (< 8 KB each) so LIKE is fast enough without FTS5.
pub fn search_memory(
    db: &Db,
    agent_id: &str,
    query: &str,
) -> Result<Vec<(String, String, String)>> {
    let conn = db.lock().expect("db lock poisoned");
    let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
    let mut stmt = conn.prepare(
        "SELECT b.label, b.value FROM shared_memory_blocks b
         JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1
           AND (LOWER(b.label) LIKE LOWER(?2) ESCAPE '\\'
                OR LOWER(b.value) LIKE LOWER(?2) ESCAPE '\\')
         ORDER BY b.updated_at DESC
         LIMIT 10",
    )?;
    let rows = stmt.query_map(params![agent_id, pattern], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let q_lower = query.to_lowercase();
    let results = rows
        .filter_map(|r| r.ok())
        .map(|(label, value)| {
            // Generate a contextual snippet: find first match position, extract ±100 chars
            let val_lower = value.to_lowercase();
            let snippet = if let Some(pos) = val_lower.find(&q_lower) {
                let start = pos.saturating_sub(80);
                let end = (pos + q_lower.len() + 80).min(value.len());
                let prefix = if start > 0 { "…" } else { "" };
                let suffix = if end < value.len() { "…" } else { "" };
                // Find char boundaries
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
                // Match is in label — return value preview
                value.chars().take(160).collect::<String>()
            };
            (label, value, snippet)
        })
        .collect();

    Ok(results)
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
    let conn = db.lock().expect("db lock poisoned");
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
    let conn = db.lock().expect("db lock poisoned");

    // FTS5 requires queries to be properly quoted to avoid syntax errors
    let fts_query = format!("\"{}\"", query.replace('\"', "\"\""));

    let mut stmt = conn.prepare(
        "SELECT id, content, tags, created_at
         FROM archival_memory
         WHERE archival_memory MATCH ?2 AND agent_id = ?1
         ORDER BY bm25(archival_memory)
         LIMIT ?3",
    )?;

    let rows = stmt.query_map(params![agent_id, fts_query, limit], |row| {
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
    let conn = db.lock().expect("db lock poisoned");
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

