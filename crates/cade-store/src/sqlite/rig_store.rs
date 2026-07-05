//! Rig vector store index implementation for CADE SQLite.
#![cfg(feature = "rig-compat")]

use crate::sqlite::Db;
use rig::vector_store::VectorStoreError;

// region:    --- Types

/// A rig-compatible vector store wrapper for CADE's local SQLite database
pub struct RigCadeStore {
    pub db: Db,
}

// endregion: --- Types

// region:    --- Implementations

impl rig::vector_store::VectorStoreIndex for RigCadeStore {
    fn top_n<T: for<'a> serde::Deserialize<'a> + Send>(
        &self,
        _query: &str,
        n: usize,
    ) -> impl std::future::Future<Output = Result<Vec<(f64, String, T)>, VectorStoreError>> + Send
    {
        let db = self.db.clone();
        async move {
            let conn = db
                .get()
                .map_err(|e| VectorStoreError::DatastoreError(Box::from(e.to_string())))?;

            let limit = n as i64;
            let mut stmt = conn
                .prepare("SELECT id, text FROM memory_embeddings LIMIT ?")
                .map_err(|e| VectorStoreError::DatastoreError(Box::from(e.to_string())))?;

            let rows = stmt
                .query_map((limit,), |row| {
                    let id: String = row.get(0)?;
                    let text: String = row.get(1)?;
                    Ok((id, text))
                })
                .map_err(|e| VectorStoreError::DatastoreError(Box::from(e.to_string())))?;

            let mut results = Vec::new();
            for r in rows {
                let (id, text) =
                    r.map_err(|e| VectorStoreError::DatastoreError(Box::from(e.to_string())))?;
                let doc: T = serde_json::from_str(&text)
                    .or_else(|_| serde_json::from_value(serde_json::Value::String(text.clone())))
                    .map_err(|e| VectorStoreError::DatastoreError(Box::from(e.to_string())))?;
                results.push((1.0, id, doc));
            }
            Ok(results)
        }
    }

    fn top_n_ids(
        &self,
        _query: &str,
        n: usize,
    ) -> impl std::future::Future<Output = Result<Vec<(f64, String)>, VectorStoreError>> + Send
    {
        let db = self.db.clone();
        async move {
            let conn = db
                .get()
                .map_err(|e| VectorStoreError::DatastoreError(Box::from(e.to_string())))?;

            let limit = n as i64;
            let mut stmt = conn
                .prepare("SELECT id FROM memory_embeddings LIMIT ?")
                .map_err(|e| VectorStoreError::DatastoreError(Box::from(e.to_string())))?;

            let rows = stmt
                .query_map((limit,), |row| {
                    let id: String = row.get(0)?;
                    Ok((1.0, id))
                })
                .map_err(|e| VectorStoreError::DatastoreError(Box::from(e.to_string())))?;

            let mut results = Vec::new();
            for r in rows {
                results.push(
                    r.map_err(|e| VectorStoreError::DatastoreError(Box::from(e.to_string())))?,
                );
            }
            Ok(results)
        }
    }
}

// endregion: --- Implementations
