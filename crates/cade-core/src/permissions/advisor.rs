//! Advisory evaluation seam for pre-screening tool execution requests.
//!
//! Provides non-authoritative risk assessment, task-alignment scoring,
//! and metadata enrichment for approval prompts and agent supervision.

use std::collections::HashMap;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Visual tone for an advisory badge/chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgeTone {
    Neutral,
    Success,
    Warning,
    Danger,
}

/// An individual advisory badge rendered in approval dialogs or logged in telemetry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisoryBadge {
    pub label: String,
    pub tone: BadgeTone,
}

impl AdvisoryBadge {
    pub fn new(label: impl Into<String>, tone: BadgeTone) -> Self {
        Self {
            label: label.into(),
            tone,
        }
    }
}

/// Synthesized advisory evaluation report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisoryReport {
    /// Assessed risk level on ordinal scale (0: minimal/safe, 1: normal, 2: elevated, 3: critical).
    pub risk_score: i32,
    /// High-level 1-line human-readable summary.
    pub summary: String,
    /// Key-value metrics and calibrated probabilities (e.g. "addresses_task": 0.95).
    #[serde(default)]
    pub metrics: HashMap<String, f64>,
    /// Badges to display in approval prompts and TUI modals.
    #[serde(default)]
    pub badges: Vec<AdvisoryBadge>,
    /// Name or model ID of the advisory provider (e.g. "jev-1.13.0", "heuristic").
    pub provider: String,
}

/// Inputs provided to an advisor when evaluating a candidate tool execution.
#[derive(Debug, Clone)]
pub struct AdvisoryRequest<'a> {
    pub tool_name: &'a str,
    pub arguments: &'a serde_json::Value,
    pub active_task: Option<&'a str>,
}

/// Deep module interface for advisory evaluation of tool executions.
///
/// Implementations must be non-authoritative: they provide advisory context
/// to enhance human review and agent telemetry, but must never override
/// deterministic permission policies or bypass explicit approval rules.
#[async_trait]
pub trait ToolAdvisor: Send + Sync {
    async fn advise(&self, request: &AdvisoryRequest<'_>) -> Option<AdvisoryReport>;
}

/// A no-op advisor that immediately returns `None`.
///
/// Used as the default when no advisory integration is configured or enabled.
#[derive(Debug, Default, Clone)]
pub struct NoopAdvisor;

#[async_trait]
impl ToolAdvisor for NoopAdvisor {
    async fn advise(&self, _request: &AdvisoryRequest<'_>) -> Option<AdvisoryReport> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_noop_advisor_returns_none() {
        let advisor = NoopAdvisor;
        let args = json!({"command": "ls"});
        let req = AdvisoryRequest {
            tool_name: "bash",
            arguments: &args,
            active_task: Some("list files"),
        };
        assert_eq!(advisor.advise(&req).await, None);
    }

    #[test]
    fn test_advisory_report_serialization() {
        let mut metrics = HashMap::new();
        metrics.insert("addresses_task".to_string(), 0.96);
        metrics.insert("unrelated_changes".to_string(), 0.04);

        let report = AdvisoryReport {
            risk_score: 1,
            summary: "Diff looks aligned".to_string(),
            metrics,
            badges: vec![
                AdvisoryBadge::new("Risk: Low", BadgeTone::Success),
                AdvisoryBadge::new("Scope: Aligned", BadgeTone::Success),
            ],
            provider: "jev-1.13.0".to_string(),
        };

        let json = serde_json::to_string(&report).expect("must serialize");
        assert!(json.contains("Risk: Low"));
        assert!(json.contains("jev-1.13.0"));

        let deserialized: AdvisoryReport =
            serde_json::from_str(&json).expect("must deserialize");
        assert_eq!(deserialized, report);
    }
}
