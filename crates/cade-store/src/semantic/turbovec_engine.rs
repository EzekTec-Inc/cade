//! TurboVec implementation of [`SemanticCodeIndex`] and [`VectorIndex`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use crate::error::Result;
use crate::sqlite::Db;
use crate::sqlite::embedding::{Embedder, SearchResult, VectorIndex};
use super::{CodeMatch, SearchFilter, SemanticCodeIndex};

#[cfg(feature = "turbovec")]
use parking_lot::RwLock;
#[cfg(feature = "turbovec")]
use crate::error::Error;

#[cfg(feature = "turbovec")]
/// TurboVec-backed implementation of the [`VectorIndex`] trait.
pub struct TurboVecVectorIndex {
    index: Arc<RwLock<turbovec::IdMapIndex>>,
    dim: usize,
    bit_width: usize,
    persist_path: Option<PathBuf>,
    id_to_str: Arc<RwLock<std::collections::HashMap<u64, (String, serde_json::Value)>>>,
    str_to_id: Arc<RwLock<std::collections::HashMap<String, u64>>>,
    next_id: Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(feature = "turbovec")]
impl TurboVecVectorIndex {
    /// Create or load a TurboVec vector index.
    pub fn new(dim: usize, bit_width: usize, persist_path: Option<PathBuf>) -> Result<Self> {
        let index = if let Some(ref path) = persist_path {
            if path.exists() {
                turbovec::IdMapIndex::load(path.to_str().unwrap_or_default())
                    .map_err(|e| Error::Custom(format!("Failed to load turbovec index: {e}")))?
            } else {
                turbovec::IdMapIndex::new(dim, bit_width)
                    .map_err(|e| Error::Custom(format!("Failed to create turbovec index: {e}")))?
            }
        } else {
            turbovec::IdMapIndex::new(dim, bit_width)
                .map_err(|e| Error::Custom(format!("Failed to create turbovec index: {e}")))?
        };

        Ok(Self {
            index: Arc::new(RwLock::new(index)),
            dim,
            bit_width,
            persist_path,
            id_to_str: Arc::new(RwLock::new(std::collections::HashMap::new())),
            str_to_id: Arc::new(RwLock::new(std::collections::HashMap::new())),
            next_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        })
    }

    pub fn dimension(&self) -> usize {
        self.dim
    }

    pub fn bit_width(&self) -> usize {
        self.bit_width
    }

    pub fn sync(&self) -> Result<()> {
        if let Some(ref path) = self.persist_path {
            let mut idx = self.index.write();
            idx.sync(path.to_str().unwrap_or_default())
                .map_err(|e| Error::Custom(format!("Failed to sync turbovec index: {e}")))?;
        }
        Ok(())
    }
}

#[cfg(feature = "turbovec")]
impl VectorIndex for TurboVecVectorIndex {
    async fn insert(&self, id: &str, vector: &[f32], payload: serde_json::Value) -> Result<()> {
        if vector.len() != self.dim {
            return Err(Error::Custom(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dim,
                vector.len()
            )));
        }

        let num_id = {
            let mut str_map = self.str_to_id.write();
            let mut id_map = self.id_to_str.write();
            let num = *str_map
                .entry(id.to_string())
                .or_insert_with(|| self.next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
            id_map.insert(num, (id.to_string(), payload));
            num
        };

        let mut idx = self.index.write();
        let _ = idx.remove(num_id);
        idx.add_with_ids(vector, &[num_id])
            .map_err(|e| Error::Custom(format!("Failed to add vector to turbovec: {e}")))?;

        Ok(())
    }

    async fn search(&self, query_vector: &[f32], limit: usize) -> Result<Vec<SearchResult>> {
        if query_vector.len() != self.dim {
            return Err(Error::Custom(format!(
                "Query vector dimension mismatch: expected {}, got {}",
                self.dim,
                query_vector.len()
            )));
        }

        let (scores, found_ids) = {
            let idx = self.index.read();
            idx.search(query_vector, limit)
        };

        let id_map = self.id_to_str.read();
        let mut results = Vec::with_capacity(found_ids.len());

        for (score, num_id) in scores.into_iter().zip(found_ids) {
            if let Some((str_id, payload)) = id_map.get(&num_id) {
                results.push(SearchResult {
                    id: str_id.clone(),
                    score,
                    payload: payload.clone(),
                });
            }
        }

        Ok(results)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let num_id = {
            let mut str_map = self.str_to_id.write();
            let mut id_map = self.id_to_str.write();
            if let Some(num) = str_map.remove(id) {
                id_map.remove(&num);
                Some(num)
            } else {
                None
            }
        };

        if let Some(num) = num_id {
            let mut idx = self.index.write();
            let _ = idx.remove(num);
        }

        Ok(())
    }
}

