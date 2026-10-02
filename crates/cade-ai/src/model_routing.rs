//! Deep module for dynamic, in-engine model routing.
//!
//! Provides intelligent model selection for token conservation and latency reduction
//! using TypeSafeAI Jev System One intent classification.

// region:    --- Modules & Imports

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use serde::{Deserialize, Serialize};

// endregion: --- Modules & Imports

// region:    --- Types

/// The result of an in-engine model routing decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRoutingDecision {
    /// The concrete model to execute for this turn.
    pub effective_model: String,
    /// The base/primary model assigned to the agent or session.
    pub original_model: String,
    /// The classified task category (e.g. "recon_read", "deep_architecture").
    pub category: String,
    /// The calibrated classification confidence in [0.0, 1.0].
    pub confidence: f64,
    /// Whether the model was downgraded to a faster/cheaper tier for economy.
    pub is_downgraded_for_economy: bool,
    /// Human-readable explanation of why this routing occurred.
    pub reason: String,
}

/// Execution outcome signal emitted by a tool during a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnExecutionSignal {
    pub is_error: bool,
    pub tool_name: String,
    pub output_snippet: String,
}

/// Specific granular trigger that caused an in-flight model escalation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EscalationTrigger {
    ToolExecutionError { tool_name: String },
    CompilationOrTestFailure { detail: String },
    SecurityOrPermissionDenial { detail: String },
    LlmUncertaintyOrRefusal { phrase: String },
    RepetitiveToolLoop { tool_name: String, repetitions: usize },
}

impl EscalationTrigger {
    pub fn description(&self) -> String {
        match self {
            Self::ToolExecutionError { tool_name } => {
                format!("Tool execution error in '{tool_name}'")
            }
            Self::CompilationOrTestFailure { detail } => {
                format!("Compilation or test failure: {detail}")
            }
            Self::SecurityOrPermissionDenial { detail } => {
                format!("Security or permission denial: {detail}")
            }
            Self::LlmUncertaintyOrRefusal { phrase } => {
                format!("Model uncertainty/refusal detected: \"{phrase}\"")
            }
            Self::RepetitiveToolLoop { tool_name, repetitions } => {
                format!("Repetitive tool loop detected in '{tool_name}' ({repetitions} executions)")
            }
        }
    }
}

/// Abstract seam for model routing engines.
pub trait ModelRoutingEngine: Send + Sync {
    fn route<'a>(
        &'a self,
        base_model: &'a str,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = ModelRoutingDecision> + Send + 'a>>;
}

// endregion: --- Types

// region:    --- PassThrough Adapter

/// Fallback / default router that preserves the base model without alteration.
#[derive(Debug, Default, Clone)]
pub struct PassThroughModelRouter;

impl ModelRoutingEngine for PassThroughModelRouter {
    fn route<'a>(
        &'a self,
        base_model: &'a str,
        _prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = ModelRoutingDecision> + Send + 'a>> {
        Box::pin(async move {
            ModelRoutingDecision {
                effective_model: base_model.to_string(),
                original_model: base_model.to_string(),
                category: "unclassified".to_string(),
                confidence: 1.0,
                is_downgraded_for_economy: false,
                reason: "Pass-through: dynamic routing disabled or unconfigured".to_string(),
            }
        })
    }
}

// endregion: --- PassThrough Adapter

// region:    --- Jev Intent Adapter

/// Function signature for invoking Jev intent classification.
pub type JevClassifierFn = Arc<
    dyn Fn(
            String, // prompt
            Vec<String>, // categories
            Option<f64>, // min_confidence
            Option<String>, // safe_default
        ) -> Pin<Box<dyn Future<Output = Result<JevRouteResponse, String>> + Send>>
        + Send
        + Sync,
>;

/// Output structure matching `jev__jev_classify_intent`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JevRouteResponse {
    pub effective: String,
    pub original_selection: String,
    pub confidence: f64,
    pub fallback_reason: Option<String>,
    pub probabilities: HashMap<String, f64>,
}

/// Deep module adapter that routes prompts via Jev intent classification.
pub struct JevIntentModelRouter {
    classifier: JevClassifierFn,
    min_confidence: f64,
}

