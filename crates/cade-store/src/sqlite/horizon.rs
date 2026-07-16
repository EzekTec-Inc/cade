use crate::error::Result;
use super::Db;
use super::runs::MessageRow;
use rusqlite::params;

pub struct TimelineHorizon;

impl TimelineHorizon {
    /// Advance the horizon for an agent (and optional conversation) to the given boundary message.
    ///
    /// This writes a compaction marker into the sqlite database at the timestamp of the boundary message.
    pub fn advance(
        db: &Db,
        agent_id: &str,
        conversation_id: Option<&str>,
        boundary_msg_id: &str,
        dropped_turns: usize,
    ) -> Result<()> {
        let conn = db.get()?;

        // Fetch the timestamp of the boundary message
        let marker_ts: i64 = conn.query_row(
            "SELECT created_at FROM messages WHERE id = ?1",
            params![boundary_msg_id],
            |r| r.get(0),
        ).unwrap_or_else(|_| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        });

        let marker_id = format!("compact-{}", uuid::Uuid::new_v4());
        let marker_content = serde_json::json!({
            "content": format!(
                "[Compaction marker: {} turns summarised into session_summary]",
                dropped_turns,
            ),
        });

        conn.execute(
            "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at, char_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                marker_id,
                agent_id,
                conversation_id,
                "compaction",
                marker_content.to_string(),
                marker_ts,
                0i64,
            ],
        )?;

        Ok(())
    }

    /// Retrieve all messages for the agent (and optional conversation) since the last compaction marker.
    ///
    /// This defines the "active timeline" visible to the agent/client.
    pub fn get_visible_messages(
        db: &Db,
        agent_id: &str,
        conversation_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MessageRow>> {
        let conn = db.get()?;

        let sql = if conversation_id.is_some() {
            "WITH boundary AS (
                 SELECT COALESCE(
                     (SELECT created_at FROM messages
                      WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'compaction'
                      ORDER BY created_at DESC, rowid DESC LIMIT 1),
                     -1
                 ) AS marker_ts
             )
             SELECT id, agent_id, conversation_id, role, content, char_count
             FROM messages
             WHERE agent_id = ?1 AND conversation_id = ?2
               AND role != 'compaction'
               AND created_at > (SELECT marker_ts FROM boundary)
             ORDER BY created_at ASC, rowid ASC
             LIMIT ?3"
        } else {
            "WITH boundary AS (
                 SELECT COALESCE(
                     (SELECT created_at FROM messages
                      WHERE agent_id = ?1 AND conversation_id IS NULL AND role = 'compaction'
                      ORDER BY created_at DESC, rowid DESC LIMIT 1),
                     -1
                 ) AS marker_ts
             )
             SELECT id, agent_id, conversation_id, role, content, char_count
             FROM messages
             WHERE agent_id = ?1 AND conversation_id IS NULL
               AND role != 'compaction'
               AND created_at > (SELECT marker_ts FROM boundary)
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
                    char_count: row.get::<_, i64>(5).unwrap_or(0).max(0) as usize,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// Check if the agent has any compaction markers in history.
    pub fn has_compaction_marker(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<bool> {
        let conn = db.get()?;
        let count: i64 = if let Some(cid) = conversation_id {
            conn.query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE agent_id = ?1 AND conversation_id = ?2 AND role = 'compaction'",
                params![agent_id, cid],
                |r| r.get(0),
            )?
        } else {
            conn.query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE agent_id = ?1 AND conversation_id IS NULL AND role = 'compaction'",
                params![agent_id],
                |r| r.get(0),
            )?
        };
        Ok(count > 0)
    }
}
