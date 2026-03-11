//! Token-bucket rate limiter for the CADE server.
//!
//! One bucket per agent ID. Configured via env vars:
//!   CADE_RATE_LIMIT_RPM   — max requests per minute (default 60)
//!   CADE_RATE_LIMIT_BURST — burst capacity in tokens  (default 10)
//!
//! Only inference endpoints are throttled:
//!   POST /v1/agents/:id/messages
//!   POST /v1/agents/:id/messages/stream
//!
//! Returns HTTP 429 with a Retry-After header when a bucket is exhausted.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::server::state::AppState;

// ── Token bucket ──────────────────────────────────────────────────────────────

#[derive(Debug)]
struct Bucket {
    tokens:      f64,
    capacity:    f64,
    refill_rate: f64,   // tokens per second
    last_refill: Instant,
}

impl Bucket {
    fn new(capacity: f64, rpm: f64) -> Self {
        Self {
            tokens:      capacity,
            capacity,
            refill_rate: rpm / 60.0,
            last_refill: Instant::now(),
        }
    }

    /// Consume one token. Returns `Ok(())` on success or `Err(retry_secs)` when limited.
    fn try_consume(&mut self) -> Result<(), u64> {
        let now     = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens      = (self.tokens + elapsed * self.refill_rate).min(self.capacity);
        self.last_refill = now;

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            let wait = ((1.0 - self.tokens) / self.refill_rate).ceil() as u64;
            Err(wait.max(1))
        }
    }
}

// ── Shared limiter ────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct RateLimiter {
    buckets:  Arc<Mutex<HashMap<String, Bucket>>>,
    capacity: f64,
    rpm:      f64,
}

impl RateLimiter {
    /// Construct from env vars (falls back to sensible defaults).
    /// CADE_RATE_LIMIT_RPM   — requests per minute per agent (default: 60)
    /// CADE_RATE_LIMIT_BURST — burst size in tokens            (default: 10)
    pub fn from_env() -> Self {
        let rpm: f64 = std::env::var("CADE_RATE_LIMIT_RPM")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(60.0);
        let burst: f64 = std::env::var("CADE_RATE_LIMIT_BURST")
            .ok().and_then(|v| v.parse().ok()).unwrap_or(10.0);
        // Bucket capacity = burst tokens (minimum 1)
        let capacity = burst.max(1.0);
        tracing::info!(
            "Rate limiter: {rpm} req/min per agent, burst={capacity} tokens"
        );
        Self {
            buckets:  Arc::new(Mutex::new(HashMap::new())),
            capacity,
            rpm,
        }
    }

    /// Try to consume one token for `agent_id`.
    pub fn check(&self, agent_id: &str) -> Result<(), u64> {
        let mut map = self.buckets.lock().unwrap();
        let bucket  = map
            .entry(agent_id.to_string())
            .or_insert_with(|| Bucket::new(self.capacity, self.rpm));
        bucket.try_consume()
    }

    /// Current config summary (for /v1/health).
    pub fn config_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "rpm_per_agent": self.rpm,
            "burst_tokens":  self.capacity,
        })
    }
}

// ── Axum middleware ───────────────────────────────────────────────────────────

/// Rate-limit middleware. Throttles POST inference requests per agent.
/// All other routes pass through untouched.
pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let path   = req.uri().path().to_string();
    let method = req.method().clone();

    // Only throttle POST to inference endpoints
    let is_inference = method == axum::http::Method::POST
        && (path.ends_with("/messages") || path.ends_with("/messages/stream"));

    if !is_inference {
        return next.run(req).await;
    }

    // Extract agent_id: /v1/agents/<id>/messages[/stream]
    let agent_id = path
        .trim_start_matches("/v1/agents/")
        .split('/')
        .next()
        .unwrap_or("unknown")
        .to_string();

    match state.rate_limiter.check(&agent_id) {
        Ok(()) => next.run(req).await,
        Err(retry_secs) => {
            tracing::warn!(
                "Rate limit exceeded — agent='{agent_id}' retry_after={retry_secs}s"
            );
            (
                StatusCode::TOO_MANY_REQUESTS,
                [
                    ("Retry-After",  retry_secs.to_string()),
                    ("Content-Type", "application/json".to_string()),
                ],
                format!(
                    r#"{{"detail":"rate limit exceeded","retry_after_secs":{retry_secs},"agent_id":"{agent_id}"}}"#
                ),
            )
                .into_response()
        }
    }
}
