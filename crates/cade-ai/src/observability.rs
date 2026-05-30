//! OpenTelemetry GenAI Semantic Convention Observability module.
//!
//! Provides standardized trace spans and attributes to monitor LLM latencies,
//! token usage, and model metadata across CADE's provider integrations.

pub mod semconv {
    pub const GEN_AI_SYSTEM: &str = "gen_ai.system";
    pub const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
    pub const GEN_AI_REQUEST_MAX_TOKENS: &str = "gen_ai.request.max_tokens";
    pub const GEN_AI_RESPONSE_MODEL: &str = "gen_ai.response.model";
    pub const GEN_AI_USAGE_INPUT_TOKENS: &str = "gen_ai.response.tokens.input";
    pub const GEN_AI_USAGE_OUTPUT_TOKENS: &str = "gen_ai.response.tokens.output";
}

/// Creates an OpenTelemetry-compliant GenAI tracing span.
#[macro_export]
macro_rules! gen_ai_span {
    ($system:expr, $request:expr) => {
        tracing::info_span!(
            "llm.completion",
            "gen_ai.system" = $system,
            "gen_ai.request.model" = $request.model.as_str(),
            "gen_ai.request.max_tokens" = $request.max_tokens,
            "gen_ai.response.model" = tracing::field::Empty,
            "gen_ai.response.tokens.input" = tracing::field::Empty,
            "gen_ai.response.tokens.output" = tracing::field::Empty,
        )
    };
}
