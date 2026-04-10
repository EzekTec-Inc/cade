// region:    --- Modules

mod error;

pub use error::{Error, Result};

pub mod anthropic;
pub mod catalogue;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod registry;
pub mod provider_registry;
pub mod router;
pub mod utils;
pub mod types;

pub use catalogue::{CATALOGUE, ModelEntry};
pub use registry::{ModelPricing, ModelRegistry};
pub use types::*;
pub use router::*;
pub use utils::*;

use std::sync::Arc;

// endregion: --- Modules

// -- Factory (kept for compatibility)

pub fn make_provider(config: &AiConfig) -> Result<Arc<dyn LlmProvider>> {
    Ok(Arc::new(LlmRouter::build(config)))
}

// region:    --- Tests

#[cfg(test)]
mod tests;

// endregion: --- Tests
