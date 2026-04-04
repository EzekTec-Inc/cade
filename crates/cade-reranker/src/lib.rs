// region:    --- Modules

pub mod config;
mod error;
#[cfg(feature = "local")]
pub mod model;
pub mod reranker;

// endregion: --- Modules

// region:    --- Re-exports

pub use config::{RerankerBackend, RerankerConfig, config_from_env, default_protected_tools};
pub use error::{Error, Result};
pub use reranker::{RerankResult, ToolDocument, ToolReranker};

// endregion: --- Re-exports
