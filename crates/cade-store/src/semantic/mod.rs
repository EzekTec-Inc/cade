//! Deep semantic code indexing module.
//!
//! Provides the [`SemanticCodeIndex`] trait for codebase semantic search,
//! along with the high-performance [`TurboVecEngine`] adapter leveraging
//! Google TurboQuant 4-bit quantization and SIMD allowlists.

pub mod turbovec_engine;

pub use turbovec_engine::{TurboVecSemanticEngine, TurboVecVectorIndex};

use std::path::Path;
use crate::error::Result;

/// A ranked match returned from semantic codebase search.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeMatch {
    pub file_path: String,
    pub symbol: Option<String>,
    pub line_range: (usize, usize),
    pub content: String,
    pub score: f32,
}

/// Search constraints for scoping queries.
#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub path_prefix: Option<String>,
    pub file_extensions: Option<Vec<String>>,
    pub hybrid_bm25: bool,
}

/// Deep semantic index module interface.
/// Encapsulates chunking, embeddings, quantization, and SIMD retrieval.
#[async_trait::async_trait]
pub trait SemanticCodeIndex: Send + Sync {
    /// Ingest or update a source file into the index.
    async fn index_file(&self, path: &Path, content: &str) -> Result<()>;

    /// Remove a deleted file from the index.
    async fn remove_file(&self, path: &Path) -> Result<()>;

    /// Search for code semantically matching the natural language query.
    async fn search(&self, query: &str, filter: &SearchFilter, limit: usize) -> Result<Vec<CodeMatch>>;

    /// Persist pending incremental changes safely to disk.
    async fn flush(&self) -> Result<()>;
}
