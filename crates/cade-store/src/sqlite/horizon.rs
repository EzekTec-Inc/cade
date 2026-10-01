use super::runs::MessageRow;
use super::{Connection, Db, OptionalExtension, params};
use crate::error::{Error, Result};

/// The historical source position, never the subsequently inserted marker rowid.
/// Sequence values are backfilled from historical rowids, then assigned by an
/// insert trigger. They survive VACUUM and physical rowid reuse after deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HistoryAnchor {
    pub created_at: i64,
    pub sequence: i64,
}

// Exact membership, not a timestamp high-water mark, defines covered history.
// A historical anchor is diagnostic metadata: after clock regression, a retained
// row can precede any previous anchor. Legacy markers are interpreted once and
// frozen into coverage; their timestamp predicate never expands to future rows.
pub(super) const VISIBLE_HISTORY_CTE: &str = "WITH boundary AS (
    SELECT created_at, compaction_anchor_seq,
           COALESCE(compaction_snapshot_seq, history_seq) AS snapshot_seq
    FROM messages m WHERE agent_id = ?1 AND conversation_id IS ?2 AND role = 'compaction'
      AND NOT EXISTS (SELECT 1 FROM compaction_horizons h WHERE h.marker_id = m.id)
    ORDER BY created_at DESC, compaction_anchor_seq DESC, history_seq DESC LIMIT 1
), visible AS (
    SELECT m.*, m.history_seq AS source_sequence FROM messages m
    WHERE m.agent_id = ?1 AND m.conversation_id IS ?2 AND m.role != 'compaction'
      AND NOT EXISTS (SELECT 1 FROM compaction_coverage c WHERE c.message_seq = m.history_seq)
      AND (NOT EXISTS (SELECT 1 FROM boundary)
        OR m.history_seq > (SELECT snapshot_seq FROM boundary)
        OR m.created_at > (SELECT created_at FROM boundary)
        OR (m.created_at = (SELECT created_at FROM boundary)
            AND ((SELECT compaction_anchor_seq FROM boundary) IS NULL
                 OR m.history_seq > (SELECT compaction_anchor_seq FROM boundary)))
      )
)";

/// Convert the existing legacy boundary's unambiguous exclusions into coverage.
/// Called by migration and before publishing an exact prefix if a legacy writer
/// has inserted another marker. Original marker payloads/timestamps stay intact.
pub(super) fn cover_legacy_on(
    conn: &Connection,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> Result<()> {
    let pending: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages m WHERE agent_id = ?1
         AND conversation_id IS ?2 AND role = 'compaction'
         AND NOT EXISTS (SELECT 1 FROM compaction_horizons h WHERE h.marker_id = m.id))",
        params![agent_id, conversation_id],
        |r| r.get(0),
    )?;
    if !pending {
        return Ok(());
    }
    let sql = format!(
        "{VISIBLE_HISTORY_CTE}, legacy_markers AS (
             SELECT m.id, m.created_at, m.compaction_anchor_seq, m.compaction_snapshot_seq,
                    LAG(created_at) OVER (ORDER BY history_seq) AS previous_ts,
                    LAG(COALESCE(compaction_anchor_seq, -1)) OVER (ORDER BY history_seq) AS previous_anchor,
                    LAG(COALESCE(compaction_snapshot_seq, history_seq)) OVER (ORDER BY history_seq) AS previous_snapshot
             FROM messages m
             WHERE agent_id = ?1 AND conversation_id IS ?2 AND role = 'compaction'
         ), ambiguous AS (
             -- v23's p.max(anchor) left an identical anchor when the real source
             -- regressed, while expanding snapshot coverage. Its newly hidden
             -- interval might include retained rows; do not certify those rows
             -- merely because the broken timestamp predicate excluded them.
             SELECT previous_snapshot, compaction_snapshot_seq, created_at
             FROM legacy_markers l
             WHERE compaction_anchor_seq = previous_anchor AND created_at = previous_ts
               AND compaction_snapshot_seq > previous_snapshot
               AND NOT EXISTS (SELECT 1 FROM compaction_horizons h WHERE h.marker_id = l.id)
         )
         INSERT OR IGNORE INTO compaction_coverage (message_seq)
         SELECT m.history_seq FROM messages m
         WHERE m.agent_id = ?1 AND m.conversation_id IS ?2 AND m.role != 'compaction'
           AND NOT EXISTS (SELECT 1 FROM visible v WHERE v.id = m.id)
           AND NOT EXISTS (
               SELECT 1 FROM ambiguous a WHERE m.history_seq > a.previous_snapshot
                 AND m.history_seq <= a.compaction_snapshot_seq AND m.created_at < a.created_at
           )"
    );
    conn.execute(&sql, params![agent_id, conversation_id])?;
    conn.execute(
        "INSERT OR IGNORE INTO compaction_horizons (marker_id)
         SELECT id FROM messages WHERE agent_id = ?1 AND conversation_id IS ?2 AND role = 'compaction'",
        params![agent_id, conversation_id],
    )?;
    Ok(())
}

