/// Ollama provider — uses OpenAI-compatible `/v1/chat/completions` endpoint
/// Available since Ollama 0.1.24.
use super::openai::OpenAiProvider;
use super::{CompletionRequest, CompletionResponse, LlmProvider, StreamChunk};
use anyhow::Result;
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

pub struct OllamaProvider {
    inner: OpenAiProvider,
}

impl OllamaProvider {
    pub fn new(base_url: String) -> Self {
        let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));
        // Ollama doesn't require an API key — use a placeholder
        Self { inner: OpenAiProvider::new("ollama".to_string(), Some(url)) }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse> {
        self.inner.complete(req).await
    }

    async fn stream(
        &self,
        req: &CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        self.inner.stream(req).await
    }
}
