//! TypeSafeAI Jev advisory adapter for pre-screening tool execution requests.

use std::collections::HashMap;
use std::time::Duration;
use async_trait::async_trait;
use cade_core::permissions::{
    AdvisoryBadge, AdvisoryReport, AdvisoryRequest, BadgeTone, ToolAdvisor,
};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Configuration options for the Jev advisor.
#[derive(Debug, Clone)]
pub struct JevAdvisorConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub timeout: Duration,
}

impl Default for JevAdvisorConfig {
    fn default() -> Self {
        let api_key = std::env::var("TYPESAFE_API_KEY").ok();
        let enabled = api_key.is_some()
            || std::env::var("CADE_ENABLE_JEV")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
        let base_url = std::env::var("JEV_BASE_URL")
            .unwrap_or_else(|_| "https://api.typesafe.ai".to_string());
        let model = std::env::var("JEV_MODEL")
            .unwrap_or_else(|_| "jev-1.13.0".to_string());

        Self {
            enabled,
            api_key,
            base_url,
            model,
            timeout: Duration::from_millis(1500),
        }
    }
}

/// Jev System One question payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum SystemOneQuestion {
    Noul {
        id: String,
        hypothesis: String,
    },
    Score {
        id: String,
        question: String,
        min: i32,
        max: i32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SystemOneRequestPayload {
    model: String,
    state: String,
    questions: Vec<SystemOneQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum SystemOneResultItem {
    Noul {
        probability: f64,
        #[serde(default)]
        verdict: Option<bool>,
    },
    Score {
        score: i32,
        #[serde(default)]
        confidence: Option<f64>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SystemOneResponsePayload {
    model: String,
    results: HashMap<String, SystemOneResultItem>,
}

/// Production advisor connecting to TypeSafeAI Jev System One endpoint.
#[derive(Debug, Clone)]
pub struct JevAdvisor {
    config: JevAdvisorConfig,
    http: reqwest::Client,
}

impl JevAdvisor {
    pub fn new(config: JevAdvisorConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .unwrap_or_default();
        Self { config, http }
    }

    /// Redact sensitive secrets from text before sending across the wire.
    pub fn sanitize(text: &str) -> String {
        // Redact PEM private keys
        let pem_re = Regex::new(
            r"(?s)-----BEGIN [A-Z ]+PRIVATE KEY-----.*?-----END [A-Z ]+PRIVATE KEY-----",
        )
        .unwrap();
        let s = pem_re.replace_all(text, "[REDACTED_PRIVATE_KEY]");

        // Redact common API keys
        let key_re = Regex::new(
            r"(?i)\b(sk-[a-zA-Z0-9]{20,}|ghp_[a-zA-Z0-9]{36}|Bearer\s+[a-zA-Z0-9_\-\.]{20,})\b",
        )
        .unwrap();
        let s = key_re.replace_all(&s, "[REDACTED_API_KEY]");

        // Redact password assignments
        let pass_re = Regex::new(
            r#"(?i)(password|secret|api_key|token)\s*[:=]\s*["']([^"']{4,})["']"#,
        )
        .unwrap();
        pass_re
            .replace_all(&s, "$1: \"[REDACTED_SECRET]\"")
            .to_string()
    }
}

#[async_trait]
impl ToolAdvisor for JevAdvisor {
    async fn advise(&self, request: &AdvisoryRequest<'_>) -> Option<AdvisoryReport> {
        if !self.config.enabled {
            return None;
        }

        let api_key = self.config.api_key.as_ref()?;

        let task_str = request.active_task.unwrap_or("general code editing");
        let args_str = serde_json::to_string_pretty(request.arguments).unwrap_or_default();
        let raw_state = format!(
            "TOOL: {}\nACTIVE TASK: {}\nARGUMENTS:\n{}",
            request.tool_name, task_str, args_str
        );
        let sanitized_state = Self::sanitize(&raw_state);

        let questions = vec![
            SystemOneQuestion::Noul {
                id: "addresses_task".to_string(),
                hypothesis: "This tool execution directly advances the active task.".to_string(),
            },
            SystemOneQuestion::Noul {
                id: "unrelated_changes".to_string(),
                hypothesis: "This tool execution touches files or resources outside the scope of the task.".to_string(),
            },
            SystemOneQuestion::Noul {
                id: "touches_sensitive".to_string(),
                hypothesis: "This tool execution accesses, exposes, or modifies credentials, private tokens, or security configuration.".to_string(),
            },
            SystemOneQuestion::Score {
                id: "risk_score".to_string(),
                question: "Risk level of this action (0: read-only/safe, 1: standard edit, 2: elevated/system logic, 3: critical/destructive):".to_string(),
                min: 0,
                max: 3,
            },
        ];

        let payload = SystemOneRequestPayload {
            model: self.config.model.clone(),
            state: sanitized_state,
            questions,
        };

        let url = format!("{}/v1/systemone", self.config.base_url.trim_end_matches('/'));

        let resp_res = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await;

        let response = match resp_res {
            Ok(resp) if resp.status().is_success() => resp,
            Ok(resp) => {
                tracing::warn!(
                    status = resp.status().as_u16(),
                    "Jev advisory API returned non-success; failing closed to standard review"
                );
                return None;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Jev advisory evaluation failed or timed out; failing closed to standard review"
                );
                return None;
            }
        };

        let parsed: SystemOneResponsePayload = response.json().await.ok()?;

        let mut metrics = HashMap::new();
        let mut badges = Vec::new();

        let mut p_task = 0.5;
        let mut p_unrelated = 0.5;
        let mut p_sensitive = 0.0;
        let mut risk_score = 1;

        for (id, item) in &parsed.results {
            match item {
                SystemOneResultItem::Noul { probability, .. } => {
                    metrics.insert(id.clone(), *probability);
                    match id.as_str() {
                        "addresses_task" => p_task = *probability,
                        "unrelated_changes" => p_unrelated = *probability,
                        "touches_sensitive" => p_sensitive = *probability,
                        _ => {}
                    }
                }
                SystemOneResultItem::Score { score, .. } => {
                    metrics.insert(id.clone(), *score as f64);
                    if id == "risk_score" {
                        risk_score = *score;
                    }
                }
                SystemOneResultItem::Unknown => {}
            }
        }

        // 1. Risk level badge
        match risk_score {
            0 => badges.push(AdvisoryBadge::new("Risk: Minimal", BadgeTone::Success)),
            1 => badges.push(AdvisoryBadge::new("Risk: Low", BadgeTone::Success)),
            2 => badges.push(AdvisoryBadge::new("Risk: Elevated", BadgeTone::Warning)),
            _ => badges.push(AdvisoryBadge::new("Risk: High", BadgeTone::Danger)),
        }

        // 2. Task scope badge
        if p_task >= 0.75 {
            badges.push(AdvisoryBadge::new("Scope: Aligned", BadgeTone::Success));
        } else if p_task < 0.40 {
            badges.push(AdvisoryBadge::new("Scope: Uncertain", BadgeTone::Warning));
        }

        // 3. Sensitive configuration alert
        if p_sensitive > 0.40 {
            badges.push(AdvisoryBadge::new("Sensitive Data", BadgeTone::Danger));
        }

        // 4. Scope drift alert
        if p_unrelated > 0.50 {
            badges.push(AdvisoryBadge::new("Scope Drift", BadgeTone::Warning));
        }

        let summary = format!("✅ Evaluated by Jev (risk level {risk_score}/3)");

        Some(AdvisoryReport {
            risk_score,
            summary,
            metrics,
            badges,
            provider: parsed.model,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn test_sanitization_removes_keys_and_passwords() {
        let text = "token is sk-123456789012345678901234 and password: \"mysecret123\"";
        let sanitized = JevAdvisor::sanitize(text);
        assert!(!sanitized.contains("sk-123456789012345678901234"));
        assert!(!sanitized.contains("mysecret123"));
        assert!(sanitized.contains("[REDACTED_API_KEY]"));
        assert!(sanitized.contains("[REDACTED_SECRET]"));
    }

    #[tokio::test]
    async fn test_jev_advisor_disabled_returns_none() {
        let config = JevAdvisorConfig {
            enabled: false,
            api_key: Some("key".to_string()),
            ..Default::default()
        };
        let advisor = JevAdvisor::new(config);
        let args = json!({"cmd": "ls"});
        let req = AdvisoryRequest {
            tool_name: "bash",
            arguments: &args,
            active_task: Some("test"),
        };
        assert_eq!(advisor.advise(&req).await, None);
    }

    #[tokio::test]
    async fn test_jev_advisor_with_mock_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;

                let mock_body = r#"{
                    "model": "jev-1.13.0",
                    "results": {
                        "addresses_task": { "type": "noul", "probability": 0.92 },
                        "unrelated_changes": { "type": "noul", "probability": 0.08 },
                        "touches_sensitive": { "type": "noul", "probability": 0.02 },
                        "risk_score": { "type": "score", "score": 1 }
                    }
                }"#;

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    mock_body.len(),
                    mock_body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let config = JevAdvisorConfig {
            enabled: true,
            api_key: Some("test_key".to_string()),
            base_url,
            model: "jev-1.13.0".to_string(),
            timeout: Duration::from_secs(2),
        };
        let advisor = JevAdvisor::new(config);
        let args = json!({"command": "cargo test"});
        let req = AdvisoryRequest {
            tool_name: "bash",
            arguments: &args,
            active_task: Some("run tests"),
        };

        let report = advisor.advise(&req).await.expect("must return advisory report");
        assert_eq!(report.risk_score, 1);
        assert_eq!(report.provider, "jev-1.13.0");
        assert!(report.badges.iter().any(|b| b.label == "Risk: Low"));
        assert!(report.badges.iter().any(|b| b.label == "Scope: Aligned"));
    }
}