#[cfg(feature = "turbovec")]
/// High-performance semantic code search engine backed by TurboVec SIMD and SQLite FTS5.
pub struct TurboVecSemanticEngine {
    vector_index: Arc<RwLock<turbovec::IdMapIndex>>,
    metadata_db: Db,
    embedder: Arc<dyn Embedder>,
    dim: usize,
    persist_path: Option<PathBuf>,
}

#[cfg(feature = "turbovec")]
impl TurboVecSemanticEngine {
    pub fn new(
        metadata_db: Db,
        embedder: Arc<dyn Embedder>,
        persist_path: Option<PathBuf>,
    ) -> Result<Self> {
        let dim = embedder.dimension();
        let dim = if dim == 0 { 384 } else { dim }; // Fallback to MiniLM default dimension

        // Initialize schema for code chunks
        {
            let conn = metadata_db.get()?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS code_chunks (
                    id INTEGER PRIMARY KEY,
                    file_path TEXT NOT NULL,
                    symbol TEXT,
                    start_line INTEGER NOT NULL,
                    end_line INTEGER NOT NULL,
                    content TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_code_chunks_file ON code_chunks(file_path);
                CREATE VIRTUAL TABLE IF NOT EXISTS code_chunks_fts USING fts5(
                    content,
                    file_path UNINDEXED,
                    content='code_chunks',
                    content_rowid='id'
                );
                CREATE TRIGGER IF NOT EXISTS code_chunks_ai AFTER INSERT ON code_chunks BEGIN
                    INSERT INTO code_chunks_fts(rowid, content, file_path) VALUES (new.id, new.content, new.file_path);
                END;
                CREATE TRIGGER IF NOT EXISTS code_chunks_ad AFTER DELETE ON code_chunks BEGIN
                    INSERT INTO code_chunks_fts(code_chunks_fts, rowid, content, file_path) VALUES('delete', old.id, old.content, old.file_path);
                END;
                CREATE TRIGGER IF NOT EXISTS code_chunks_au AFTER UPDATE ON code_chunks BEGIN
                    INSERT INTO code_chunks_fts(code_chunks_fts, rowid, content, file_path) VALUES('delete', old.id, old.content, old.file_path);
                    INSERT INTO code_chunks_fts(rowid, content, file_path) VALUES (new.id, new.content, new.file_path);
                END;",
            )?;
        }

        let index = if let Some(ref path) = persist_path {
            if path.exists() {
                turbovec::IdMapIndex::load(path.to_str().unwrap_or_default())
                    .map_err(|e| Error::Custom(format!("Failed to load turbovec index: {e}")))?
            } else {
                turbovec::IdMapIndex::new(dim, 4)
                    .map_err(|e| Error::Custom(format!("Failed to create turbovec index: {e}")))?
            }
        } else {
            turbovec::IdMapIndex::new(dim, 4)
                .map_err(|e| Error::Custom(format!("Failed to create turbovec index: {e}")))?
        };

        Ok(Self {
            vector_index: Arc::new(RwLock::new(index)),
            metadata_db,
            embedder,
            dim,
            persist_path,
        })
    }

    /// Split source code into overlapping line-based semantic chunks.
    fn chunk_content(content: &str, chunk_lines: usize, overlap_lines: usize) -> Vec<(usize, usize, String)> {
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Vec::new();
        }

        let mut chunks = Vec::new();
        let mut start = 0;

        while start < lines.len() {
            let end = (start + chunk_lines).min(lines.len());
            let chunk_text = lines[start..end].join("\n");
            if !chunk_text.trim().is_empty() {
                chunks.push((start + 1, end, chunk_text));
            }
            if end >= lines.len() {
                break;
            }
            start += chunk_lines.saturating_sub(overlap_lines).max(1);
        }

        chunks
    }
}

#[cfg(feature = "turbovec")]
#[async_trait::async_trait]
impl SemanticCodeIndex for TurboVecSemanticEngine {
    async fn index_file(&self, path: &Path, content: &str) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();

