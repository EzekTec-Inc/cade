//! Rig model adapter implementing CADE LlmProvider.
#![cfg(feature = "rig-compat")]

use crate::{CompletionRequest, CompletionResponse, LlmProvider, Result, StreamChunk};
use async_trait::async_trait;
use rig::completion::{AssistantContent, CompletionModel};
use std::pin::Pin;
use tokio_stream::Stream;

// region:    --- Types

/// Adapter that wraps any `rig` completion model as a CADE `LlmProvider`
pub struct RigProviderAdapter<M: CompletionModel + Clone + Send + Sync> {
    pub model: M,
}

// endregion: --- Types

// region:    --- Implementations

#[async_trait]
impl<M: CompletionModel + Clone + Send + Sync + 'static> LlmProvider
    for RigProviderAdapter<M>
{
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        // Map CADE messages to a flat prompt string for rig-core basic completion
        let prompt = req
            .messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let request = self.model.completion_request(prompt).build();
        let response = self
            .model
            .completion(request)
            .await
            .map_err(|e| crate::Error::custom(format!("Rig Model Error: {e}")))?;

        let mut content = String::new();
        for item in response.choice {
            if let AssistantContent::Text(text) = item {
                content.push_str(&text.text);
            }
        }

        Ok(CompletionResponse {
            content: Some(content),
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
        })
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let res = self.complete(req).await?;
        let content = res.content.unwrap_or_default();

        let s = async_stream::stream! {
            yield Ok(StreamChunk::Text(content));
            yield Ok(StreamChunk::Done);
        };
        Ok(Box::pin(s))
    }
}
