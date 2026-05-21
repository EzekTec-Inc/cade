//! Embedding module for semantic memory search.
//!
//! Provides:
//! - The [`Embedder`] trait — a small abstraction over text → `Vec<f32>`
//!   embedding implementations. Always defined so the rest of the codebase
//!   has a stable type, regardless of the `semantic-search` feature.
//! - [`NoopEmbedder`] — always-available stub. `dimension() == 0`, all
//!   embed calls return empty vectors. Lets default builds carry an
//!   `Option<Box<dyn Embedder>>` without conditional compilation.
//! - Helper functions for RRF (Reciprocal Rank Fusion) hybrid ranking.
//! - FTS5 query builder for memory-block keyword search.

use crate::error::Result;

/// Compute embeddings for memory block text.
///
/// Implementations must be `Send + Sync` so the embedder can be stored
/// behind `Arc<dyn Embedder>` and shared across async tasks.
pub trait Embedder: Send + Sync {
    /// Embed a single piece of text.
    fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a batch of texts. Default impl iterates [`embed`].
    /// Backend-specific implementations may override for throughput.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// The embedding dimension. `0` means "no embedding" (e.g. `NoopEmbedder`).
    fn dimension(&self) -> usize;
}

/// Always-available no-op embedder. Returns empty vectors and dimension `0`.
///
/// Used in default builds (no `semantic-search` feature) so the rest of the
/// codebase can carry an `Option<Arc<dyn Embedder>>` field uniformly.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopEmbedder;

impl Embedder for NoopEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(Vec::new())
    }

    fn dimension(&self) -> usize {
        0
    }
}

/// Local ONNX-runtime embedder using `fastembed`'s `all-MiniLM-L6-v2`
/// quantised model (384 dims).
///
/// Only compiled when the `semantic-search` feature is enabled. First
/// instantiation downloads the model (~25 MB) into the user cache directory
/// — subsequent calls reuse the cached weights.
#[cfg(feature = "semantic-search")]
pub struct FastEmbedder {
    inner: fastembed::TextEmbedding,
    dim: usize,
}

#[cfg(feature = "semantic-search")]
impl FastEmbedder {
    /// Create a new embedder backed by the quantised MiniLM-L6-v2 model.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying `fastembed::TextEmbedding`
    /// initialisation fails (e.g. model download / cache error).
    pub fn new() -> Result<Self> {
        use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
        let inner = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2Q))
            .map_err(|e| crate::error::Error::Custom(format!("fastembed init failed: {e}")))?;
        // MiniLM-L6-v2 produces 384-dim embeddings.
        Ok(Self { inner, dim: 384 })
    }
}

#[cfg(feature = "semantic-search")]
impl Embedder for FastEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut out = self
            .inner
            .embed(vec![text], None)
            .map_err(|e| crate::error::Error::Custom(format!("fastembed embed failed: {e}")))?;
        out.pop()
            .ok_or_else(|| crate::error::Error::Custom("fastembed returned empty batch".into()))
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.inner
            .embed(texts.to_vec(), None)
            .map_err(|e| crate::error::Error::Custom(format!("fastembed embed_batch failed: {e}")))
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