impl JevIntentModelRouter {
    pub const RECON_READ: &'static str = "recon_read";
    pub const ROUTINE_PATCH: &'static str = "routine_patch";
    pub const DEEP_ARCHITECTURE: &'static str = "deep_architecture";
    pub const SECURITY_AUDIT: &'static str = "security_audit";

    pub fn new(classifier: JevClassifierFn) -> Self {
        Self {
            classifier,
            min_confidence: 0.70,
        }
    }

    pub fn with_min_confidence(mut self, min_confidence: f64) -> Self {
        self.min_confidence = min_confidence;
        self
    }

    /// Candidate task categories passed to Jev Choice.
    pub fn candidate_categories() -> Vec<String> {
        vec![
            Self::RECON_READ.to_string(),
            Self::ROUTINE_PATCH.to_string(),
            Self::DEEP_ARCHITECTURE.to_string(),
            Self::SECURITY_AUDIT.to_string(),
        ]
    }

    /// Resolves the optimal fast-tier economy model for a given provider family.
    pub fn resolve_fast_model(base_model: &str) -> String {
        let (provider, _bare) = base_model.split_once('/').unwrap_or(("", base_model));
        match provider {
            "gemini" => "gemini/gemini-2.5-flash".to_string(),
            "anthropic" => "anthropic/claude-3-5-haiku-latest".to_string(),
            "openai" => "openai/gpt-4o-mini".to_string(),
            _ => "anthropic/claude-3-5-haiku-latest".to_string(),
        }
    }

    /// Resolves the balanced tier model for a given provider family.
    pub fn resolve_balanced_model(base_model: &str) -> String {
        let (provider, _bare) = base_model.split_once('/').unwrap_or(("", base_model));
        match provider {
            "gemini" => "gemini/gemini-2.5-flash".to_string(),
            "anthropic" => "anthropic/claude-3-5-sonnet-latest".to_string(),
            "openai" => "openai/gpt-4o".to_string(),
            _ => base_model.to_string(),
        }
    }

    /// Inherent route method for ergonomic non-trait dispatch.
    pub async fn route(&self, base_model: &str, prompt: &str) -> ModelRoutingDecision {
        <Self as ModelRoutingEngine>::route(self, base_model, prompt).await
    }

    /// Evaluates whether an in-flight economy model should be escalated back to the base frontier model.
    /// Escalation triggers when:
    /// 1. Current model is downgraded from base_model (i.e. running on fast tier).
    /// 2. Turn >= 2.
    /// 3. Any tool in the prior turn resulted in an error, compilation failure, security denial,
    ///    repetitive loop, or prior assistant text expressed refusal/uncertainty.
    pub fn evaluate_turn_escalation(
        current_model: &str,
        base_model: &str,
        turn_number: usize,
        recent_signals: &[TurnExecutionSignal],
        prior_assistant_text: Option<&str>,
    ) -> Option<ModelRoutingDecision> {
        if current_model == base_model || turn_number < 2 {
            return None;
        }

        // 1. Check for specific tool execution or compilation errors
        let mut detected_trigger = None;
        for s in recent_signals {
            if s.is_error {
                detected_trigger = Some(EscalationTrigger::ToolExecutionError {
                    tool_name: s.tool_name.clone(),
                });
                break;
            }
            if s.output_snippet.contains("error[E")
                || s.output_snippet.contains("error:")
                || s.output_snippet.contains("FAILED")
                || s.output_snippet.contains("Compilation failed")
                || s.output_snippet.contains("Build failed")
            {
                let detail = s.output_snippet.lines().next().unwrap_or("build error").to_string();
                detected_trigger = Some(EscalationTrigger::CompilationOrTestFailure { detail });
                break;
            }
            if s.output_snippet.contains("Security Exception")
                || s.output_snippet.contains("Access denied")
                || s.output_snippet.contains("Permission denied")
                || s.output_snippet.contains("blocked by policy")
            {
                let detail = s.output_snippet.lines().next().unwrap_or("access denied").to_string();
                detected_trigger = Some(EscalationTrigger::SecurityOrPermissionDenial { detail });
                break;
            }
        }

        // 2. Check for repetitive tool loop (>= 3 calls to the same tool in turn)
        if detected_trigger.is_none() && recent_signals.len() >= 3 {
            let first_tool = &recent_signals[0].tool_name;
            if recent_signals.iter().all(|s| &s.tool_name == first_tool) {
                detected_trigger = Some(EscalationTrigger::RepetitiveToolLoop {
                    tool_name: first_tool.clone(),
                    repetitions: recent_signals.len(),
                });
            }
        }

        // 3. Check for LLM uncertainty / refusal phrases in prior assistant output
        if detected_trigger.is_none()
            && let Some(text) = prior_assistant_text
        {
            let lower = text.to_lowercase();
            let uncertainty_markers = [
                "i cannot determine",
                "unable to complete",
                "need more context to resolve",
                "unsure how to",
                "i am not sure",
                "exceeds my capability",
                "i cannot solve",
            ];
            for marker in uncertainty_markers {
                if lower.contains(marker) {
                    detected_trigger = Some(EscalationTrigger::LlmUncertaintyOrRefusal {
                        phrase: marker.to_string(),
                    });
                    break;
                }
            }
        }

        detected_trigger.map(|trigger| ModelRoutingDecision {
            effective_model: base_model.to_string(),
            original_model: current_model.to_string(),
            category: Self::DEEP_ARCHITECTURE.to_string(),
            confidence: 1.0,
            is_downgraded_for_economy: false,
            reason: format!(
                "In-flight escalation: {}; restored base frontier model ({base_model})",
                trigger.description()
            ),
        })
    }
}

