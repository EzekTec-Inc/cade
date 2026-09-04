pub mod crypto;
pub mod error;
pub mod knowledge_engine;
pub mod semantic;
pub mod sqlite;

pub use knowledge_engine::{
    CompactionSummary, FactInput, KnowledgeEngine, RecallOutput, RecalledFact,
};
pub use sqlite::Db;
pub use sqlite::embedding::{
    Embedder, LocalFastEmbedAdapter, NoopEmbedder, SearchResult, VectorIndex,
};
