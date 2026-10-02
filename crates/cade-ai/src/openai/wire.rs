//! Deep module: Autonomic Dual-Wire Engine for OpenAI.
//!
//! Encapsulates protocol-level adaptations for:
//! 1. Standard Chat Completions (`/v1/chat/completions`)
//! 2. Modern Responses API (`/v1/responses`)
//!
//! Isolates wire schemas, endpoint resolution, and protocol decoding behind a small interface.

use serde_json::Value;

use super::{ApiProtocol, OpenAiProvider, responses};
use crate::types::{CompletionRequest, CompletionResponse};
use crate::Result;

/// Autonomic wire engine coordinating protocol-level adaptations.
pub struct OpenAiWireEngine;

impl OpenAiWireEngine {
    /// Build the wire request body according to the target protocol dialect.
    pub fn build_body(
        provider: &OpenAiProvider,
        req: &CompletionRequest,
        stream: bool,
        protocol: ApiProtocol,
    ) -> Value {
        match protocol {
            ApiProtocol::Responses => ResponsesAdapter::build_body(provider, req, stream),
            ApiProtocol::ChatCompletions => ChatCompletionsAdapter::build_body(provider, req, stream),
        }
    }

    /// Decode a complete JSON response according to the wire protocol.
    pub fn decode_response(body: &Value, protocol: ApiProtocol) -> Result<CompletionResponse> {
        match protocol {
            ApiProtocol::Responses => responses::decode(body),
            ApiProtocol::ChatCompletions => {
                if let Some(choices) = body.get("choices").and_then(Value::as_array)
                    && let Some(choice0) = choices.first()
                    && choice0.get("message").is_some()
                {
                    Ok(OpenAiProvider::parse_response(body))
                } else {
                    Err(crate::Error::custom(
                        "Expected Chat Completions choices[0].message",
                    ))
                }
            }
        }
    }

    /// Validate that an incoming SSE event matches the expected wire protocol.
    pub fn validate_stream_event(
        event: &str,
        v: &Value,
        protocol: ApiProtocol,
    ) -> Result<()> {
        if (protocol == ApiProtocol::Responses && v.get("choices").is_some())
            || (protocol == ApiProtocol::ChatCompletions && event.starts_with("response."))
        {
            return Err(crate::Error::custom(
                "Completion stream protocol does not match configured endpoint",
            ));
        }
        Ok(())
    }
}

/// Adapter for standard Chat Completions (`/v1/chat/completions`).
pub struct ChatCompletionsAdapter;

impl ChatCompletionsAdapter {
    pub fn build_body(provider: &OpenAiProvider, req: &CompletionRequest, stream: bool) -> Value {
        provider.build_chat_body(req, stream)
    }
}

/// Adapter for modern Responses API (`/v1/responses`).
pub struct ResponsesAdapter;

impl ResponsesAdapter {
    pub fn build_body(provider: &OpenAiProvider, req: &CompletionRequest, stream: bool) -> Value {
        provider.build_responses_body(req, stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_stream_event_mismatches() {
        let chat_json = serde_json::json!({ "choices": [{ "delta": { "content": "hi" } }] });
        let responses_json = serde_json::json!({ "type": "response.output_item.added" });

        assert!(
            OpenAiWireEngine::validate_stream_event("", &chat_json, ApiProtocol::Responses).is_err(),
            "Chat choices payload must be rejected on Responses protocol stream"
        );
        assert!(
            OpenAiWireEngine::validate_stream_event("response.output_item.added", &responses_json, ApiProtocol::ChatCompletions).is_err(),
            "response.* event must be rejected on ChatCompletions protocol stream"
        );
        assert!(
            OpenAiWireEngine::validate_stream_event("", &chat_json, ApiProtocol::ChatCompletions).is_ok(),
            "Chat payload on ChatCompletions stream must be accepted"
        );
    }
}