pub(super) fn message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRow> {
    let raw: String = row.get(4)?;
    Ok(MessageRow {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        conversation_id: row.get(2)?,
        role: row.get(3)?,
        content: serde_json::from_str(&raw).unwrap_or(serde_json::Value::String(raw)),
        char_count: row.get::<_, i64>(5)?.max(0) as usize,
    })
}

pub(super) fn latest_marker(
    conn: &Connection,
    agent_id: &str,
    conversation_id: Option<&str>,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM messages WHERE agent_id = ?1 AND conversation_id IS ?2
         AND role = 'compaction'
          ORDER BY history_seq DESC LIMIT 1",
            params![agent_id, conversation_id],
            |r| r.get(0),
        )
        .optional()?)
}

pub(super) fn advance_on(
    conn: &Connection,
    agent_id: &str,
    conversation_id: Option<&str>,
    anchor: HistoryAnchor,
    snapshot_seq: i64,
    dropped_turns: usize,
    covered_sequences: impl IntoIterator<Item = i64>,
) -> Result<()> {
    cover_legacy_on(conn, agent_id, conversation_id)?;
    for sequence in covered_sequences {
        conn.execute(
            "INSERT INTO compaction_coverage (message_seq) VALUES (?1)",
            [sequence],
        )?;
    }
    // Pin to the source actually summarized, even when its timestamp regressed.
    // Neither this timestamp nor the captured insertion bound certifies any
    // additional message: only the exact supplied sequences become covered.
    let marker_id = format!("compact-{}", uuid::Uuid::new_v4());
    let content = serde_json::json!({"content": format!(
        "[Compaction marker: {dropped_turns} turns summarised into session_summary]"
    )});
    conn.execute(
        "INSERT INTO messages (id, agent_id, conversation_id, role, content, created_at,
                               char_count, compaction_anchor_seq, compaction_snapshot_seq)
         VALUES (?1, ?2, ?3, 'compaction', ?4, ?5, 0, ?6, ?7)",
        params![
            marker_id,
            agent_id,
            conversation_id,
            content.to_string(),
            anchor.created_at,
            anchor.sequence,
            snapshot_seq
        ],
    )?;
    conn.execute(
        "INSERT INTO compaction_horizons (marker_id) VALUES (?1)",
        [marker_id],
    )?;
    Ok(())
}

pub struct TimelineHorizon;

impl TimelineHorizon {
    /// Publication revision for readers assembling memory and history separately.
    pub fn marker_id(
        db: &Db,
        agent_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<Option<String>> {
        let conn = db.get()?;
        latest_marker(&conn, agent_id, conversation_id)
    }

    /// Compatibility entry point; resolves and validates the actual historical
    /// message. Consolidation uses its captured snapshot's atomic commit instead.
    pub fn advance(
        db: &Db,
        agent_id: &str,
        conversation_id: Option<&str>,
        boundary_msg_id: &str,
        dropped_turns: usize,
    ) -> Result<()> {
        let mut conn = db.get()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let anchor = tx
            .query_row(
                "SELECT created_at, history_seq FROM messages WHERE id = ?1 AND agent_id = ?2
             AND conversation_id IS ?3 AND role != 'compaction'",
                params![boundary_msg_id, agent_id, conversation_id],
                |r| {
                    Ok(HistoryAnchor {
                        created_at: r.get(0)?,
                        sequence: r.get(1)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::custom("Compaction boundary is missing or outside scope"))?;
        // This compatibility API certifies the visible historical prefix through
        // the supplied source. Later insertions cannot be inferred as summarized.
        let sql = format!(
            "{VISIBLE_HISTORY_CTE}
             SELECT source_sequence FROM visible
             WHERE source_sequence <= ?3
               AND (created_at < ?4 OR (created_at = ?4 AND source_sequence <= ?3))"
        );
        let sequences = {
            let mut stmt = tx.prepare(&sql)?;
            let rows = stmt.query_map(
                params![
                    agent_id,
                    conversation_id,
                    anchor.sequence,
                    anchor.created_at
                ],
                |r| r.get::<_, i64>(0),
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !sequences.contains(&anchor.sequence) {
            return Err(Error::custom("Compaction boundary is already covered"));
        }
        advance_on(
            &tx,
            agent_id,
            conversation_id,
            anchor,
            anchor.sequence,
            dropped_turns,
            sequences,
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_visible_messages(
        db: &Db,
        agent_id: &str,
        conversation_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MessageRow>> {
        let conn = db.get()?;
        let sql = format!(
            "{VISIBLE_HISTORY_CTE}
            SELECT id, agent_id, conversation_id, role, content, char_count FROM visible
            ORDER BY created_at ASC, source_sequence ASC LIMIT ?3"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![agent_id, conversation_id, limit as i64],
            message_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn has_compaction_marker(
        db: &Db,
        agent_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<bool> {
        Ok(Self::marker_id(db, agent_id, conversation_id)?.is_some())
    }
}