        // 1. Remove existing chunks for this file
        self.remove_file(path).await?;

        // 2. Chunk source code
        let chunks = Self::chunk_content(content, 60, 10);
        if chunks.is_empty() {
            return Ok(());
        }

        // 3. Compute embeddings
        let chunk_texts: Vec<&str> = chunks.iter().map(|(_, _, t)| t.as_str()).collect();
        let embeddings = self.embedder.embed_batch(&chunk_texts)?;

        // 4. Save metadata to SQLite and collect new IDs
        let mut inserted_ids = Vec::with_capacity(chunks.len());
        let mut flat_vectors = Vec::with_capacity(chunks.len() * self.dim);

        {
            let conn = self.metadata_db.get()?;
            let mut stmt = conn.prepare(
                "INSERT INTO code_chunks (file_path, symbol, start_line, end_line, content)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;

            for ((start_line, end_line, text), emb) in chunks.into_iter().zip(embeddings) {
                if emb.len() != self.dim {
                    continue;
                }
                let row_id = stmt.insert(rusqlite::params![
                    path_str,
                    Option::<String>::None,
                    start_line as i64,
                    end_line as i64,
                    text
                ])?;

                inserted_ids.push(row_id as u64);
                flat_vectors.extend_from_slice(&emb);
            }
        }

        // 5. Ingest into TurboVec SIMD index
        if !inserted_ids.is_empty() {
            let mut idx = self.vector_index.write();
            idx.add_with_ids(&flat_vectors, &inserted_ids)
                .map_err(|e| Error::Custom(format!("Failed to add chunks to TurboVec: {e}")))?;
        }

        Ok(())
    }

    async fn remove_file(&self, path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy().to_string();
        let ids_to_remove: Vec<u64> = {
            let conn = self.metadata_db.get()?;
            let mut stmt = conn.prepare("SELECT id FROM code_chunks WHERE file_path = ?1")?;
            let rows = stmt.query_map(rusqlite::params![path_str], |row| row.get::<_, i64>(0))?;
            rows.filter_map(|r| r.ok().map(|id| id as u64)).collect()
        };

        if !ids_to_remove.is_empty() {
            {
                let mut idx = self.vector_index.write();
                for &id in &ids_to_remove {
                    let _ = idx.remove(id);
                }
            }

            let conn = self.metadata_db.get()?;
            conn.execute("DELETE FROM code_chunks WHERE file_path = ?1", rusqlite::params![path_str])?;
        }

        Ok(())
    }