impl ModelRoutingEngine for JevIntentModelRouter {
    fn route<'a>(
        &'a self,
        base_model: &'a str,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = ModelRoutingDecision> + Send + 'a>> {
        Box::pin(async move {
            let categories = Self::candidate_categories();
            let safe_default = Self::DEEP_ARCHITECTURE.to_string();

            let classification = (self.classifier)(
                prompt.to_string(),
                categories,
                Some(self.min_confidence),
                Some(safe_default),
            )
            .await;

            match classification {
                Ok(resp) => {
                    let cat = resp.effective.clone();
                    let conf = resp.confidence;

                    // If confidence is below threshold or fallen back to deep architecture, keep base model
                    if resp.fallback_reason.is_some() || conf < self.min_confidence {
                        return ModelRoutingDecision {
                            effective_model: base_model.to_string(),
                            original_model: base_model.to_string(),
                            category: cat,
                            confidence: conf,
                            is_downgraded_for_economy: false,
                            reason: format!(
                                "Confidence below threshold ({conf:.2} < {:.2}); safe fallback to base model",
                                self.min_confidence
                            ),
                        };
                    }

                    match cat.as_str() {
                        Self::RECON_READ => {
                            let fast = Self::resolve_fast_model(base_model);
                            let is_downgraded = fast != base_model;
                            ModelRoutingDecision {
                                effective_model: fast,
                                original_model: base_model.to_string(),
                                category: cat,
                                confidence: conf,
                                is_downgraded_for_economy: is_downgraded,
                                reason: format!(
                                    "Routed to fast tier for reconnaissance/read economy (confidence: {conf:.2})"
                                ),
                            }
                        }
                        Self::ROUTINE_PATCH => {
                            let balanced = Self::resolve_balanced_model(base_model);
                            let is_downgraded = balanced != base_model;
                            ModelRoutingDecision {
                                effective_model: balanced,
                                original_model: base_model.to_string(),
                                category: cat,
                                confidence: conf,
                                is_downgraded_for_economy: is_downgraded,
                                reason: format!(
                                    "Routed to balanced tier for routine patch economy (confidence: {conf:.2})"
                                ),
                            }
                        }
                        _ => {
                            // Deep architecture, security audit, or complex reasoning -> preserve base model
                            ModelRoutingDecision {
                                effective_model: base_model.to_string(),
                                original_model: base_model.to_string(),
                                category: cat.clone(),
                                confidence: conf,
                                is_downgraded_for_economy: false,
                                reason: format!(
                                    "Preserved frontier model for deep task (category: {cat}, confidence: {conf:.2})"
                                ),
                            }
                        }
                    }
                }
                Err(err) => {
                    // Fail-safe: Any Jev or network failure falls back to base model seamlessly
                    ModelRoutingDecision {
                        effective_model: base_model.to_string(),
                        original_model: base_model.to_string(),
                        category: "error_fallback".to_string(),
                        confidence: 0.0,
                        is_downgraded_for_economy: false,
                        reason: format!("Jev classification error: {err}; safe fallback to base model"),
                    }
                }
            }
        })
    }
}

