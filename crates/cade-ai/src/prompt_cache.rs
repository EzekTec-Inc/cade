use crate::CompletionRequest;
use crate::runtime::{ModelMetadata, PromptCacheKind};
use crate::tokenizer::{TokenCounter, bpe_counter_with_metadata};

/// Polymorphic interface for managing and optimizing prompt caching
/// across different LLM providers.
pub trait PromptCacheManager: Send + Sync {
    /// Optimizes the completion request in-place for prompt caching.
    /// This includes padding strings, inserting cache control markers,
    /// and aligning segment boundaries.
    fn optimize(&self, req: &mut CompletionRequest);
}

// ── Anthropic (Claude) Cache Adapter ─────────────────────────────────────────

pub struct AnthropicCacheAdapter;

impl PromptCacheManager for AnthropicCacheAdapter {
    fn optimize(&self, req: &mut CompletionRequest) {
        // 1. Breakpoint 1: Static system prompt & constitutions
        if let Some(sys_msg) = req.messages.first_mut()
            && sys_msg.role == "system"
        {
            sys_msg.cache_control = Some("ephemeral".to_string());
        }

        // 2. Breakpoint 2: Final tool schema
        if let Some(last_tool) = req.tools.last_mut()
            && let Some(obj) = last_tool.as_object_mut()
        {
            obj.insert(
                "cache_control".to_string(),
                serde_json::json!({ "type": "ephemeral" }),
            );
        }

        // 3. Multi-turn conversation breakpoints (up to 2 user turns: milestone + recent)
        let user_indices: Vec<usize> = req
            .messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
            .collect();

        if user_indices.len() >= 4 {
            let milestone_idx = user_indices[user_indices.len() / 2];
            req.messages[milestone_idx].cache_control = Some("ephemeral".to_string());

            let recent_idx = user_indices[user_indices.len().saturating_sub(2)];
            req.messages[recent_idx].cache_control = Some("ephemeral".to_string());
        } else if let Some(&target_idx) = user_indices
            .iter()
            .rev()
            .nth(1)
            .or_else(|| user_indices.last())
        {
            req.messages[target_idx].cache_control = Some("ephemeral".to_string());
        }
    }
}

// ── OpenAI Cache Adapter ─────────────────────────────────────────────────────

pub struct OpenAiCacheAdapter;

impl PromptCacheManager for OpenAiCacheAdapter {
    fn optimize(&self, req: &mut CompletionRequest) {
        let policy = crate::catalogue::metadata_for_model(&req.model);
        self.optimize_with_metadata(req, &policy);
    }
}

impl OpenAiCacheAdapter {
    /// Apply a stable, explicit policy, including the character fallback when
    /// BPE is unconfigured or unavailable. Unknown cache capability is a no-op.
    pub fn optimize_with_metadata(&self, req: &mut CompletionRequest, policy: &ModelMetadata) {
        if policy.prompt_cache != Some(PromptCacheKind::Openai) {
            return;
        }
        let padding_limit = policy.cache_padding_limit.unwrap_or(0);
        if let Some(sys_msg) = req.messages.first_mut()
            && sys_msg.role == "system"
            && !sys_msg.content.is_empty()
        {
            if let Some(counter) = bpe_counter_with_metadata(policy)
                && let Some(boundary) = policy.cache_token_boundary.filter(|value| *value > 0)
                && let tokens = counter.count(&sys_msg.content)
                && tokens > 0
            {
                let remainder = tokens % boundary;
                if remainder > 0 {
                    let pad_tokens = boundary - remainder;
                    let target_tokens = tokens.saturating_add(pad_tokens);
                    let mut padded_content = sys_msg.content.clone();

                    // Iteratively pad with spaces until count_tokens matches target_tokens
                    for _ in 0..padding_limit {
                        let current_toks = counter.count(&padded_content);
                        if current_toks >= target_tokens {
                            break;
                        }
                        padded_content.push(' ');
                    }
                    sys_msg.content = padded_content;
                }
            } else {
                // Estimates are not precise BPE boundaries. Use the configured
                // character boundary even when the estimator reports nonzero tokens.
                let Some(boundary) = policy.cache_character_boundary.filter(|value| *value > 0)
                else {
                    return;
                };
                let len = sys_msg.content.chars().count();
                let remainder = len % boundary;
                if remainder > 0 {
                    let padding_len = (boundary - remainder).min(padding_limit);
                    sys_msg.content.push_str(&" ".repeat(padding_len));
                }
            }
        }
    }
}

// ── Gemini Cache Adapter ─────────────────────────────────────────────────────

pub struct GeminiCacheAdapter;

impl PromptCacheManager for GeminiCacheAdapter {
    fn optimize(&self, _req: &mut CompletionRequest) {
        // Gemini caching requires explicit creation and references to cachedContent sessions.
        // This is handled statefully at the transport/provider layer in `GeminiProvider`
        // (inside `crates/cade-ai/src/gemini.rs`) by dynamically creating and injecting
        // `cachedContent` session references into the REST payloads sent to Google.
    }
}

// ── Fallback Cache Adapter ───────────────────────────────────────────────────

pub struct FallbackCacheAdapter;

impl PromptCacheManager for FallbackCacheAdapter {
    fn optimize(&self, _req: &mut CompletionRequest) {
        // Default fallback: do nothing
    }
}