    async fn search(&self, query: &str, filter: &SearchFilter, limit: usize) -> Result<Vec<CodeMatch>> {
        let query_emb = self.embedder.embed(query)?;
        if query_emb.len() != self.dim {
            return Ok(Vec::new());
        }

        // Resolve candidate IDs based on filters & BM25
        let candidate_ids: Option<Vec<u64>> = {
            let conn = self.metadata_db.get()?;
            let mut sql = String::from("SELECT DISTINCT c.id FROM code_chunks c");
            let mut joins = String::new();
            let mut where_clauses = Vec::new();

            if filter.hybrid_bm25 && !query.trim().is_empty() {
                // Escape query for FTS5
                let sanitized_query = query
                    .replace('"', "\"\"")
                    .split_whitespace()
                    .map(|w| format!("\"{w}\""))
                    .collect::<Vec<_>>()
                    .join(" OR ");

                if !sanitized_query.is_empty() {
                    joins.push_str(" JOIN code_chunks_fts f ON f.rowid = c.id");
                    where_clauses.push(format!("code_chunks_fts MATCH '{}'", sanitized_query));
                }
            }

            if let Some(ref prefix) = filter.path_prefix {
                where_clauses.push(format!("c.file_path LIKE '{}%'", prefix.replace('\'', "''")));
            }

            if let Some(ref exts) = filter.file_extensions {
                if !exts.is_empty() {
                    let ext_conditions: Vec<String> = exts
                        .iter()
                        .map(|ext| format!("c.file_path LIKE '%.{}'", ext.trim_start_matches('.').replace('\'', "''")))
                        .collect();
                    where_clauses.push(format!("({})", ext_conditions.join(" OR ")));
                }
            }

            if !joins.is_empty() {
                sql.push_str(&joins);
            }
            if !where_clauses.is_empty() {
                sql.push_str(" WHERE ");
                sql.push_str(&where_clauses.join(" AND "));
            }

            if joins.is_empty() && where_clauses.is_empty() {
                None
            } else {
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
                let ids: Vec<u64> = rows.filter_map(|r| r.ok().map(|id| id as u64)).collect();
                Some(ids)
            }
        };

        // Query TurboVec index with or without allowlist
        let (scores, found_ids) = {
            let idx = self.vector_index.read();
            if let Some(ref allowlist) = candidate_ids {
                if allowlist.is_empty() {
                    return Ok(Vec::new());
                }
                idx.search_with_allowlist(&query_emb, limit, Some(allowlist.as_slice()))
                    .map_err(|e| Error::Custom(format!("TurboVec allowlist search error: {e}")))?
            } else {
                idx.search(&query_emb, limit)
            }
        };

        if found_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Fetch chunk metadata for retrieved IDs
        let conn = self.metadata_db.get()?;
        let mut matches = Vec::with_capacity(found_ids.len());

        let mut stmt = conn.prepare(
            "SELECT file_path, symbol, start_line, end_line, content FROM code_chunks WHERE id = ?1",
        )?;

        for (score, id) in scores.into_iter().zip(found_ids) {
            let chunk_opt = stmt
                .query_row(rusqlite::params![id as i64], |row| {
                    Ok(CodeMatch {
                        file_path: row.get(0)?,
                        symbol: row.get(1)?,
                        line_range: (row.get::<_, i64>(2)? as usize, row.get::<_, i64>(3)? as usize),
                        content: row.get(4)?,
                        score,
                    })
                })
                .ok();

            if let Some(m) = chunk_opt {
                matches.push(m);
            }
        }

        Ok(matches)
    }

    async fn flush(&self) -> Result<()> {
        if let Some(ref path) = self.persist_path {
            let mut idx = self.vector_index.write();
            idx.sync(path.to_str().unwrap_or_default())
                .map_err(|e| Error::Custom(format!("Failed to sync TurboVec index: {e}")))?;
        }
        Ok(())
    }
}

// -- Non-feature-gated stubs when semantic-search is disabled --

#[cfg(not(feature = "turbovec"))]
pub struct TurboVecVectorIndex;

#[cfg(not(feature = "turbovec"))]
impl TurboVecVectorIndex {
    pub fn new(_dim: usize, _bit_width: usize, _persist_path: Option<PathBuf>) -> Result<Self> {
        Ok(Self)
    }
}

#[cfg(not(feature = "turbovec"))]
impl VectorIndex for TurboVecVectorIndex {
    async fn insert(&self, _id: &str, _vector: &[f32], _payload: serde_json::Value) -> Result<()> {
        Ok(())
    }
    async fn search(&self, _query_vector: &[f32], _limit: usize) -> Result<Vec<SearchResult>> {
        Ok(vec![])
    }
    async fn delete(&self, _id: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(not(feature = "turbovec"))]
pub struct TurboVecSemanticEngine;

#[cfg(not(feature = "turbovec"))]
impl TurboVecSemanticEngine {
    pub fn new(
        _metadata_db: Db,
        _embedder: Arc<dyn Embedder>,
        _persist_path: Option<PathBuf>,
    ) -> Result<Self> {
        Ok(Self)
    }
}

#[cfg(not(feature = "turbovec"))]
#[async_trait::async_trait]
impl SemanticCodeIndex for TurboVecSemanticEngine {
    async fn index_file(&self, _path: &Path, _content: &str) -> Result<()> {
        Ok(())
    }
    async fn remove_file(&self, _path: &Path) -> Result<()> {
        Ok(())
    }
    async fn search(&self, _query: &str, _filter: &SearchFilter, _limit: usize) -> Result<Vec<CodeMatch>> {
        Ok(vec![])
    }
    async fn flush(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(all(test, feature = "turbovec"))]
mod tests {
    use super::*;
    use tempfile::tempdir;

    struct DeterministicEmbedder {
        dim: usize,
    }

