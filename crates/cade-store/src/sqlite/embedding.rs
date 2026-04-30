//! Embedding module for semantic memory search.
//!
//! Feature-gated behind `semantic-search`. Provides:
//! - Local text embedding via `fastembed` (AllMiniLML6V2, 384-dim)
//! - sqlite-vec initialization for vector similarity search
//! - Helper functions for embedding text and searching by cosine distance

use crate::error::Result;

#[cfg(feature = "semantic-search")]
use std::sync::OnceLock;

#[cfg(feature = "semantic-search")]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

/// Embedding dimension for AllMiniLML6V2.
pub const EMBEDDING_DIM: usize = 384;

/// Lazily-initialized singleton embedding model.
#[cfg(feature = "semantic-search")]
static MODEL: OnceLock<std::result::Result<TextEmbedding, String>> = OnceLock::new();

/// Initialize the fastembed model (downloads on first use, ~50MB).
/// Returns the shared model reference. Thread-safe via OnceLock.
#[cfg(feature = "semantic-search")]
pub fn get_model() -> Result<&'static TextEmbedding> {
    let result = MODEL.get_or_init(|| {
        tracing::info!("Initializing fastembed model (AllMiniLML6V2)...");
        TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                .with_show_download_progress(true),
        )
        .map_err(|e| format!("fastembed init: {e}"))
    });
    match result {
        Ok(model) => Ok(model),
        Err(e) => Err(crate::error::Error::custom(e.clone())),
    }
}

/// Embed a single text string into a 384-dim float vector.
#[cfg(feature = "semantic-search")]
pub fn embed_text(text: &str) -> Result<Vec<f32>> {
    let model = get_model()?;
    let mut embeddings = model
        .embed(vec![text], None)
        .map_err(|e| crate::error::Error::custom(format!("embed_text: {e}")))?;
    embeddings
        .pop()
        .ok_or_else(|| crate::error::Error::custom("embed_text: empty result"))
}

/// Embed multiple texts in a single batch (more efficient than one-by-one).
#[cfg(feature = "semantic-search")]
pub fn embed_batch(texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let model = get_model()?;
    model
        .embed(texts.to_vec(), None)
        .map_err(|e| crate::error::Error::custom(format!("embed_batch: {e}")))
}

/// Register the sqlite-vec extension as a global auto-extension.
/// Must be called once before any `Connection::open()`. Also called
/// from migration 8 to ensure vec0 is available even for connections
/// opened before the first `open()` call (e.g. test helpers).
#[cfg(feature = "semantic-search")]
pub fn register_sqlite_vec() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        tracing::info!("sqlite-vec extension registered");
    });
}

/// Serialize a f32 vector to bytes for sqlite-vec (little-endian f32 array).
#[cfg(feature = "semantic-search")]
pub fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialize bytes back to f32 vector.
#[cfg(feature = "semantic-search")]
pub fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

// -- No-op stubs when feature is disabled

/// Stub: always returns an error when semantic-search is disabled.
#[cfg(not(feature = "semantic-search"))]
pub fn embed_text(_text: &str) -> Result<Vec<f32>> {
    Err(crate::error::Error::custom(
        "semantic-search feature not enabled",
    ))
}

/// Stub: no-op when semantic-search is disabled.
#[cfg(not(feature = "semantic-search"))]
pub fn register_sqlite_vec() {}

/// Returns true when the semantic-search feature is compiled in.
pub fn is_available() -> bool {
    cfg!(feature = "semantic-search")
}

// -- Semantic search queries via sqlite-vec