// ── Resolver ─────────────────────────────────────────────────────────────────

/// Resolves the optimal `PromptCacheManager` based on the active model ID.
pub fn resolve_prompt_cache_manager(model_id: &str) -> Box<dyn PromptCacheManager> {
    use crate::runtime::PromptCacheKind;
    match crate::catalogue::metadata_for_model(model_id).prompt_cache {
        Some(PromptCacheKind::Anthropic) => Box::new(AnthropicCacheAdapter),
        Some(PromptCacheKind::Openai) => Box::new(OpenAiCacheAdapter),
        Some(PromptCacheKind::Gemini) => Box::new(GeminiCacheAdapter),
        Some(PromptCacheKind::None) | None => Box::new(FallbackCacheAdapter),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LlmMessage;
    use serde_json::json;

    #[test]
    fn test_resolve_prompt_cache_manager() {
        let anthropic = resolve_prompt_cache_manager("anthropic/claude-3-5-sonnet");
        let mut req = CompletionRequest {
            model: "claude-3-5-sonnet".to_string(),
            messages: vec![LlmMessage {
                role: "system".to_string(),
                content: "sys".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            }],
            tools: vec![],
            max_tokens: 0,
            reasoning_effort: None,
        };
        anthropic.optimize(&mut req);
        assert_eq!(req.messages[0].cache_control, Some("ephemeral".to_string()));

        let gemini = resolve_prompt_cache_manager("google/gemini-1.5-pro");
        let mut req_gemini = CompletionRequest {
            model: "gemini-1.5-pro".to_string(),
            messages: vec![LlmMessage {
                role: "system".to_string(),
                content: "sys".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            }],
            tools: vec![],
            max_tokens: 0,
            reasoning_effort: None,
        };
        gemini.optimize(&mut req_gemini);
        assert_eq!(req_gemini.messages[0].cache_control, None);
    }

    #[test]
    fn test_anthropic_cache_optimization() {
        let adapter = AnthropicCacheAdapter;
        let mut req = CompletionRequest {
            model: "claude-3-5-sonnet".to_string(),
            messages: vec![
                LlmMessage {
                    role: "system".to_string(),
                    content: "static system prompt".to_string(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                    cache_control: None,
                },
                LlmMessage {
                    role: "user".to_string(),
                    content: "user msg 1".to_string(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                    cache_control: None,
                },
                LlmMessage {
                    role: "assistant".to_string(),
                    content: "assistant msg 1".to_string(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                    cache_control: None,
                },
                LlmMessage {
                    role: "user".to_string(),
                    content: "user msg 2".to_string(),
                    tool_call_id: None,
                    tool_calls: None,
                    images: None,
                    cache_control: None,
                },
            ],
            tools: vec![json!({
                "name": "tool_1",
                "description": "desc 1"
            })],
            max_tokens: 0,
            reasoning_effort: None,
        };

        adapter.optimize(&mut req);

        // First system message annotated
        assert_eq!(req.messages[0].cache_control, Some("ephemeral".to_string()));

        // Second-to-last user message annotated (which is req.messages[1], the first user msg)
        assert_eq!(req.messages[1].cache_control, Some("ephemeral".to_string()));

        // Last tool schema annotated
        assert_eq!(
            req.tools[0].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    #[test]
    fn test_openai_cache_optimization_fallback() {
        let adapter = OpenAiCacheAdapter;
        let mut req = CompletionRequest {
            model: "private/nonexistent-model-so-it-triggers-fallback".to_string(),
            messages: vec![LlmMessage {
                role: "system".to_string(),
                content: "system prompt".to_string(),
                tool_call_id: None,
                tool_calls: None,
                images: None,
                cache_control: None,
            }],
            tools: vec![],
            max_tokens: 0,
            reasoning_effort: None,
        };

        let registry = crate::runtime::RuntimeRegistry::from_json(&json!({
            "models":[{"id":req.model, "prompt_cache":"openai", "tokenizer":"characters",
                "cache_token_boundary":8, "cache_character_boundary":16, "cache_padding_limit":16}]
        }).to_string()).unwrap();
        let policy = registry.metadata_for_id(&req.model);
        adapter.optimize_with_metadata(&mut req, &policy);
        // Character mode must follow its own boundary, not treat an estimate as BPE.
        assert_eq!(req.messages[0].content, "system prompt   ");
        assert_eq!(req.messages[0].content.chars().count() % 16, 0);
        adapter.optimize_with_metadata(&mut req, &policy);
        assert_eq!(req.messages[0].content, "system prompt   ");

        let mut limited = req.clone();
        limited.messages[0].content = "system prompt".into();
        let mut bounded = policy.clone();
        bounded.cache_padding_limit = Some(2);
        adapter.optimize_with_metadata(&mut limited, &bounded);
        assert_eq!(limited.messages[0].content, "system prompt  ");

        let mut disabled = policy;
        disabled.prompt_cache = Some(PromptCacheKind::None);
        limited.messages[0].content = "system prompt".into();
        adapter.optimize_with_metadata(&mut limited, &disabled);
        assert_eq!(limited.messages[0].content, "system prompt");

        // An unknown name alone does not enable padding under the default policy.
        adapter.optimize(&mut limited);
        assert_eq!(limited.messages[0].content, "system prompt");
    }
}
