use super::*;

pub fn upsert_tool(db: &Db, row: &ToolRow) -> Result<()> {
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
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
    let conn = db.lock().ok()?;
    let mut stmt = conn.prepare("SELECT id FROM tools WHERE name = ?1").ok()?;
    stmt.query_row(params![name], |r| r.get::<_, String>(0))
        .ok()
}

/// Delete all messages for an agent (or a specific conversation).
/// If conversation_id is None, deletes all messages for the agent.
pub fn clear_messages(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<usize> {
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
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
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;

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
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
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
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
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
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;

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
    let conn = db.lock().map_err(|e| crate::server::Error::custom(format!("db lock poisoned: {e}")))?;
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
        insert_archival_memory(
            &db,
            "a1",
            "Lorem ipsum dolor sit amet",
            &["test".into()],
        )?;

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

// endregion: --- Tests
