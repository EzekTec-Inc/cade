//! Rig vector store implementation for CADE SQLite.
#![cfg(feature = "rig-compat")]

use crate::sqlite::Db;
use async_trait::async_trait;

// region:    --- Types

/// A rig-compatible vector store wrapper for CADE's local SQLite database
pub struct RigCadeStore {
    pub db: Db,
}

// endregion: --- Types

// region:    --- Implementations

#[async_trait]
impl rig_core::vector_store::VectorStore for RigCadeStore {
    type Error = crate::Error;

    async fn add_documents(&self, documents: &[rig_core::vector_store::Document]) -> Result<(), Self::Error> {
        let conn = self.db.get().map_err(|e| crate::Error::custom(e.to_string()))?;
        for doc in documents {
            // Encode f32 vector to bytes
            let vec_bytes = serde_json::to_vec(&doc.vector)
                .map_err(|e| crate::Error::custom(e.to_string()))?;
            conn.execute(
                "INSERT INTO memory_embeddings (id, embedding, text) VALUES (?, ?, ?)",
                (&doc.id, &vec_bytes, &doc.text),
            ).map_err(|e| crate::Error::custom(e.to_string()))?;
        }
        Ok(())
    }

    async fn search(&self, query: &[f32], limit: usize) -> Result<Vec<rig_core::vector_store::SearchResult>, Self::Error> {
        let conn = self.db.get().map_err(|e| crate::Error::custom(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT id, text FROM memory_embeddings LIMIT ?"
        ).map_err(|e| crate::Error::custom(e.to_string()))?;

        let rows = stmt.query_map((limit,), |row| {
            let id: String = row.get(0)?;
            let text: String = row.get(1)?;
            Ok(rig_core::vector_store::SearchResult {
                id,
                text,
                score: 1.0, // flat mock score for default / rig interface compliance
            })
        }).map_err(|e| crate::Error::custom(e.to_string()))?;

        let mut results = Vec::new();
        for r in rows {
            results.push(r.map_err(|e| crate::Error::custom(e.to_string()))?);
        }
        Ok(results)
    }
}

// endregion: --- Implementations
