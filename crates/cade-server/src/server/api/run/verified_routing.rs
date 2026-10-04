//! Verified dynamic model routing adapter and streaming probe.
//!
//! Provides proactive health verification for dynamic model switching:
//! 1. Probes candidate economy models before switching to ensure they work without error.
//! 2. If a candidate fails, automatically cycles to another candidate model and probes again.
//! 3. If up to three (3) candidate models fail, automatically falls back to the user's
//!    original session frontier model.

use cade_ai::model_routing::ModelRoutingDecision;
use cade_ai::{CompletionRequest, LlmMessage, StreamChunk};
use futures::StreamExt;
use std::time::Duration;

pub const MAX_VERIFIED_CANDIDATE_ATTEMPTS: usize = 3;
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

/// Outcome of attempting to route through candidate economy models with active stream verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifiedRouteOutcome {
    /// A candidate model passed stream verification and is safe to use.
    VerifiedCandidate {
        model: String,
        attempts: usize,
        discarded_failures: Vec<(String, String)>,
    },
    /// All candidate attempts failed; restored the user's session frontier model.
    FallbackToFrontier {
        frontier_model: String,
        attempts: usize,
        failures: Vec<(String, String)>,
        reason: String,
    },
}

impl VerifiedRouteOutcome {
    #[allow(dead_code)]
    pub fn effective_model(&self) -> &str {
        match self {
            Self::VerifiedCandidate { model, .. } => model,
            Self::FallbackToFrontier { frontier_model, .. } => frontier_model,
        }
    }
}

/// A lightweight probe request used to verify stream readiness.
pub fn build_probe_request(model: &str) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages: vec![LlmMessage {
            role: "user".to_string(),
            content: "ping".to_string(),
            tool_call_id: None,
            tool_calls: None,
            images: None,
            cache_control: None,
        }],
        tools: Vec::new(),
        max_tokens: 5,
        reasoning_effort: None,
    }
}