// endregion: --- Jev Intent Adapter

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn test_passthrough_router_preserves_base_model() -> Result<()> {
        // -- Setup & Fixtures
        let router = PassThroughModelRouter;
        let base_model = "openai/gpt-6-sol";
        let prompt = "Find all files matching *.rs";

        // -- Exec
        let decision = router.route(base_model, prompt).await;

        // -- Check
        assert_eq!(decision.effective_model, "openai/gpt-6-sol");
        assert_eq!(decision.original_model, "openai/gpt-6-sol");
        assert!(!decision.is_downgraded_for_economy);
        Ok(())
    }

    #[tokio::test]
    async fn test_jev_router_routes_recon_read_to_fast_tier() -> Result<()> {
        // -- Setup & Fixtures
        let classifier = Arc::new(|_prompt: String, _cats: Vec<String>, _min: Option<f64>, _def: Option<String>| {
            Box::pin(async {
                Ok(JevRouteResponse {
                    effective: JevIntentModelRouter::RECON_READ.to_string(),
                    original_selection: JevIntentModelRouter::RECON_READ.to_string(),
                    confidence: 0.92,
                    fallback_reason: None,
                    probabilities: HashMap::new(),
                })
            }) as Pin<Box<dyn Future<Output = core::result::Result<JevRouteResponse, String>> + Send>>
        });
        let router = JevIntentModelRouter::new(classifier);
        let base_model = "openai/gpt-6-sol";
        let prompt = "Search for error handling in crates/cade-server";

        // -- Exec
        let decision = router.route(base_model, prompt).await;

        // -- Check
        assert_eq!(decision.effective_model, "openai/gpt-4o-mini");
        assert_eq!(decision.original_model, "openai/gpt-6-sol");
        assert_eq!(decision.category, JevIntentModelRouter::RECON_READ);
        assert!(decision.is_downgraded_for_economy);
        assert!(decision.reason.contains("fast tier"));
        Ok(())
    }

    #[tokio::test]
    async fn test_jev_router_falls_back_when_confidence_below_threshold() -> Result<()> {
        // -- Setup & Fixtures
        let classifier = Arc::new(|_prompt: String, _cats: Vec<String>, _min: Option<f64>, _def: Option<String>| {
            Box::pin(async {
                Ok(JevRouteResponse {
                    effective: JevIntentModelRouter::DEEP_ARCHITECTURE.to_string(),
                    original_selection: JevIntentModelRouter::RECON_READ.to_string(),
                    confidence: 0.54, // below 0.70 threshold
                    fallback_reason: Some("confidence_below_threshold".to_string()),
                    probabilities: HashMap::new(),
                })
            }) as Pin<Box<dyn Future<Output = core::result::Result<JevRouteResponse, String>> + Send>>
        });
        let router = JevIntentModelRouter::new(classifier);
        let base_model = "openai/gpt-6-sol";
        let prompt = "Do something with the code";

        // -- Exec
        let decision = router.route(base_model, prompt).await;

        // -- Check
        assert_eq!(decision.effective_model, "openai/gpt-6-sol");
        assert!(!decision.is_downgraded_for_economy);
        assert!(decision.reason.contains("Confidence below threshold"));
        Ok(())
    }

    #[tokio::test]
    async fn test_jev_router_preserves_frontier_for_deep_architecture() -> Result<()> {
        // -- Setup & Fixtures
        let classifier = Arc::new(|_prompt: String, _cats: Vec<String>, _min: Option<f64>, _def: Option<String>| {
            Box::pin(async {
                Ok(JevRouteResponse {
                    effective: JevIntentModelRouter::DEEP_ARCHITECTURE.to_string(),
                    original_selection: JevIntentModelRouter::DEEP_ARCHITECTURE.to_string(),
                    confidence: 0.95,
                    fallback_reason: None,
                    probabilities: HashMap::new(),
                })
            }) as Pin<Box<dyn Future<Output = core::result::Result<JevRouteResponse, String>> + Send>>
        });
        let router = JevIntentModelRouter::new(classifier);
        let base_model = "openai/gpt-6-sol";
        let prompt = "Refactor the memory subsystem to support zero-copy arena allocation";

        // -- Exec
        let decision = router.route(base_model, prompt).await;

        // -- Check
        assert_eq!(decision.effective_model, "openai/gpt-6-sol");
        assert_eq!(decision.original_model, "openai/gpt-6-sol");
        assert!(!decision.is_downgraded_for_economy);
        Ok(())
    }

    #[tokio::test]
    async fn test_provider_family_fast_model_resolution() -> Result<()> {
        // -- Exec & Check
        assert_eq!(
            JevIntentModelRouter::resolve_fast_model("gemini/gemini-2.5-pro"),
            "gemini/gemini-2.5-flash"
        );
        assert_eq!(
            JevIntentModelRouter::resolve_fast_model("anthropic/claude-3-7-sonnet"),
            "anthropic/claude-3-5-haiku-latest"
        );
        assert_eq!(
            JevIntentModelRouter::resolve_fast_model("openai/gpt-6-sol"),
            "openai/gpt-4o-mini"
        );
        Ok(())
    }

    #[test]
    fn test_evaluate_turn_escalation_triggers_on_tool_error() -> Result<()> {
        // -- Setup & Fixtures
        let current_model = "openai/gpt-4o-mini";
        let base_model = "openai/gpt-6-sol";
        let turn_number = 2;
        let signals = vec![TurnExecutionSignal {
            is_error: true,
            tool_name: "bash".to_string(),
            output_snippet: "generic failure".to_string(),
        }];

        // -- Exec
        let decision = JevIntentModelRouter::evaluate_turn_escalation(
            current_model,
            base_model,
            turn_number,
            &signals,
            None,
        );

        // -- Check
        let dec = decision.ok_or("Expected escalation decision on error signal")?;
        assert_eq!(dec.effective_model, "openai/gpt-6-sol");
        assert_eq!(dec.original_model, "openai/gpt-4o-mini");
        assert!(dec.reason.contains("Tool execution error in 'bash'"));
        Ok(())
    }

    #[test]
    fn test_evaluate_turn_escalation_triggers_on_compilation_failure() -> Result<()> {
        // -- Setup & Fixtures
        let current_model = "gemini/gemini-2.5-flash";
        let base_model = "gemini/gemini-2.5-pro";
        let turn_number = 2;
        let signals = vec![TurnExecutionSignal {
            is_error: false,
            tool_name: "bash".to_string(),
            output_snippet: "error[E0308]: mismatched types\n  expected struct A, found struct B".to_string(),
        }];

        // -- Exec
        let decision = JevIntentModelRouter::evaluate_turn_escalation(
            current_model,
            base_model,
            turn_number,
            &signals,
            None,
        );

        // -- Check
        let dec = decision.ok_or("Expected escalation on compiler error")?;
        assert_eq!(dec.effective_model, "gemini/gemini-2.5-pro");
        assert!(dec.reason.contains("Compilation or test failure"));
        Ok(())
    }

    #[test]
    fn test_evaluate_turn_escalation_triggers_on_security_denial() -> Result<()> {
        // -- Setup & Fixtures
        let current_model = "anthropic/claude-3-5-haiku-latest";
        let base_model = "anthropic/claude-3-7-sonnet";
        let turn_number = 3;
        let signals = vec![TurnExecutionSignal {
            is_error: false,
            tool_name: "write_file".to_string(),
            output_snippet: "Security Exception: Access denied to path outside sandbox boundary".to_string(),
        }];

        // -- Exec
        let decision = JevIntentModelRouter::evaluate_turn_escalation(
            current_model,
            base_model,
            turn_number,
            &signals,
            None,
        );

        // -- Check
        let dec = decision.ok_or("Expected escalation on security denial")?;
        assert_eq!(dec.effective_model, "anthropic/claude-3-7-sonnet");
        assert!(dec.reason.contains("Security or permission denial"));
        Ok(())
    }

    #[test]
    fn test_evaluate_turn_escalation_triggers_on_repetitive_tool_loop() -> Result<()> {
        // -- Setup & Fixtures
        let current_model = "openai/gpt-4o-mini";
        let base_model = "openai/gpt-6-sol";
        let turn_number = 2;
        let signals = vec![
            TurnExecutionSignal {
                is_error: false,
                tool_name: "grep".to_string(),
                output_snippet: "no match".to_string(),
            },
            TurnExecutionSignal {
                is_error: false,
                tool_name: "grep".to_string(),
                output_snippet: "no match".to_string(),
            },
            TurnExecutionSignal {
                is_error: false,
                tool_name: "grep".to_string(),
                output_snippet: "no match".to_string(),
            },
        ];

        // -- Exec
        let decision = JevIntentModelRouter::evaluate_turn_escalation(
            current_model,
            base_model,
            turn_number,
            &signals,
            None,
        );

        // -- Check
        let dec = decision.ok_or("Expected escalation on repetitive tool loop")?;
        assert_eq!(dec.effective_model, "openai/gpt-6-sol");
        assert!(dec.reason.contains("Repetitive tool loop detected in 'grep'"));
        Ok(())
    }

    #[test]
    fn test_evaluate_turn_escalation_triggers_on_llm_uncertainty() -> Result<()> {
        // -- Setup & Fixtures
        let current_model = "openai/gpt-4o-mini";
        let base_model = "openai/gpt-6-sol";
        let turn_number = 2;
        let signals = vec![];
        let assistant_text = "I am unable to complete this refactoring safely without deeper architectural analysis.";

        // -- Exec
        let decision = JevIntentModelRouter::evaluate_turn_escalation(
            current_model,
            base_model,
            turn_number,
            &signals,
            Some(assistant_text),
        );

        // -- Check
        let dec = decision.ok_or("Expected escalation on assistant uncertainty")?;
        assert_eq!(dec.effective_model, "openai/gpt-6-sol");
        assert!(dec.reason.contains("Model uncertainty/refusal detected"));
        Ok(())
    }

    #[test]
    fn test_evaluate_turn_escalation_skips_when_already_on_base_model() -> Result<()> {
        // -- Setup & Fixtures
        let current_model = "openai/gpt-6-sol";
        let base_model = "openai/gpt-6-sol";
        let turn_number = 2;
        let signals = vec![TurnExecutionSignal {
            is_error: true,
            tool_name: "bash".to_string(),
            output_snippet: "error".to_string(),
        }];

        // -- Exec
        let decision = JevIntentModelRouter::evaluate_turn_escalation(
            current_model,
            base_model,
            turn_number,
            &signals,
            None,
        );

        // -- Check
        assert!(decision.is_none(), "Must not escalate if already on base model");
        Ok(())
    }

    #[test]
    fn test_evaluate_turn_escalation_skips_when_turn_is_1() -> Result<()> {
        // -- Setup & Fixtures
        let current_model = "openai/gpt-4o-mini";
        let base_model = "openai/gpt-6-sol";
        let turn_number = 1;
        let signals = vec![TurnExecutionSignal {
            is_error: true,
            tool_name: "bash".to_string(),
            output_snippet: "error".to_string(),
        }];

        // -- Exec
        let decision = JevIntentModelRouter::evaluate_turn_escalation(
            current_model,
            base_model,
            turn_number,
            &signals,
            None,
        );

        // -- Check
        assert!(decision.is_none(), "Must not evaluate turn escalation on Turn 1");
        Ok(())
    }
}

// endregion: --- Tests
