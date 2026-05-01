//! Embedding module for semantic memory search.
//!
//! Provides:
//! - Helper functions for RRF (Reciprocal Rank Fusion) hybrid ranking.
//! - FTS5 fallback query builders (BM25).

use crate::error::Result;

/// Search memory blocks by keyword similarity using FTS5 BM25.
/// Returns `(block_id, rank, label, value)` tuples ordered by relevance.
pub fn search_memory_blocks_fts(
    conn: &rusqlite::Connection,
    agent_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<(String, f64, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT f.rowid, bm25(messages_fts) as rank, b.label, b.value
         FROM messages_fts f
         JOIN shared_memory_blocks b ON b.id = f.rowid
         JOIN agent_memory_blocks amb ON amb.block_id = b.id AND amb.agent_id = ?1
         WHERE messages_fts MATCH ?2
         ORDER BY rank
         LIMIT ?3",
    )?;
    
    // FTS5 BM25 ranks lower (better) so we order by rank ASC.
    let rows = stmt.query_map(
        rusqlite::params![agent_id, query, limit as i64],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Reciprocal Rank Fusion (RRF) for hybrid ranking.
pub fn reciprocal_rank_fusion(
    keyword_ids: &[String],
    semantic_ids: &[String],
    k: f64,
) -> Vec<String> {
    use std::collections::HashMap;
    let mut scores: HashMap<&str, f64> = HashMap::new();

    for (rank, id) in keyword_ids.iter().enumerate() {
        *scores.entry(id.as_str()).or_default() += 1.0 / (k + rank as f64 + 1.0);
    }
    for (rank, id) in semantic_ids.iter().enumerate() {
        *scores.entry(id.as_str()).or_default() += 1.0 / (k + rank as f64 + 1.0);
    }

    let mut ranked: Vec<(&str, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.into_iter().map(|(id, _)| id.to_string()).collect()
}

pub fn is_available() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrf_merges_disjoint_lists() {
        let kw = vec!["a".to_string(), "b".to_string()];
        let sem = vec!["c".to_string(), "d".to_string()];
        let fused = reciprocal_rank_fusion(&kw, &sem, 60.0);
        assert_eq!(fused.len(), 4);
        assert!(fused.contains(&"a".to_string()));
        assert!(fused.contains(&"c".to_string()));
    }

    #[test]
    fn rrf_boosts_overlapping_results() {
        let kw = vec!["a".to_string(), "b".to_string()];
        let sem = vec!["b".to_string(), "c".to_string()];
        let fused = reciprocal_rank_fusion(&kw, &sem, 60.0);
        assert_eq!(fused[0], "b");
    }

    #[test]
    fn rrf_empty_inputs() {
        let empty: Vec<String> = vec![];
        let fused = reciprocal_rank_fusion(&empty, &empty, 60.0);
        assert!(fused.is_empty());
    }

    #[test]
    fn rrf_one_empty_list() {
        let kw = vec!["a".to_string(), "b".to_string()];
        let empty: Vec<String> = vec![];
        let fused = reciprocal_rank_fusion(&kw, &empty, 60.0);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0], "a");
    }
}
