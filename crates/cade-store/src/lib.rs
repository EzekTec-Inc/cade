pub mod crypto;
pub mod error;
pub mod semantic;
pub mod sqlite;

pub use sqlite::Db;
pub use sqlite::embedding::{
    Embedder, LocalFastEmbedAdapter, NoopEmbedder, SearchResult, VectorIndex,
};