/// Search memory blocks by keyword similarity using FTS5 BM25.
/// Returns `(block_id, rank, label, value)` tuples ordered by relevance.
///
/// WI-SEMANTIC Phase 1 fix: this previously queried `messages_fts` (the
/// conversation-history FTS) and joined on `b.id = f.rowid`, which never
/// matched because `shared_memory_blocks.id` is a TEXT UUID, not the
/// integer rowid. Now queries the dedicated `memory_blocks_fts` virtual
/// table introduced by Migration 10, and joins on the integer rowid that
/// FTS5 stores via `content_rowid='rowid'`.
pub fn search_memory_blocks_fts(
    conn: &rusqlite::Connection,
    agent_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<(String, f64, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, bm25(memory_blocks_fts) AS rank, b.label, b.value
         FROM memory_blocks_fts f
         JOIN shared_memory_blocks b ON b.rowid = f.rowid
         JOIN agent_memory_blocks amb ON amb.block_id = b.id AND amb.agent_id = ?1
         WHERE memory_blocks_fts MATCH ?2
         ORDER BY rank
         LIMIT ?3",
    )?;

    // FTS5 BM25 ranks lower (better) so we order by rank ASC.
    let rows = stmt.query_map(rusqlite::params![agent_id, query, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, f64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Compute and store embeddings for every `shared_memory_blocks` row that
/// currently has `embedding IS NULL`. Returns the number of rows processed.
///
/// Intended to be called once at server startup when the `semantic-search`
/// feature is enabled, so that DBs created before embeddings existed get
/// upgraded incrementally. Failures on individual rows (e.g. an embedder
/// hiccup on a single value) are logged and skipped — the function returns
/// the count of rows successfully written.
pub fn backfill_embeddings(db: &super::Db, embedder: &dyn Embedder) -> Result<usize> {
    // Snapshot the (id, value) pairs under a short-lived lock so we don't
    // hold the connection while running CPU-bound embedding work.
    let pending_blocks: Vec<(String, String)> = {
        let conn = db.get()?;
        let mut stmt =
            conn.prepare("SELECT id, value FROM shared_memory_blocks WHERE embedding IS NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let pending_chunks: Vec<(String, String)> = {
        let conn = db.get()?;
        let mut stmt =
            conn.prepare("SELECT id, content FROM memory_chunks WHERE embedding IS NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    if pending_blocks.is_empty() && pending_chunks.is_empty() {
        return Ok(0);
    }

    let mut written = 0usize;
    for (id, value) in &pending_blocks {
        let vec = match embedder.embed(value) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("backfill_embeddings: embedder failed on block id {id}: {e}");
                continue;
            }
        };
        if vec.is_empty() {
            continue;
        }
        let mut bytes: Vec<u8> = Vec::with_capacity(vec.len() * 4);
        for f in &vec {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        let conn = db.get()?;
        match conn.execute(
            "UPDATE shared_memory_blocks SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![bytes, id],
        ) {
            Ok(_) => written += 1,
            Err(e) => tracing::warn!("backfill_embeddings: UPDATE failed on block id {id}: {e}"),
        }
    }

    for (id, value) in &pending_chunks {
        let vec = match embedder.embed(value) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("backfill_embeddings: embedder failed on chunk id {id}: {e}");
                continue;
            }
        };
        if vec.is_empty() {
            continue;
        }
        let mut bytes: Vec<u8> = Vec::with_capacity(vec.len() * 4);
        for f in &vec {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        let conn = db.get()?;
        match conn.execute(
            "UPDATE memory_chunks SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![bytes, id],
        ) {
            Ok(_) => written += 1,
            Err(e) => tracing::warn!("backfill_embeddings: UPDATE failed on chunk id {id}: {e}"),
        }
    }

    Ok(written)
}

/// Decode a packed little-endian f32 BLOB back into a `Vec<f32>`.
/// Returns `None` if the byte slice is not a multiple of 4 bytes long.
fn decode_embedding_blob(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().ok()?;
        out.push(f32::from_le_bytes(arr));
    }
    Some(out)
}

/// Cosine similarity between two equal-length vectors.
/// Returns `None` if either vector is empty or the lengths differ.
fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return None;
    }
    Some(dot / (na.sqrt() * nb.sqrt()))
}

