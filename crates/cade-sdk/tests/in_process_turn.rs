//! Integration tests for in-process CADE SDK transport (ADR-0020 & ADR-0021).
//!
//! Verifies that third-party applications importing `cade-sdk` can configure
//! and run an autonomous `EmbeddedSession` linking directly to an in-memory SQLite
//! database and custom `LlmProvider` without spawning any background HTTP daemon.

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use tokio_stream::wrappers::ReceiverStream;

use cade_ai::types::{CompletionRequest, CompletionResponse, LlmProvider, StreamChunk, TokenUsage};
use cade_sdk::events::CadeStreamEvent;
use cade_sdk::{EmbeddedSession, Result};

// region:    --- Mock LLM Provider

struct MockStreamingProvider {
    response_text: String,
}

impl MockStreamingProvider {
    fn new(response_text: impl Into<String>) -> Self {
        Self {
            response_text: response_text.into(),
        }
    }
}

#[async_trait]
impl LlmProvider for MockStreamingProvider {
    async fn complete(&self, _req: &CompletionRequest) -> cade_ai::Result<CompletionResponse> {
        Ok(CompletionResponse {
            content: Some(self.response_text.clone()),
            tool_calls: vec![],
            finish_reason: "stop".to_string(),
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> cade_ai::Result<Pin<Box<dyn Stream<Item = cade_ai::Result<StreamChunk>> + Send>>> {
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let text = self.response_text.clone();

        tokio::spawn(async move {
            // Stream in two text chunks
            let half = text.len() / 2;
            let (part1, part2) = text.split_at(half);
            let _ = tx.send(Ok(StreamChunk::Text(part1.to_string()))).await;
            let _ = tx.send(Ok(StreamChunk::Text(part2.to_string()))).await;
            let _ = tx
                .send(Ok(StreamChunk::Usage(TokenUsage {
                    input_tokens: 15,
                    output_tokens: 10,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    model: "mock-model".to_string(),
                })))
                .await;
            let _ = tx.send(Ok(StreamChunk::Done)).await;
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

// endregion: --- Mock LLM Provider

// region:    --- Tests

#[tokio::test]
async fn test_in_process_session_builder_and_direct_prompt() -> Result<()> {
    // -- Setup & Fixtures
    let mock_provider = Arc::new(MockStreamingProvider::new(
        "In-process CADE autonomous execution completed.",
    ));

    let session = EmbeddedSession::builder()
        .in_memory()
        .agent_name("TestWorker")
        .model("mock/test-worker-v1")
        .system_prompt("You are a helpful in-process test assistant.")
        .provider(mock_provider)
        .build()
        .await?;

    // -- Exec
    let response = session.prompt("Run diagnostic turn in-process").await?;

    // -- Check
    assert_eq!(
        response, "In-process CADE autonomous execution completed.",
        "in-process prompt must return the fully accumulated mock assistant response"
    );
    assert_eq!(session.model(), "mock/test-worker-v1");
    assert!(session.agent_id().starts_with("emb-"));
    Ok(())
}

#[tokio::test]
async fn test_in_process_session_streaming_telemetry() -> Result<()> {
    // -- Setup & Fixtures
    let expected = "Real-time stream delta test.";
    let mock_provider = Arc::new(MockStreamingProvider::new(expected));

    let session = EmbeddedSession::builder()
        .in_memory()
        .agent_name("StreamAgent")
        .model("mock/stream-v1")
        .provider(mock_provider)
        .build()
        .await?;

    // -- Exec
    let mut stream = session.stream_prompt("Verify telemetry streaming").await?;
    let mut accumulated = String::new();
    let mut received_deltas = 0;

    while let Some(event) = stream.next().await {
        if let CadeStreamEvent::MessageDelta(delta) = event {
            accumulated.push_str(&delta);
            received_deltas += 1;
        }
    }

    // -- Check
    assert_eq!(
        accumulated, expected,
        "streamed deltas must assemble into the complete response"
    );
    assert!(
        received_deltas >= 2,
        "expected multiple message deltas from streaming provider"
    );
    Ok(())
}

#[tokio::test]
async fn test_in_process_session_memory_blocks() -> Result<()> {
    // -- Setup & Fixtures
    let mock_provider = Arc::new(MockStreamingProvider::new("Memory test agent ready."));

    let session = EmbeddedSession::builder()
        .in_memory()
        .agent_name("MemoryAgent")
        .provider(mock_provider)
        .build()
        .await?;

    // -- Exec: Set memory block
    session
        .set_memory("project_convention", "Always write tests first (TDD).")
        .await?;

    // -- Check: Read existing memory block
    let value = session.get_memory("project_convention").await?;
    assert_eq!(
        value,
        Some("Always write tests first (TDD).".to_string()),
        "stored memory block must be readable in-process from SQLite"
    );

    // -- Check: Non-existent memory block returns None
    let missing = session.get_memory("nonexistent_block").await?;
    assert_eq!(missing, None);
    Ok(())
}

// endregion: --- Tests
