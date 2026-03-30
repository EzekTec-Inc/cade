// region:    --- Modules

pub mod api;
pub mod config;
pub mod consolidation;
pub mod crypto;
pub mod error;
pub mod rate_limit;
pub mod reflection;
pub mod state;
pub mod storage;

pub use error::{Error, Result};

// -- Optional re-exports for feature-gated crates
#[cfg(feature = "reranker")]
pub use cade_reranker;

// endregion: --- Modules