pub fn search_memory_semantic(
    conn: &rusqlite::Connection,
    agent_id: &str,
    query_embedding: &[f32],
    memory_type: Option<&str>,
    limit: usize,
) -> Result<Vec<(String, f64, String, String)>> {
    if query_embedding.is_empty() {
        return Ok(Vec::new());
    }

    let rows = if let Some(mtype) = memory_type {
        let mut stmt = conn.prepare(
            "SELECT b.id, b.label, b.value, b.embedding
             FROM shared_memory_blocks b
             JOIN agent_memory_blocks amb ON amb.block_id = b.id
             WHERE amb.agent_id = ?1 AND b.memory_type = ?2 AND b.embedding IS NOT NULL
             UNION ALL
             SELECT b.id, b.label, b.value, c.embedding
             FROM memory_chunks c
             JOIN shared_memory_blocks b ON b.id = c.block_id
             JOIN agent_memory_blocks amb ON amb.block_id = b.id
             WHERE amb.agent_id = ?1 AND b.memory_type = ?2 AND c.embedding IS NOT NULL",
        )?;
        stmt.query_map(rusqlite::params![agent_id, mtype], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>()
    } else {
        let mut stmt = conn.prepare(
            "SELECT b.id, b.label, b.value, b.embedding
             FROM shared_memory_blocks b
             JOIN agent_memory_blocks amb ON amb.block_id = b.id
             WHERE amb.agent_id = ?1 AND b.embedding IS NOT NULL
             UNION ALL
             SELECT b.id, b.label, b.value, c.embedding
             FROM memory_chunks c
             JOIN shared_memory_blocks b ON b.id = c.block_id
             JOIN agent_memory_blocks amb ON amb.block_id = b.id
             WHERE amb.agent_id = ?1 AND c.embedding IS NOT NULL",
        )?;
        stmt.query_map(rusqlite::params![agent_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect::<Vec<_>>()
    };

    let mut scored_map: std::collections::HashMap<String, (f64, String, String)> =
        std::collections::HashMap::new();
    for r in rows {
        let (id, label, value, blob) = r;
        let Some(vec) = decode_embedding_blob(&blob) else {
            continue;
        };
        let Some(sim) = cosine_similarity(query_embedding, &vec) else {
            continue;
        };

        // Keep the highest similarity for this block id
        let current_best = scored_map
            .get(&id)
            .map(|v| v.0)
            .unwrap_or(f64::NEG_INFINITY);
        if sim as f64 > current_best {
            scored_map.insert(id, (sim as f64, label, value));
        }
    }

    let mut scored: Vec<(String, f64, String, String)> = scored_map
        .into_iter()
        .map(|(id, (sim, label, value))| (id, sim, label, value))
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    Ok(scored)
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

/// `true` when the build has the `semantic-search` feature enabled (i.e.
/// a real [`Embedder`] backend such as [`FastEmbedder`] is available).
pub fn is_available() -> bool {
    cfg!(feature = "semantic-search")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_embedder_dimension_zero_and_empty_embed() -> Result<()> {
        let e = NoopEmbedder;
        assert_eq!(e.dimension(), 0);
        let v = e.embed("hello world")?;
        assert!(v.is_empty(), "NoopEmbedder must return empty vector");
        let batch = e.embed_batch(&["a", "b"])?;
        assert_eq!(batch.len(), 2);
        assert!(batch.iter().all(|v| v.is_empty()));
        Ok(())
    }

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

    #[test]
    fn is_available_reflects_feature_flag() {
        // `cfg!(feature = "semantic-search")` is the source of truth.
        assert_eq!(is_available(), cfg!(feature = "semantic-search"));
    }

    /// Smoke test for the real ONNX-runtime backed embedder.
    ///
    /// `#[ignore]` because the first run downloads ~25 MB of model weights
    /// into the user cache directory. Run manually with:
    ///   cargo test -p cade-store --features semantic-search --ignored
    #[cfg(feature = "semantic-search")]
    #[test]
    #[ignore]
    fn fast_embedder_produces_384_dim_vector() -> crate::error::Result<()> {
        let e = FastEmbedder::new()?;
        assert_eq!(e.dimension(), 384);
        let v = e.embed("hello world")?;
        assert_eq!(v.len(), 384);
        // Embeddings should not be all-zero.
        assert!(v.iter().any(|f| *f != 0.0));
        Ok(())
    }
}
