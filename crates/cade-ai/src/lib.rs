// region:    --- Modules

mod error;

pub use error::{Error, Result};

pub mod anthropic;
pub mod catalogue;
pub mod gemini;
pub mod observability;
pub mod ollama;
pub mod openai;
pub mod prompt_cache;
pub mod its;
pub mod provider_registry;
pub mod registry;
#[cfg(feature = "rig-compat")]
pub mod rig_adapter;
pub mod router;
pub mod tokenizer;
pub mod types;
pub mod utils;
pub mod vcr;

pub use catalogue::{CATALOGUE, ModelEntry};
pub use registry::{ModelPricing, ModelRegistry, PricingRule};
pub use router::*;
pub use prompt_cache::{PromptCacheManager, resolve_prompt_cache_manager};
pub use its::{TaggedToolSchema, IntelligentToolSelector, AdaptiveToolSelector, PassThroughToolSelector, resolve_tool_selector};
pub use tokenizer::{
    ContextBudgetResult, FALLBACK_CHARS_PER_TOKEN, PromptBudgetManager, chars_for_tokens,
    count_tokens,
};
pub use types::*;
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
