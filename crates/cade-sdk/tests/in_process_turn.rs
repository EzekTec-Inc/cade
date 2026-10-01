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

#[tokio::test]
async fn embedded_observation_rejects_failed_terminal_publication() -> Result<()> {
    let session = EmbeddedSession::builder()
        .in_memory()
        .provider(Arc::new(MockStreamingProvider::new("partial output")))
        .build()
        .await?;
    // Terminal publication and status are atomic. Failure to persist success
    // must roll back success and publish an error outcome through recovery.
    session
        .db()
        .get()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER drop_success BEFORE INSERT ON run_events
         WHEN json_extract(NEW.data, '$.message_type') = 'run_done'
          AND json_extract(NEW.data, '$.status') = 'done'
         BEGIN SELECT RAISE(FAIL, 'injected terminal publication failure'); END;",
        )
        .unwrap();
    let events: Vec<_> = session
        .stream_prompt("observe status")
        .await?
        .collect()
        .await;
    assert!(
        events.iter().any(|event| matches!(event,
        CadeStreamEvent::Finished { outcome } if outcome == "error")),
        "{events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(event,
        CadeStreamEvent::Finished { outcome } if outcome == "done")),
        "{events:?}"
    );
    let response = session.prompt("observe through prompt").await;
    assert!(response.is_err(), "{response:?}");
    Ok(())
}

#[tokio::test]
async fn embedded_prompt_cannot_succeed_without_terminal_evidence() -> Result<()> {
    let session = EmbeddedSession::builder()
        .in_memory()
        .provider(Arc::new(MockStreamingProvider::new("partial output")))
        .build()
        .await?;
    session
        .db()
        .get()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER drop_terminal BEFORE INSERT ON run_events
         WHEN json_extract(NEW.data, '$.message_type') = 'run_done'
         BEGIN SELECT RAISE(FAIL, 'injected terminal publication failure'); END;
         CREATE TRIGGER drop_finish BEFORE UPDATE OF status ON runs
         BEGIN SELECT RAISE(FAIL, 'injected status failure'); END;",
        )
        .unwrap();
    let result = session.prompt("observe incomplete Run").await;
    assert!(
        result.is_err(),
        "partial output is not completion: {result:?}"
    );
    let events: Vec<_> = session
        .stream_prompt("observe failed finalization")
        .await?
        .collect()
        .await;
    let errors: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            CadeStreamEvent::Error(error) => Some(error),
            _ => None,
        })
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "finalization diagnostic must end observation once: {events:?}"
    );
    assert!(errors[0].contains("incomplete"), "{errors:?}");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, CadeStreamEvent::Finished { .. }))
    );
    Ok(())
}

#[tokio::test]
async fn remote_sdk_reports_failed_finalization_instead_of_following_stale_running_status() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request.split_whitespace().nth(1).unwrap();
            let (kind, body) = if path.ends_with("/run") {
                (
                    "text/event-stream",
                    "data: {\"message_type\":\"error\",\"run_id\":\"r-sdk\",\"code\":\"run_finalization_failed\",\"terminal_status_persisted\":false,\"error\":\"disk full\"}\n\ndata: [DONE]\n\n",
                )
            } else if path.contains("/stream") {
                ("text/event-stream", "data: [DONE]\n\n")
            } else {
                ("application/json", "{\"status\":\"running\"}")
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let session = cade_sdk::AgentSession::create(cade_sdk::SessionOptions {
        server_url: format!("http://{address}"),
        agent_id: Some("a-sdk".into()),
        ..Default::default()
    })
    .await
    .unwrap();
    for streaming in [false, true] {
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            if streaming {
                session.prompt_stream("hello", |_| {}).await
            } else {
                session.prompt("hello").await
            }
        })
        .await
        .expect("SDK must end observation on explicit persistence failure");
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("incomplete") && error.contains("r-sdk") && error.contains("disk full"),
            "{error}"
        );
    }
    peer.abort();
}

#[tokio::test]
async fn http_tail_recovery_and_embedded_observation_have_equivalent_events() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let session = EmbeddedSession::builder()
        .in_memory()
        .provider(Arc::new(MockStreamingProvider::new("same execution trace")))
        .build()
        .await?;
    let embedded: Vec<_> = session
        .stream_prompt("compare adapters")
        .await?
        .collect()
        .await;
    let run = cade_store::sqlite::list_agent_runs(session.db(), session.agent_id(), 1)
        .unwrap()
        .remove(0);
    let rows = cade_store::sqlite::run_events_after(session.db(), &run.id, -1).unwrap();
    let frames: Vec<_> = rows
        .into_iter()
        .map(|(seq, data)| {
            let mut value: serde_json::Value = serde_json::from_str(&data).unwrap();
            value["run_id"] = run.id.clone().into();
            value["seq_id"] = seq.into();
            format!("data: {value}\n\n")
        })
        .collect();
    let live = frames[..2].concat();
    let replay = frames.concat();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        for body in [live, replay] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let _ = socket.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let client =
        cade_agent::agent::client::HttpTransport::new(format!("http://{address}"), String::new())
            .unwrap();
    let messages = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        client.start_run("agent", "compare adapters", None, |_| {}),
    )
    .await
    .unwrap()
    .unwrap();
    let remote: Vec<_> = messages
        .iter()
        .filter_map(|message| {
            let event = serde_json::from_value(serde_json::to_value(message).unwrap()).unwrap();
            CadeStreamEvent::from_stream_event(&event)
        })
        .collect();
    assert_eq!(remote, embedded);
    peer.await.unwrap();
    Ok(())
}
