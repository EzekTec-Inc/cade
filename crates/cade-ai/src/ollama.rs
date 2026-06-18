/// Ollama provider — uses OpenAI-compatible `/v1/chat/completions` endpoint
/// Available since Ollama 0.1.24.
use super::openai::OpenAiProvider;
use super::{CompletionRequest, CompletionResponse, LlmProvider, StreamChunk};
use crate::Result;
use async_trait::async_trait;
use std::pin::Pin;
use tokio_stream::Stream;

pub struct OllamaProvider {
    inner: OpenAiProvider,
    base_url: String,
}

impl OllamaProvider {
    pub fn new(base_url: String) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        let url = format!("{base_url}/v1/chat/completions");
        // Ollama doesn't require an API key — use a placeholder
        Self {
            inner: OpenAiProvider::new("ollama".to_string(), Some(url)),
            base_url,
        }
    }

    /// Query Ollama's `/api/tags` endpoint and return installed model names.
    /// Returns an empty Vec if Ollama is unreachable or returns no models.
    pub async fn list_models(&self) -> Vec<String> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = match reqwest::get(&url).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Ollama list_models: request failed for {url}: {e}");
                return vec![];
            }
        };
        let body = match resp.json::<serde_json::Value>().await {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    "Ollama list_models: failed to parse JSON response body for {url}: {e}"
                );
                return vec![];
            }
        };
        body["models"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
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
