use super::*;

pub fn get_max_rowid(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<u64> {
    let conn = db.get()?;
    let sql = if conversation_id.is_some() {
        "SELECT COALESCE(MAX(rowid), 0) FROM messages WHERE agent_id = ?1 AND conversation_id = ?2"
    } else {
        "SELECT COALESCE(MAX(rowid), 0) FROM messages WHERE agent_id = ?1 AND conversation_id IS NULL"
    };
    let mut stmt = conn.prepare(sql)?;
    let val: i64 = if let Some(cid) = conversation_id {
        stmt.query_row(params![agent_id, cid], |r| r.get(0))?
    } else {
        stmt.query_row(params![agent_id], |r| r.get(0))?
    };
    Ok(val as u64)
}

pub fn last_assistant_message(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> Result<Option<MessageRow>> {
    let conn = db.get()?;

    let sql = if conversation_id.is_some() {
        "SELECT id, agent_id, conversation_id, role, content, char_count FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'assistant'
         ORDER BY created_at DESC, rowid DESC LIMIT 1"
    } else {
        "SELECT id, agent_id, conversation_id, role, content, char_count FROM messages
         WHERE agent_id = ?1 AND conversation_id IS NULL AND role = 'assistant'
         ORDER BY created_at DESC, rowid DESC LIMIT 1"
    };

    let mut stmt = conn.prepare(sql)?;
    let mut rows = if let Some(cid) = conversation_id {
        stmt.query(params![agent_id, cid])?
    } else {
        stmt.query(params![agent_id])?
    };

    if let Some(r) = rows.next()? {
        let content_str: String = r.get(4)?;
        let content: Value = serde_json::from_str(&content_str).unwrap_or(Value::Null);
        let char_count: i64 = r.get(5)?;
        Ok(Some(MessageRow {
            id: r.get(0)?,
            agent_id: r.get(1)?,
            conversation_id: r.get(2)?,
            role: r.get(3)?,
            content,
            char_count: char_count as usize,
        }))
    } else {
        Ok(None)
    }
}

pub fn get_latest_user_message(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> Result<Option<String>> {
    let conn = db.get()?;

    let sql = if conversation_id.is_some() {
        "SELECT content FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'user'
         ORDER BY created_at DESC, rowid DESC LIMIT 1"
    } else {
        "SELECT content FROM messages
         WHERE agent_id = ?1 AND conversation_id IS NULL AND role = 'user'
         ORDER BY created_at DESC, rowid DESC LIMIT 1"
    };

    let mut stmt = conn.prepare(sql)?;
    let mut rows = if let Some(cid) = conversation_id {
        stmt.query(params![agent_id, cid])?
    } else {
        stmt.query(params![agent_id])?
    };

    if let Some(r) = rows.next()? {
        let content_str: String = r.get(0)?;
        let content: Value = serde_json::from_str(&content_str).unwrap_or(Value::Null);
        Ok(content.as_str().map(String::from))
    } else {
        Ok(None)
    }
}

pub fn insert_message(db: &Db, row: &MessageRow) -> Result<()> {
    let conn = db.get()?;
    conn.execute(
        "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at, char_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            row.id,
            row.agent_id,
            row.conversation_id,
            row.role,
            row.content.to_string(),
            now_ts(),
            row.char_count as i64
        ],
    )?;
    Ok(())
}

/// Load the last `limit` messages for an agent (or a specific conversation), oldest-first.
/// If `conversation_id` is None → load messages with NULL conversation_id (legacy/default).
/// Pass `Some("")` for the stub "all messages" mode — but we don't use that; always filter.
pub fn list_messages(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
    limit: usize,
) -> Result<Vec<MessageRow>> {
    list_messages_page(db, agent_id, conversation_id, limit, 0)
}

/// Return all messages **after** the most recent compaction marker for the
/// given agent+conversation, oldest-first, excluding compaction rows.
///
/// Used by the consolidation loop so it never re-summarises turns that were
/// already covered by a previous compaction run.  When no marker exists, all
/// messages are returned (same as `list_messages`).
pub fn list_messages_since_last_compaction(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
    limit: usize,
) -> Result<Vec<MessageRow>> {
    let conn = db.get()?;

    let sql = if conversation_id.is_some() {
        "WITH boundary AS (
             SELECT COALESCE(
                 (SELECT rowid FROM messages
                  WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'compaction'
                  ORDER BY created_at DESC, rowid DESC LIMIT 1),
                 0
             ) AS marker_rowid
         )
         SELECT id, agent_id, conversation_id, role, content, char_count
         FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2
           AND role != 'compaction'
           AND rowid > (SELECT marker_rowid FROM boundary)
         ORDER BY created_at ASC, rowid ASC
         LIMIT ?3"
    } else {
        "WITH boundary AS (
             SELECT COALESCE(
                 (SELECT rowid FROM messages
                  WHERE agent_id = ?1 AND conversation_id IS NULL AND role = 'compaction'
                  ORDER BY created_at DESC, rowid DESC LIMIT 1),
                 0
             ) AS marker_rowid
         )
         SELECT id, agent_id, conversation_id, role, content, char_count
         FROM messages
         WHERE agent_id = ?1 AND conversation_id IS NULL
           AND role != 'compaction'
           AND rowid > (SELECT marker_rowid FROM boundary)
         ORDER BY created_at ASC, rowid ASC
         LIMIT ?3"
    };

    let mut stmt = conn.prepare(sql)?;
    let conv_placeholder = conversation_id.unwrap_or("");
    let rows = stmt
        .query_map(params![agent_id, conv_placeholder, limit as i64], |row| {
            Ok(MessageRow {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                conversation_id: row.get(2)?,
                role: row.get(3)?,
                content: {
                    let s: String = row.get(4)?;
                    serde_json::from_str(&s).unwrap_or(serde_json::Value::Null)
                },
                char_count: row.get(5).unwrap_or(0),
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(rows)
}

/// Page through messages with limit/offset, newest-first at the SQL level,
/// returned oldest-first for convenience.
pub fn list_messages_page(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<Vec<MessageRow>> {
    let conn = db.get()?;
    // Filter: conversation_id IS NULL for legacy messages, or matches given id.
    // Exclude compaction markers — they are DB-level sentinels, not real messages.
    let sql = if conversation_id.is_some() {
        "SELECT id, agent_id, conversation_id, role, content, char_count FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2 AND role != 'compaction'
         ORDER BY created_at DESC, rowid DESC LIMIT ?3 OFFSET ?4"
    } else {
        "SELECT id, agent_id, conversation_id, role, content, char_count FROM messages
         WHERE agent_id = ?1 AND conversation_id IS NULL AND role != 'compaction'
         ORDER BY created_at DESC, rowid DESC LIMIT ?3 OFFSET ?4"
    };

    let mut stmt = conn.prepare(sql)?;
    let conv_placeholder = conversation_id.unwrap_or("");
    let rows = stmt.query_map(
        params![agent_id, conv_placeholder, limit as i64, offset as i64],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;
    let mut result: Vec<MessageRow> = rows
        .filter_map(|r| r.ok())
        .map(
            |(id, agent_id, conversation_id, role, content, char_count)| MessageRow {
                id,
                agent_id,
                conversation_id,
                role,
                content: serde_json::from_str(&content).unwrap_or(Value::String(content)),
                char_count: char_count as usize,
            },
        )
        .collect();
    // list_messages historically returned oldest-first; keep that invariant here
    result.reverse();
    Ok(result)
}

/// Fetch messages backwards until the cumulative char_count exceeds the budget.
/// This offloads context assembly math into SQLite using a window function.
///
/// When compaction markers exist (`role = 'compaction'`), the query only scans
/// messages AFTER the most recent marker — everything before it is already
/// captured in the marker's summary (stored in `session_summary`).
/// Compaction markers themselves are excluded from the returned messages.
pub fn get_context_window(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
    char_budget: usize,
) -> Result<Vec<MessageRow>> {
    let conn = db.get()?;

    // The CTE `boundary` finds the rowid of the most recent compaction marker.
    // If none exists, COALESCE falls back to 0 (scan all messages).
    // The `ranked` CTE then only considers messages with rowid > boundary
    // and role != 'compaction', applying the usual char_budget windowing.
    let sql = if conversation_id.is_some() {
        "WITH boundary AS (
             SELECT COALESCE(
                 (SELECT rowid FROM messages
                  WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'compaction'
                  ORDER BY created_at DESC, rowid DESC LIMIT 1),
                 0
             ) AS marker_rowid
         ),
         ranked AS (
             SELECT id, agent_id, conversation_id, role, content, char_count, created_at, rowid,
                    SUM(char_count) OVER (ORDER BY created_at DESC, rowid DESC) as running_total
             FROM messages
             WHERE agent_id = ?1 AND conversation_id = ?2
               AND role != 'compaction'
               AND rowid > (SELECT marker_rowid FROM boundary)
         )
         SELECT id, agent_id, conversation_id, role, content, char_count
         FROM ranked
         WHERE running_total - char_count <= ?3
         ORDER BY created_at DESC, rowid DESC"
    } else {
        "WITH boundary AS (
             SELECT COALESCE(
                 (SELECT rowid FROM messages
                  WHERE agent_id = ?1 AND conversation_id IS NULL AND role = 'compaction'
                  ORDER BY created_at DESC, rowid DESC LIMIT 1),
                 0
             ) AS marker_rowid
         ),
         ranked AS (
             SELECT id, agent_id, conversation_id, role, content, char_count, created_at, rowid,
                    SUM(char_count) OVER (ORDER BY created_at DESC, rowid DESC) as running_total
             FROM messages
             WHERE agent_id = ?1 AND conversation_id IS NULL
               AND role != 'compaction'
               AND rowid > (SELECT marker_rowid FROM boundary)
         )
         SELECT id, agent_id, conversation_id, role, content, char_count
         FROM ranked
         WHERE running_total - char_count <= ?3
         ORDER BY created_at DESC, rowid DESC"
    };

    let mut stmt = conn.prepare(sql)?;
    let conv_placeholder = conversation_id.unwrap_or("");
    let rows = stmt.query_map(
        params![agent_id, conv_placeholder, char_budget as i64],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;

    let mut result: Vec<MessageRow> = rows
        .filter_map(|r| r.ok())
        .map(
            |(id, agent_id, conversation_id, role, content, char_count)| MessageRow {
                id,
                agent_id,
                conversation_id,
                role,
                content: serde_json::from_str(&content).unwrap_or(Value::String(content)),
                char_count: char_count as usize,
            },
        )
        .collect();

    // The query returns newest-first because of ORDER BY ... DESC.
    // The calling code expects oldest-first.
    result.reverse();
    Ok(result)
}

/// Compact old tool-result message content in the DB.
///
/// Walks tool messages backwards (newest → oldest). Keeps the most recent
/// `protect_chars` worth of tool output at full fidelity. Any older tool
/// messages with `char_count > min_chars` have their content replaced with
/// a compact placeholder, freeing context space without deleting the message.
///
/// Returns the number of rows compacted.
pub fn compact_old_tool_outputs(
    db: &Db,
    agent_id: &str,
    conversation_id: Option<&str>,
    protect_chars: usize,
    min_chars: usize,
) -> Result<usize> {
    let conn = db.get()?;

    // Find all tool messages ordered newest-first.
    let sql = if conversation_id.is_some() {
        "SELECT rowid, char_count FROM messages
         WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'tool'
         ORDER BY created_at DESC, rowid DESC"
    } else {
        "SELECT rowid, char_count FROM messages
         WHERE agent_id = ?1 AND conversation_id IS NULL AND role = 'tool'
         ORDER BY created_at DESC, rowid DESC"
    };

    let mut stmt = conn.prepare(sql)?;
    let conv = conversation_id.unwrap_or("");
    let rows: Vec<(i64, i64)> = if conversation_id.is_some() {
        let mapped = stmt.query_map(params![agent_id, conv], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        mapped.filter_map(|r| r.ok()).collect()
    } else {
        let mapped = stmt.query_map(params![agent_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        mapped.filter_map(|r| r.ok()).collect()
    };

    // Walk newest-first, accumulating total chars. Once we exceed protect_chars,
    // everything older with char_count > min_chars is eligible for compaction.
    let mut cumulative = 0usize;
    let mut to_compact: Vec<(i64, i64)> = Vec::new();

    for &(rowid, char_count) in &rows {
        let cc = char_count as usize;
        cumulative += cc;
        if cumulative > protect_chars && cc > min_chars {
            to_compact.push((rowid, char_count));
        }
    }

    if to_compact.is_empty() {
        return Ok(0);
    }

    // Compact each eligible row: replace content with a placeholder.
    let update_sql = "UPDATE messages SET content = ?1, char_count = ?2 WHERE rowid = ?3";
    let mut compacted = 0usize;
    for (rowid, original_chars) in &to_compact {
        let placeholder = serde_json::json!({
            "content": format!("[tool output compacted — {} chars]", original_chars),
            "tool_call_id": null
        });
        let placeholder_str = placeholder.to_string();
        let new_char_count = placeholder_str.len() as i64;
        conn.execute(update_sql, params![placeholder_str, new_char_count, rowid])?;
        compacted += 1;
    }

    Ok(compacted)
}

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
            },
        )?;
        Ok(())
    }

    #[test]
    fn test_insert_and_list_messages() -> Result<()> {
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
                content: json!("hi there"),
                char_count: 8,
            },
        )?;

        let msgs = list_messages(&db, "a1", None, 10)?;
        assert_eq!(msgs.len(), 2);
        Ok(())
    }

    #[test]
    fn test_last_assistant_message() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // No messages yet
        let last = last_assistant_message(&db, "a1", None)?;
        assert!(last.is_none());

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
                content: json!("response"),
                char_count: 8,
            },
        )?;

        let last = last_assistant_message(&db, "a1", None)?;
        assert!(last.is_some());
        assert_eq!(last.unwrap().id, "m2");
        Ok(())
    }

    #[test]
    fn test_list_messages_page() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        for i in 0..5 {
            insert_message(
                &db,
                &MessageRow {
                    id: format!("m{i}"),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: "user".into(),
                    content: json!(format!("msg {i}")),
                    char_count: 5,
                },
            )?;
        }

        let page1 = list_messages_page(&db, "a1", None, 2, 0)?;
        assert_eq!(page1.len(), 2);

        let page2 = list_messages_page(&db, "a1", None, 2, 2)?;
        assert_eq!(page2.len(), 2);

        let page3 = list_messages_page(&db, "a1", None, 2, 4)?;
        assert_eq!(page3.len(), 1);
        Ok(())
    }

    #[test]
    fn test_get_context_window() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Insert messages with known char_count
        for i in 0..10 {
            insert_message(
                &db,
                &MessageRow {
                    id: format!("m{i}"),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                    content: json!(format!("message number {i}")),
                    char_count: 20,
                },
            )?;
        }

        // Large budget → all messages
        let all = get_context_window(&db, "a1", None, 999_999)?;
        assert_eq!(all.len(), 10);

        // Tiny budget → only the most recent messages
        let few = get_context_window(&db, "a1", None, 50)?;
        assert!(few.len() < 10);
        assert!(!few.is_empty());
        Ok(())
    }

    #[test]
    fn test_compact_old_tool_outputs_basic() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Insert a user message and several tool messages with large content
        insert_message(
            &db,
            &MessageRow {
                id: "m0".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!({"content": "hello"}),
                char_count: 5,
            },
        )?;

        let big_content = "x".repeat(500);
        for i in 1..=5 {
            insert_message(
                &db,
                &MessageRow {
                    id: format!("t{i}"),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: "tool".into(),
                    content: json!({"content": big_content, "tool_call_id": format!("tc{i}")}),
                    char_count: 500,
                },
            )?;
        }

        // protect_chars=1000 → keeps ~2 recent tool outputs; older 3 get compacted
        // min_chars=100 → all 500-char tool outputs are eligible
        let compacted = compact_old_tool_outputs(&db, "a1", None, 1000, 100)?;
        assert!(compacted > 0, "should have compacted some tool outputs");
        assert!(compacted <= 5, "should not exceed total tool messages");

        // Verify compacted messages have placeholder content
        let msgs = list_messages(&db, "a1", None, 100)?;
        let tool_msgs: Vec<_> = msgs.iter().filter(|m| m.role == "tool").collect();
        let compacted_count = tool_msgs
            .iter()
            .filter(|m| {
                m.content["content"]
                    .as_str()
                    .unwrap_or("")
                    .contains("compacted")
            })
            .count();
        assert_eq!(compacted_count, compacted);

        Ok(())
    }

    #[test]
    fn test_compact_skips_small_outputs() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Insert tool messages with small content
        for i in 1..=3 {
            insert_message(
                &db,
                &MessageRow {
                    id: format!("t{i}"),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: "tool".into(),
                    content: json!({"content": "ok", "tool_call_id": format!("tc{i}")}),
                    char_count: 2,
                },
            )?;
        }

        // min_chars=100 → none qualify (all are 2 chars)
        let compacted = compact_old_tool_outputs(&db, "a1", None, 0, 100)?;
        assert_eq!(compacted, 0);
        Ok(())
    }

    #[test]
    fn test_compact_noop_when_no_tool_messages() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        insert_message(
            &db,
            &MessageRow {
                id: "m1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!({"content": "hello"}),
                char_count: 5,
            },
        )?;

        let compacted = compact_old_tool_outputs(&db, "a1", None, 0, 0)?;
        assert_eq!(compacted, 0);
        Ok(())
    }

    // ── Compaction marker tests ───────────────────────────────────────────

    /// Helper: insert a message with a specific created_at timestamp.
    fn insert_message_at(db: &Db, row: &MessageRow, ts: i64) -> Result<()> {
        let conn = db.get()?;
        conn.execute(
            "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at, char_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                row.id,
                row.agent_id,
                row.conversation_id,
                row.role,
                row.content.to_string(),
                ts,
                row.char_count as i64,
            ],
        )?;
        Ok(())
    }

    #[test]
    fn test_list_messages_excludes_compaction_markers() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Insert user, assistant, compaction marker, then another user
        insert_message_at(
            &db,
            &MessageRow {
                id: "m1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!({"content": "hello"}),
                char_count: 5,
            },
            100,
        )?;
        insert_message_at(
            &db,
            &MessageRow {
                id: "m2".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "assistant".into(),
                content: json!({"content": "hi"}),
                char_count: 2,
            },
            101,
        )?;
        insert_message_at(
            &db,
            &MessageRow {
                id: "c1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "compaction".into(),
                content: json!({"content": "[Compaction marker: 1 turn]"}),
                char_count: 0,
            },
            102,
        )?;
        insert_message_at(
            &db,
            &MessageRow {
                id: "m3".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!({"content": "next"}),
                char_count: 4,
            },
            103,
        )?;

        // list_messages_page should return 3 messages (no compaction)
        let msgs = list_messages_page(&db, "a1", None, 100, 0)?;
        assert_eq!(msgs.len(), 3, "compaction marker should be excluded");
        assert!(msgs.iter().all(|m| m.role != "compaction"));
        Ok(())
    }

    #[test]
    fn test_get_context_window_stops_at_compaction_marker() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Insert 3 old messages, then a compaction marker, then 2 new messages.
        for i in 1..=3 {
            insert_message_at(
                &db,
                &MessageRow {
                    id: format!("old{i}"),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: "user".into(),
                    content: json!({"content": format!("old message {i}")}),
                    char_count: 15,
                },
                100 + i as i64,
            )?;
        }
        insert_message_at(
            &db,
            &MessageRow {
                id: "compact1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "compaction".into(),
                content: json!({"content": "[Compaction marker]"}),
                char_count: 0,
            },
            104,
        )?;
        for i in 1..=2 {
            insert_message_at(
                &db,
                &MessageRow {
                    id: format!("new{i}"),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: "user".into(),
                    content: json!({"content": format!("new message {i}")}),
                    char_count: 15,
                },
                104 + i as i64,
            )?;
        }

        // With a large budget, should only get the 2 new messages (after marker)
        let window = get_context_window(&db, "a1", None, 999_999)?;
        assert_eq!(
            window.len(),
            2,
            "should only load messages after compaction marker"
        );
        assert!(window.iter().all(|m| m.id.starts_with("new")));
        Ok(())
    }

    #[test]
    fn test_get_context_window_no_marker_loads_all() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Insert 5 messages with no compaction marker
        for i in 1..=5 {
            insert_message_at(
                &db,
                &MessageRow {
                    id: format!("m{i}"),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: "user".into(),
                    content: json!({"content": format!("msg {i}")}),
                    char_count: 5,
                },
                100 + i as i64,
            )?;
        }

        // All 5 should load (backward compatible)
        let window = get_context_window(&db, "a1", None, 999_999)?;
        assert_eq!(window.len(), 5);
        Ok(())
    }

    #[test]
    fn test_get_context_window_multiple_markers_uses_latest() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Old messages
        insert_message_at(
            &db,
            &MessageRow {
                id: "old1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!({"content": "old"}),
                char_count: 3,
            },
            100,
        )?;
        // First compaction marker
        insert_message_at(
            &db,
            &MessageRow {
                id: "c1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "compaction".into(),
                content: json!({"content": "[marker 1]"}),
                char_count: 0,
            },
            101,
        )?;
        // Middle messages
        insert_message_at(
            &db,
            &MessageRow {
                id: "mid1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!({"content": "middle"}),
                char_count: 6,
            },
            102,
        )?;
        // Second (latest) compaction marker
        insert_message_at(
            &db,
            &MessageRow {
                id: "c2".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "compaction".into(),
                content: json!({"content": "[marker 2]"}),
                char_count: 0,
            },
            103,
        )?;
        // New messages
        insert_message_at(
            &db,
            &MessageRow {
                id: "new1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!({"content": "new"}),
                char_count: 3,
            },
            104,
        )?;

        // Should only get the 1 message after the latest marker
        let window = get_context_window(&db, "a1", None, 999_999)?;
        assert_eq!(window.len(), 1);
        assert_eq!(window[0].id, "new1");
        Ok(())
    }

    // ── list_messages_since_last_compaction ──────────────────────────────────

    #[test]
    fn test_since_compaction_no_marker_returns_all() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        for (i, id) in ["m1", "m2", "m3"].iter().enumerate() {
            insert_message_at(
                &db,
                &MessageRow {
                    id: id.to_string(),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: "user".into(),
                    content: json!({"content": format!("msg{i}")}),
                    char_count: 4,
                },
                100 + i as i64,
            )?;
        }

        let rows = list_messages_since_last_compaction(&db, "a1", None, 100)?;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, "m1"); // oldest first
        Ok(())
    }

    #[test]
    fn test_since_compaction_with_marker_skips_old() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        // Pre-marker messages
        insert_message_at(
            &db,
            &MessageRow {
                id: "pre1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!({"content": "old"}),
                char_count: 3,
            },
            100,
        )?;
        insert_message_at(
            &db,
            &MessageRow {
                id: "pre2".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "assistant".into(),
                content: json!({"content": "old reply"}),
                char_count: 9,
            },
            101,
        )?;
        // Compaction marker
        insert_message_at(
            &db,
            &MessageRow {
                id: "c1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "compaction".into(),
                content: json!({"content": "[compacted 2 turns]"}),
                char_count: 0,
            },
            102,
        )?;
        // Post-marker messages
        insert_message_at(
            &db,
            &MessageRow {
                id: "post1".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "user".into(),
                content: json!({"content": "new"}),
                char_count: 3,
            },
            103,
        )?;
        insert_message_at(
            &db,
            &MessageRow {
                id: "post2".into(),
                agent_id: "a1".into(),
                conversation_id: None,
                role: "assistant".into(),
                content: json!({"content": "new reply"}),
                char_count: 9,
            },
            104,
        )?;

        let rows = list_messages_since_last_compaction(&db, "a1", None, 100)?;
        // Only post-marker messages, no compaction row itself
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "post1");
        assert_eq!(rows[1].id, "post2");
        Ok(())
    }

    #[test]
    fn test_since_compaction_uses_latest_marker() -> Result<()> {
        let db = setup_mem_db()?;
        make_agent(&db, "a1")?;

        for (ts, id, role) in [
            (100, "m1", "user"),
            (101, "c1", "compaction"),
            (102, "m2", "user"),
            (103, "c2", "compaction"), // latest marker
            (104, "m3", "user"),
            (105, "m4", "assistant"),
        ] {
            insert_message_at(
                &db,
                &MessageRow {
                    id: id.into(),
                    agent_id: "a1".into(),
                    conversation_id: None,
                    role: role.into(),
                    content: json!({"content": id}),
                    char_count: if role == "compaction" { 0 } else { 4 },
                },
                ts,
            )?;
        }

        let rows = list_messages_since_last_compaction(&db, "a1", None, 100)?;
        // Only m3 and m4, after the latest (c2) marker
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "m3");
        assert_eq!(rows[1].id, "m4");
        Ok(())
    }
}

// endregion: --- Tests
