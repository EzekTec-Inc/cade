//! SQLite consolidation snapshot/claim and atomic summary + timeline publication.
//! The claim holds no connection or transaction while the caller awaits an LLM.

use super::horizon::{self, HistoryAnchor};
use super::runs::MessageRow;
use super::{Connection, Db, OptionalExtension, now_ts, params};
use crate::error::{Error, Result};

// Crash recovery: an abandoned claim can be replaced after fifteen minutes.
// Replacement changes its fencing token; the abandoned worker cannot commit.
const CLAIM_LEASE_SECS: i64 = 15 * 60;
pub const LIVE_CAP: usize = 8_000;
pub const ARCHIVED_CAP: usize = 4_000;
pub const INDEX_CAP: usize = 10_000;
pub const RING_CAP: usize = 8;

pub fn is_claimed(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<bool> {
    let conn = db.get()?;
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM consolidation_claims
         WHERE agent_id = ?1 AND conversation_key = ?2 AND expires_at > ?3)",
        params![agent_id, serde_json::to_string(&conversation_id)?, now_ts()],
        |r| r.get(0),
    )?)
}

#[derive(Debug, Clone, Default)]
pub struct SummaryPlan {
    pub upserts: Vec<(String, String)>,
    pub deletes: Vec<String>,
    pub append_to_index: Option<String>,
    pub archive_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedSource {
    id: String,
    anchor: HistoryAnchor,
    role: String,
    content: String,
    char_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedBlock {
    id: String,
    label: String,
    value: String,
    metadata: String,
}

fn managed_blocks(conn: &Connection, agent_id: &str) -> Result<Vec<CapturedBlock>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.label, b.value,
                json_array(b.description, b.max_chars, b.tier, b.memory_type,
                           b.confidence, b.updated_at, b.last_turn)
         FROM shared_memory_blocks b JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND (b.label IN ('session_summary', 'session_index', 'active_goal')
                                     OR b.label GLOB 'session_summary_*')
         ORDER BY b.label, b.id",
    )?;
    let rows = stmt.query_map([agent_id], |r| {
        Ok(CapturedBlock {
            id: r.get(0)?,
            label: r.get(1)?,
            value: r.get(2)?,
            metadata: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// A single captured history and managed-memory view for one agent+conversation.
/// Drop releases the claim on success, failure, early return or cancellation.
pub struct ConsolidationSnapshot {
    db: Db,
    agent_id: String,
    conversation_id: Option<String>,
    conversation_key: String,
    token: String,
    marker: Option<String>,
    snapshot_seq: i64,
    messages: Vec<MessageRow>,
    sources: Vec<CapturedSource>,
    blocks: Vec<CapturedBlock>,
    block_values: Vec<(String, String)>,
}

impl ConsolidationSnapshot {
    /// Claim and capture in one short transaction. None means another worker owns
    /// this scope. NULL and the empty conversation name are distinct scopes.
    pub fn capture(db: &Db, agent_id: &str, conversation_id: Option<&str>) -> Result<Option<Self>> {
        let mut conn = db.get()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let conversation_key = serde_json::to_string(&conversation_id)?;
        let token = uuid::Uuid::new_v4().to_string();
        let changed = tx.execute(
            "INSERT INTO consolidation_claims (agent_id, conversation_key, token, expires_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id, conversation_key) DO UPDATE
             SET token = excluded.token, expires_at = excluded.expires_at
             WHERE consolidation_claims.expires_at <= ?5",
            params![
                agent_id,
                conversation_key,
                token,
                now_ts() + CLAIM_LEASE_SECS,
                now_ts()
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        let marker = horizon::latest_marker(&tx, agent_id, conversation_id)?;
        let snapshot_seq = tx.query_row(
            "SELECT value FROM message_history_sequence WHERE singleton = 1",
            [],
            |r| r.get(0),
        )?;
        let sql = format!("{}
            SELECT id, agent_id, conversation_id, role, content, char_count, created_at, source_sequence
            FROM visible ORDER BY created_at, source_sequence", horizon::VISIBLE_HISTORY_CTE);
        let (messages, sources) = {
            let mut stmt = tx.prepare(&sql)?;
            let rows = stmt.query_map(params![agent_id, conversation_id], |r| {
                Ok((
                    horizon::message_from_row(r)?,
                    CapturedSource {
                        id: r.get(0)?,
                        role: r.get(3)?,
                        content: r.get(4)?,
                        char_count: r.get(5)?,
                        anchor: HistoryAnchor {
                            created_at: r.get(6)?,
                            sequence: r.get(7)?,
                        },
                    },
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .unzip()
        };
        let blocks = managed_blocks(&tx, agent_id)?;
        // Multiple linked blocks with the same label are ambiguous. Never guess
        // which shared identity to mutate, nor silently merge their values.
        if blocks.windows(2).any(|w| w[0].label == w[1].label) {
            return Err(Error::custom("Ambiguous consolidation memory labels"));
        }
        let block_values = blocks
            .iter()
            .map(|b| (b.label.clone(), b.value.clone()))
            .collect();
        tx.commit()?;
        Ok(Some(Self {
            db: db.clone(),
            agent_id: agent_id.to_string(),
            conversation_id: conversation_id.map(str::to_string),
            conversation_key,
            token,
            marker,
            snapshot_seq,
            messages,
            sources,
            blocks,
            block_values,
        }))
    }

    pub fn messages(&self) -> &[MessageRow] {
        &self.messages
    }
    pub fn block_values(&self) -> &[(String, String)] {
        &self.block_values
    }

    fn check_claim(&self, conn: &Connection) -> Result<()> {
        let owns: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM consolidation_claims
             WHERE agent_id = ?1 AND conversation_key = ?2 AND token = ?3 AND expires_at > ?4)",
            params![self.agent_id, self.conversation_key, self.token, now_ts()],
            |r| r.get(0),
        )?;
        if !owns {
            return Err(Error::custom("Consolidation claim expired or replaced"));
        }
        Ok(())
    }

    /// Intentional independent durable outcome: raw archival survives an LLM or
    /// publication failure. It does not advance the horizon or change summaries.
    pub fn archive_source(&self, content: &str, tags: &[String]) -> Result<String> {
        let mut conn = self.db.get()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        self.check_claim(&tx)?;
        let id = insert_archive(&tx, &self.agent_id, content, tags)?;
        tx.commit()?;
        Ok(id)
    }

    /// Publish all rotation values, revisions, links, tiers, eviction archival,
    /// index and historical marker together. Any error rolls the entire group back.
    /// Concurrent inserts are allowed; changed source rows or shared memory are
    /// rejected so the caller cannot publish a summary of a stale snapshot.
    pub fn commit(
        &self,
        dropped_messages: usize,
        dropped_turns: usize,
        plan: &SummaryPlan,
    ) -> Result<usize> {
        if dropped_messages == 0 || dropped_messages > self.sources.len() {
            return Err(Error::custom("Invalid dropped history prefix"));
        }
        if !plan
            .upserts
            .iter()
            .any(|(l, v)| l == "session_summary" && !v.trim().is_empty())
        {
            return Err(Error::custom("Consolidation plan has no live summary"));
        }
        let mut labels = std::collections::HashSet::new();
        if plan
            .upserts
            .iter()
            .any(|(label, _)| !summary_label(label) || !labels.insert(label))
            || plan.deletes.iter().any(|label| !summary_label(label))
        {
            return Err(Error::custom(
                "Invalid or duplicate consolidation summary label",
            ));
        }
        let mut conn = self.db.get()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        self.check_claim(&tx)?;
        if horizon::latest_marker(&tx, &self.agent_id, self.conversation_id.as_deref())?
            != self.marker
            || managed_blocks(&tx, &self.agent_id)? != self.blocks
        {
            return Err(Error::custom(
                "Consolidation snapshot changed before publication",
            ));
        }
        for source in &self.sources {
            let current = tx
                .query_row(
                    "SELECT id, created_at, history_seq, role, content, char_count FROM messages
                 WHERE id = ?1 AND agent_id = ?2 AND conversation_id IS ?3",
                    params![source.id, self.agent_id, self.conversation_id],
                    |r| {
                        Ok(CapturedSource {
                            id: r.get(0)?,
                            anchor: HistoryAnchor {
                                created_at: r.get(1)?,
                                sequence: r.get(2)?,
                            },
                            role: r.get(3)?,
                            content: r.get(4)?,
                            char_count: r.get(5)?,
                        })
                    },
                )
                .optional()?;
            if current.as_ref() != Some(source) {
                return Err(Error::custom(
                    "Consolidation source changed before publication",
                ));
            }
        }
        if let Some(content) = &plan.archive_content {
            insert_archive(
                &tx,
                &self.agent_id,
                content,
                &["evicted-session-summary".to_string()],
            )?;
        }
        if let Some(excerpt) = &plan.append_to_index {
            append_index_on(&tx, &self.agent_id, excerpt)?;
        }
        // A ring shift overwrites destination slots in place, preserving block
        // IDs and revision histories (including explicitly shared identities).
        for label in &plan.deletes {
            if !plan.upserts.iter().any(|(l, _)| l == label) {
                tx.execute(
                    "DELETE FROM agent_memory_blocks WHERE agent_id = ?1 AND block_id IN
                     (SELECT id FROM shared_memory_blocks WHERE label = ?2)",
                    params![self.agent_id, label],
                )?;
            }
        }
        let mut live_chars = 0;
        for (label, value) in &plan.upserts {
            let live = label == "session_summary";
            let write = super::memory::upsert_memory_block_on(
                &tx,
                &self.agent_id,
                label,
                value,
                Some(if live {
                    "Auto-generated summary of older conversation turns (Sleeptime consolidation)"
                } else {
                    "Rotated session summary (Phase C ring)"
                }),
                Some(if live { LIVE_CAP } else { ARCHIVED_CAP }),
            )?;
            if live {
                live_chars = write.stored_chars;
            }
            set_tier_on(
                &tx,
                &self.agent_id,
                label,
                if live { "pinned" } else { "long" },
            )?;
        }
        if self
            .blocks
            .iter()
            .any(|b| b.label == "active_goal" && !b.value.trim().is_empty())
        {
            set_tier_on(&tx, &self.agent_id, "active_goal", "pinned")?;
            tx.execute(
                "UPDATE shared_memory_blocks SET last_turn =
                    (SELECT memory_turn_counter FROM agents WHERE id = ?1)
                 WHERE label = 'active_goal' AND id IN
                    (SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?1)",
                [&self.agent_id],
            )?;
        }
        horizon::advance_on(
            &tx,
            &self.agent_id,
            self.conversation_id.as_deref(),
            self.sources[dropped_messages - 1].anchor,
            self.snapshot_seq,
            dropped_turns,
            self.sources[..dropped_messages]
                .iter()
                .map(|source| source.anchor.sequence),
        )?;
        tx.execute(
            "DELETE FROM consolidation_claims WHERE agent_id = ?1 AND conversation_key = ?2 AND token = ?3",
            params![self.agent_id, self.conversation_key, self.token],
        )?;
        tx.commit()?;
        Ok(live_chars)
    }
}

impl Drop for ConsolidationSnapshot {
    fn drop(&mut self) {
        if let Ok(conn) = self.db.get() {
            let _ = conn.execute(
                "DELETE FROM consolidation_claims WHERE agent_id = ?1 AND conversation_key = ?2 AND token = ?3",
                params![self.agent_id, self.conversation_key, self.token],
            );
        }
    }
}

fn summary_label(label: &str) -> bool {
    label == "session_summary"
        || label.strip_prefix("session_summary_").is_some_and(|s| {
            s.parse::<usize>()
                .is_ok_and(|n| (1..=RING_CAP).contains(&n) && s == n.to_string())
        })
}

fn insert_archive(
    conn: &Connection,
    agent_id: &str,
    content: &str,
    tags: &[String],
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO archival_memory (id, agent_id, content, tags, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, agent_id, content, serde_json::to_string(tags)?, now_ts()],
    )?;
    Ok(id)
}

fn set_tier_on(conn: &Connection, agent_id: &str, label: &str, tier: &str) -> Result<()> {
    conn.execute(
        "UPDATE shared_memory_blocks SET tier = ?1 WHERE label = ?2
         AND id IN (SELECT block_id FROM agent_memory_blocks WHERE agent_id = ?3)",
        params![tier, label, agent_id],
    )?;
    Ok(())
}

fn append_index_on(conn: &Connection, agent_id: &str, excerpt: &str) -> Result<()> {
    let line: String = excerpt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect();
    if line.is_empty() {
        return Ok(());
    }
    let existing: Option<String> = conn.query_row(
        "SELECT b.value FROM shared_memory_blocks b JOIN agent_memory_blocks amb ON amb.block_id = b.id
         WHERE amb.agent_id = ?1 AND b.label = 'session_index'",
        [agent_id], |r| r.get(0),
    ).optional()?;
    let mut combined = existing
        .filter(|v| !v.is_empty())
        .map_or_else(|| line.clone(), |v| format!("{v}\n{line}"));
    while combined.chars().count() > INDEX_CAP {
        if let Some(i) = combined.find('\n') {
            combined.drain(..=i);
        } else {
            break;
        }
    }
    super::memory::upsert_memory_block_on(
        conn,
        agent_id,
        "session_index",
        &combined,
        Some("Timeline index of evicted session summaries (Phase C)"),
        Some(INDEX_CAP),
    )?;
    set_tier_on(conn, agent_id, "session_index", "pinned")
}

/// Compatibility helper for index-only maintenance; uses the same atomic writer.
pub fn append_session_index(db: &Db, agent_id: &str, excerpt: &str) -> Result<()> {
    let mut conn = db.get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    append_index_on(&tx, agent_id, excerpt)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
#[path = "consolidation_tests.rs"]
mod tests;