/// Search memory blocks by cosine similarity using sqlite-vec.
/// Returns `(block_id, label, value, distance)` tuples ordered by nearest.
#[cfg(feature = "semantic-search")]
pub fn search_memory_blocks_semantic(
    conn: &rusqlite::Connection,
    agent_id: &str,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<(String, String, String, f64)>> {
    let query_bytes = vec_to_bytes(query_embedding);
    let mut stmt = conn.prepare(
        "SELECT v.block_id, v.distance, b.label, b.value
         FROM vec_memory_blocks v
         JOIN shared_memory_blocks b ON b.id = v.block_id
         JOIN agent_memory_blocks amb ON amb.block_id = b.id AND amb.agent_id = ?1
         WHERE v.embedding MATCH ?2
         ORDER BY v.distance
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![agent_id, query_bytes, limit as i64],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    Ok(rows
        .filter_map(|r| r.ok())
        .map(|(id, dist, label, value)| (id, label, value, dist))
        .collect())
}

/// Search archival memory entries by cosine similarity.
/// Returns `(entry_id, content_snippet, distance)` tuples.
#[cfg(feature = "semantic-search")]
pub fn search_archival_semantic(
    conn: &rusqlite::Connection,
    agent_id: &str,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<(String, String, f64)>> {
    let query_bytes = vec_to_bytes(query_embedding);
    let mut stmt = conn.prepare(
        "SELECT v.entry_id, v.distance, a.content
         FROM vec_archival_memory v
         JOIN archival_memory a ON a.id = v.entry_id AND a.agent_id = ?1
         WHERE v.embedding MATCH ?2
         ORDER BY v.distance
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![agent_id, query_bytes, limit as i64],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    Ok(rows
        .filter_map(|r| r.ok())
        .map(|(id, dist, content)| (id, content, dist))
        .collect())
}

/// Search conversation messages by cosine similarity.
/// Returns `(message_id, role, content, distance)` tuples.
#[cfg(feature = "semantic-search")]
pub fn search_messages_semantic(
    conn: &rusqlite::Connection,
    agent_id: &str,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<(String, String, String, f64)>> {
    let query_bytes = vec_to_bytes(query_embedding);
    let mut stmt = conn.prepare(
        "SELECT v.message_id, v.distance, m.role, m.content
         FROM vec_messages v
         JOIN messages m ON m.id = v.message_id AND m.agent_id = ?1
         WHERE v.embedding MATCH ?2
         ORDER BY v.distance
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![agent_id, query_bytes, limit as i64],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    Ok(rows
        .filter_map(|r| r.ok())
        .map(|(id, dist, role, content)| (id, role, content, dist))
        .collect())
}

// -- Embedding insertion helpers

/// Insert or replace a memory block's embedding in the vec0 table.
#[cfg(feature = "semantic-search")]
pub fn upsert_memory_block_embedding(
    conn: &rusqlite::Connection,
    block_id: &str,
    embedding: &[f32],
) -> Result<()> {
    let bytes = vec_to_bytes(embedding);
    // vec0 tables use INSERT OR REPLACE semantics
    conn.execute(
        "INSERT OR REPLACE INTO vec_memory_blocks(block_id, embedding) VALUES (?1, ?2)",
        rusqlite::params![block_id, bytes],
    )?;
    Ok(())
}

/// Insert or replace an archival memory entry's embedding.
#[cfg(feature = "semantic-search")]
pub fn upsert_archival_embedding(
    conn: &rusqlite::Connection,
    entry_id: &str,
    embedding: &[f32],
) -> Result<()> {
    let bytes = vec_to_bytes(embedding);
    conn.execute(
        "INSERT OR REPLACE INTO vec_archival_memory(entry_id, embedding) VALUES (?1, ?2)",
        rusqlite::params![entry_id, bytes],
    )?;
    Ok(())
}

/// Insert or replace a message's embedding.
#[cfg(feature = "semantic-search")]
pub fn upsert_message_embedding(
    conn: &rusqlite::Connection,
    message_id: &str,
    embedding: &[f32],
) -> Result<()> {
    let bytes = vec_to_bytes(embedding);
    conn.execute(
        "INSERT OR REPLACE INTO vec_messages(message_id, embedding) VALUES (?1, ?2)",
        rusqlite::params![message_id, bytes],
    )?;
    Ok(())
}

/// Delete a memory block embedding when the block is removed.
#[cfg(feature = "semantic-search")]
pub fn delete_memory_block_embedding(
    conn: &rusqlite::Connection,
    block_id: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM vec_memory_blocks WHERE block_id = ?1",
        rusqlite::params![block_id],
    )?;
    Ok(())
}

// -- Reciprocal Rank Fusion (RRF) for hybrid ranking

/// Merge keyword results and semantic results using RRF.
/// Each input is a list of IDs (or labels) in rank order.
/// Returns merged IDs in fused rank order.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_available_reflects_feature_gate() {
        // This test always passes — it just verifies the function compiles.
        let _ = is_available();
    }

    #[test]
    fn rrf_merges_disjoint_lists() {
        let kw = vec!["a".to_string(), "b".to_string()];
        let sem = vec!["c".to_string(), "d".to_string()];
        let fused = reciprocal_rank_fusion(&kw, &sem, 60.0);
        assert_eq!(fused.len(), 4);
        // All IDs present
        assert!(fused.contains(&"a".to_string()));
        assert!(fused.contains(&"c".to_string()));
    }

    #[test]
    fn rrf_boosts_overlapping_results() {
        // "b" appears in both lists — it should rank higher than items in only one list
        let kw = vec!["a".to_string(), "b".to_string()];
        let sem = vec!["b".to_string(), "c".to_string()];
        let fused = reciprocal_rank_fusion(&kw, &sem, 60.0);
        assert_eq!(fused[0], "b"); // boosted by appearing in both
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
        assert_eq!(fused[0], "a"); // rank 1 in keyword, no semantic boost
    }

    #[cfg(feature = "semantic-search")]
    #[test]
    fn vec_to_bytes_roundtrip() {
        let v = vec![1.0f32, 2.0, 3.0, -0.5];
        let bytes = vec_to_bytes(&v);
        assert_eq!(bytes.len(), 16); // 4 floats × 4 bytes
        let back = bytes_to_vec(&bytes);
        assert_eq!(v, back);
    }

    #[cfg(feature = "semantic-search")]
    #[test]
    fn vec_to_bytes_empty() {
        let v: Vec<f32> = vec![];
        let bytes = vec_to_bytes(&v);
        assert!(bytes.is_empty());
        let back = bytes_to_vec(&bytes);
        assert!(back.is_empty());
    }
}