/// Proactively probe candidate models by attempting to open and consume at least one chunk
/// from their actual response stream.
///
/// Cycles through up to 3 candidate models on failure. If all candidate attempts fail,
/// returns `FallbackToFrontier` specifying the original session frontier model.
pub async fn resolve_and_verify_candidate_model<F, Fut>(
    decision: &ModelRoutingDecision,
    candidate_models: &[String],
    mut stream_fn: F,
) -> VerifiedRouteOutcome
where
    F: FnMut(CompletionRequest) -> Fut,
    Fut: std::future::Future<Output = cade_ai::Result<std::pin::Pin<Box<dyn futures::Stream<Item = cade_ai::Result<StreamChunk>> + Send>>>>,
{
    let mut failures = Vec::new();
    let mut attempts = 0;

    for candidate in candidate_models.iter().take(MAX_VERIFIED_CANDIDATE_ATTEMPTS) {
        attempts += 1;
        let probe_req = build_probe_request(candidate);

        tracing::info!(
            "Dynamic model routing: probing candidate economy model #{attempts}: {candidate}"
        );

        let probe_fut = async {
            let mut stream = stream_fn(probe_req).await?;
            // Read at least one chunk to verify that headers/auth/dispatch succeeded
            // and the upstream API did not reject during initial streaming negotiation.
            let chunk_opt = stream.next().await;
            match chunk_opt {
                Some(Ok(StreamChunk::Text(_)))
                | Some(Ok(StreamChunk::Reasoning(_)))
                | Some(Ok(StreamChunk::ToolCall(_)))
                | Some(Ok(StreamChunk::Usage(_)))
                | Some(Ok(StreamChunk::FinishReason(_)))
                | Some(Ok(StreamChunk::Done))
                | None => Ok(()),
                Some(Err(e)) => Err(e),
            }
        };

        match tokio::time::timeout(PROBE_TIMEOUT, probe_fut).await {
            Ok(Ok(())) => {
                tracing::info!(
                    "Dynamic model routing: verified candidate {candidate} is working without errors"
                );
                return VerifiedRouteOutcome::VerifiedCandidate {
                    model: candidate.clone(),
                    attempts,
                    discarded_failures: failures,
                };
            }
            Ok(Err(err)) => {
                let err_msg = err.to_string();
                tracing::warn!(
                    "Dynamic model routing: candidate #{attempts} ({candidate}) returned error: {err_msg}; cycling to next model"
                );
                failures.push((candidate.clone(), err_msg));
            }
            Err(_) => {
                let err_msg = format!("Probe stream timed out after {:?}", PROBE_TIMEOUT);
                tracing::warn!(
                    "Dynamic model routing: candidate #{attempts} ({candidate}) timed out; cycling to next model"
                );
                failures.push((candidate.clone(), err_msg));
            }
        }
    }

    let reason = format!(
        "Dynamic model routing: all {attempts} candidate economy models failed verification. Restoring session frontier model {}.",
        decision.original_model
    );
    tracing::warn!("{reason}");

    VerifiedRouteOutcome::FallbackToFrontier {
        frontier_model: decision.original_model.clone(),
        attempts,
        failures,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cade_ai::Result;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_verify_selects_first_healthy_candidate() {
        let decision = ModelRoutingDecision {
            effective_model: "anthropic/claude-3-5-haiku-latest".into(),
            original_model: "anthropic/claude-3-7-sonnet".into(),
            category: "recon_read".into(),
            confidence: 0.9,
            is_downgraded_for_economy: true,
            reason: "recon economy".into(),
        };
        let candidates = vec!["anthropic/claude-3-5-haiku-latest".into()];

        let outcome = resolve_and_verify_candidate_model(&decision, &candidates, |_req| {
            Box::pin(async {
                let stream = futures::stream::iter(vec![Ok(StreamChunk::Text("ok".into()))]);
                Ok(Box::pin(stream) as std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>)
            })
        })
        .await;

        match outcome {
            VerifiedRouteOutcome::VerifiedCandidate { model, attempts, discarded_failures } => {
                assert_eq!(model, "anthropic/claude-3-5-haiku-latest");
                assert_eq!(attempts, 1);
                assert!(discarded_failures.is_empty());
            }
            _ => panic!("Expected verified candidate"),
        }
    }

    #[tokio::test]
    async fn test_verify_cycles_past_failing_candidate_to_working_model() {
        let decision = ModelRoutingDecision {
            effective_model: "failing/bad-model-1".into(),
            original_model: "frontier/session-model".into(),
            category: "recon_read".into(),
            confidence: 0.95,
            is_downgraded_for_economy: true,
            reason: "test".into(),
        };
        let candidates = vec![
            "failing/bad-model-1".into(),
            "working/good-model-2".into(),
        ];

        let counter = Arc::new(AtomicUsize::new(0));
        let outcome = resolve_and_verify_candidate_model(&decision, &candidates, {
            let counter = counter.clone();
            move |req| {
                let call_idx = counter.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    if call_idx == 0 {
                        // Simulates error in stream chunk
                        let stream = futures::stream::iter(vec![Err(cade_ai::Error::custom("Missing Gemini API key"))]);
                        Ok(Box::pin(stream) as std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>)
                    } else {
                        assert_eq!(req.model, "working/good-model-2");
                        let stream = futures::stream::iter(vec![Ok(StreamChunk::Text("pong".into()))]);
                        Ok(Box::pin(stream) as std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk>> + Send>>)
                    }
                })
            }
        })
        .await;

        match outcome {
            VerifiedRouteOutcome::VerifiedCandidate { model, attempts, discarded_failures } => {
                assert_eq!(model, "working/good-model-2");
                assert_eq!(attempts, 2);
                assert_eq!(discarded_failures.len(), 1);
                assert_eq!(discarded_failures[0].0, "failing/bad-model-1");
                assert!(discarded_failures[0].1.contains("Missing Gemini API key"));
            }
            _ => panic!("Expected verified second candidate"),
        }
    }

    #[tokio::test]
    async fn test_verify_falls_back_to_session_frontier_after_three_failures() {
        let decision = ModelRoutingDecision {
            effective_model: "failing/bad-1".into(),
            original_model: "anthropic/claude-3-7-sonnet".into(),
            category: "recon_read".into(),
            confidence: 0.95,
            is_downgraded_for_economy: true,
            reason: "recon".into(),
        };
        let candidates = vec![
            "failing/bad-1".into(),
            "failing/bad-2".into(),
            "failing/bad-3".into(),
            "should-not-reach-4".into(),
        ];

        let attempts_seen = Arc::new(AtomicUsize::new(0));
        let outcome = resolve_and_verify_candidate_model(&decision, &candidates, {
            let attempts_seen = attempts_seen.clone();
            move |_req| {
                attempts_seen.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Err(cade_ai::Error::custom("APIConnectionError"))
                })
            }
        })
        .await;

        assert_eq!(attempts_seen.load(Ordering::SeqCst), 3);
        match outcome {
            VerifiedRouteOutcome::FallbackToFrontier { frontier_model, attempts, failures, reason } => {
                assert_eq!(frontier_model, "anthropic/claude-3-7-sonnet");
                assert_eq!(attempts, 3);
                assert_eq!(failures.len(), 3);
                assert!(reason.contains("all 3 candidate economy models failed verification"));
            }
            _ => panic!("Expected fallback to session frontier model"),
        }
    }
}