    impl Embedder for DeterministicEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            let mut vec = vec![0.0f32; self.dim];
            let bytes = text.as_bytes();
            for (i, &b) in bytes.iter().enumerate() {
                vec[i % self.dim] += (b as f32) / 255.0;
            }
            // Normalize
            let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vec {
                    *v /= norm;
                }
            }
            Ok(vec)
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    #[tokio::test]
    async fn test_turbovec_vector_index_crud() {
        let index = TurboVecVectorIndex::new(8, 4, None).unwrap();
        let vec_a = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let vec_b = vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

        index.insert("doc_a", &vec_a, serde_json::json!({"tag": "a"})).await.unwrap();
        index.insert("doc_b", &vec_b, serde_json::json!({"tag": "b"})).await.unwrap();

        let results = index.search(&vec_a, 2).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].id, "doc_a");

        index.delete("doc_a").await.unwrap();
        let results_after = index.search(&vec_a, 2).await.unwrap();
        assert!(results_after.iter().all(|r| r.id != "doc_a"));
    }

    #[tokio::test]
    async fn test_turbovec_semantic_engine_slice1_ingest_and_search() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("metadata.db");
        let db = crate::sqlite::open(db_path.to_str().unwrap()).unwrap();
        let embedder = Arc::new(DeterministicEmbedder { dim: 16 });

        let engine = TurboVecSemanticEngine::new(db, embedder, None).unwrap();

        let file_path = Path::new("src/auth.rs");
        let code = "pub fn authenticate_user(token: &str) -> bool {\n    token == \"secret\"\n}\n";
        engine.index_file(file_path, code).await.unwrap();

        let filter = SearchFilter::default();
        let matches = engine.search("authenticate user token", &filter, 5).await.unwrap();
        assert!(!matches.is_empty());
        assert_eq!(matches[0].file_path, "src/auth.rs");
        assert!(matches[0].content.contains("authenticate_user"));
    }

    #[tokio::test]
    async fn test_turbovec_semantic_engine_slice2_hybrid_allowlist_filtering() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("metadata.db");
        let db = crate::sqlite::open(db_path.to_str().unwrap()).unwrap();
        let embedder = Arc::new(DeterministicEmbedder { dim: 16 });

        let engine = TurboVecSemanticEngine::new(db, embedder, None).unwrap();

        engine.index_file(Path::new("src/auth.rs"), "fn login() {}").await.unwrap();
        engine.index_file(Path::new("src/db.rs"), "fn connect() {}").await.unwrap();
        engine.index_file(Path::new("docs/readme.md"), "documentation file").await.unwrap();

        // 1. Path extension filter
        let filter = SearchFilter {
            path_prefix: None,
            file_extensions: Some(vec!["rs".to_string()]),
            hybrid_bm25: false,
        };
        let matches = engine.search("login", &filter, 5).await.unwrap();
        assert!(matches.iter().all(|m| m.file_path.ends_with(".rs")));

        // 2. Hybrid BM25 keyword matching
        let filter_bm25 = SearchFilter {
            path_prefix: None,
            file_extensions: None,
            hybrid_bm25: true,
        };
        let matches_bm25 = engine.search("connect", &filter_bm25, 5).await.unwrap();
        assert!(!matches_bm25.is_empty());
        assert_eq!(matches_bm25[0].file_path, "src/db.rs");
    }

    #[tokio::test]
    async fn test_turbovec_semantic_engine_slice3_persistence_and_sync() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("metadata.db");
        let tv_path = dir.path().join("index.tvim");

        let db = crate::sqlite::open(db_path.to_str().unwrap()).unwrap();
        let embedder = Arc::new(DeterministicEmbedder { dim: 16 });

        {
            let engine = TurboVecSemanticEngine::new(db.clone(), embedder.clone(), Some(tv_path.clone())).unwrap();
            engine.index_file(Path::new("src/main.rs"), "fn main() { println!(\"hello\"); }").await.unwrap();
            engine.flush().await.unwrap();
        }

        // Reload engine from disk
        {
            let engine_reloaded = TurboVecSemanticEngine::new(db, embedder, Some(tv_path)).unwrap();
            let matches = engine_reloaded.search("main println", &SearchFilter::default(), 5).await.unwrap();
            assert!(!matches.is_empty());
            assert_eq!(matches[0].file_path, "src/main.rs");

            // Test removal
            engine_reloaded.remove_file(Path::new("src/main.rs")).await.unwrap();
            let matches_after = engine_reloaded.search("main println", &SearchFilter::default(), 5).await.unwrap();
            assert!(matches_after.is_empty());
        }
    }
}
