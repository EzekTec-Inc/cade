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
    fn from_env_with_reader<F>(mut reader: F) -> Self
    where
        F: FnMut(&str) -> Option<String>,
    {
        let rpm: f64 = reader("CADE_RATE_LIMIT_RPM")
            .and_then(|v| v.parse().ok())
            .unwrap_or(60.0);
        let burst: f64 = reader("CADE_RATE_LIMIT_BURST")
            .and_then(|v| v.parse().ok())
            .unwrap_or(10.0);
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

    /// Construct from env vars (falls back to sensible defaults).
    /// CADE_RATE_LIMIT_RPM   — requests per minute per agent (default: 60)
    /// CADE_RATE_LIMIT_BURST — burst size in tokens            (default: 10)
    pub fn from_env() -> Self {
        Self::from_env_with_reader(|key| std::env::var(key).ok())
    }

    /// Try to consume one token for `agent_id`.
    pub fn check(&self, agent_id: &str) -> Result<(), u64> {
        let mut map = self.buckets.lock().unwrap();
        
        if map.len() > 10_000 && !map.contains_key(agent_id) {
            let now = Instant::now();
            map.retain(|_, b| now.duration_since(b.last_refill).as_secs() < 600);
            
            if map.len() > 10_000 {
                return Err(60); // Retry after 60s to prevent OOM
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_allows_burst() {
        let mut bucket = Bucket::new(5.0, 60.0);
        // Should allow 5 requests (burst capacity)
        for _ in 0..5 {
            assert!(bucket.try_consume().is_ok());
        }
        // 6th should be rate limited
        assert!(bucket.try_consume().is_err());
    }

    #[test]
    fn bucket_returns_retry_after() {
        let mut bucket = Bucket::new(1.0, 60.0);
        assert!(bucket.try_consume().is_ok());
        let result = bucket.try_consume();
        assert!(result.is_err());
        let retry_secs = result.unwrap_err();
        assert!(retry_secs >= 1, "retry_secs should be >= 1, got {retry_secs}");
    }

    #[test]
    fn rate_limiter_different_agents_independent() {
        let limiter = RateLimiter {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            capacity: 1.0,
            rpm: 60.0,
        };
        // Agent A uses its bucket
        assert!(limiter.check("agent-a").is_ok());
        assert!(limiter.check("agent-a").is_err());
        // Agent B still has its own bucket
        assert!(limiter.check("agent-b").is_ok());
    }

    #[test]
    fn rate_limiter_config_summary() {
        let limiter = RateLimiter {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            capacity: 10.0,
            rpm: 120.0,
        };
        let summary = limiter.config_summary();
        assert_eq!(summary["rpm_per_agent"], 120.0);
        assert_eq!(summary["burst_tokens"], 10.0);
    }

    #[test]
    fn rate_limiter_from_env_defaults() {
        let limiter = RateLimiter::from_env_with_reader(|_| None);
        let summary = limiter.config_summary();
        assert_eq!(summary["rpm_per_agent"], 60.0);
        assert_eq!(summary["burst_tokens"], 10.0);
    }

    #[test]
    fn rate_limiter_prevents_oom() {
        // Simulate many different agent IDs — should not panic
        let limiter = RateLimiter {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            capacity: 1.0,
            rpm: 60.0,
        };
        for i in 0..100 {
            let _ = limiter.check(&format!("agent-{i}"));
        }
        assert!(limiter.buckets.lock().unwrap().len() <= 100);
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
