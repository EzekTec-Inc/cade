//! Rig model adapter implementing CADE LlmProvider.
#![cfg(feature = "rig-compat")]

use crate::{LlmProvider, CompletionRequest, CompletionResponse, StreamChunk, Result};
use async_trait::async_trait;
#[allow(unused_imports)]
use rig::completion::{CompletionModel, Prompt};
use std::pin::Pin;
use tokio_stream::Stream;

// region:    --- Types

/// Adapter that wraps any `rig` completion model as a CADE `LlmProvider`
pub struct RigProviderAdapter<M: CompletionModel> {
    pub model: M,
}

// endregion: --- Types

// region:    --- Implementations

#[async_trait]
impl<M: CompletionModel + rig::completion::Prompt + Send + Sync> LlmProvider for RigProviderAdapter<M> {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        // Map CADE messages to a flat prompt string for rig-core basic completion
        let prompt = req.messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let response = self.model.prompt(&prompt)
            .await
            .map_err(|e| crate::Error::custom(format!("Rig Model Error: {e}")))?;

        Ok(CompletionResponse {
            content: Some(response),
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        Err(crate::Error::custom("Streaming is not yet implemented for the Rig Adapter"))
    }
}

// endregion: --- Implementations
