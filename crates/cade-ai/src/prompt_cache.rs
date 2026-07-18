use crate::{CompletionRequest, count_tokens};

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
        // 1. Annotate the first system message (static system prompt)
        if let Some(sys_msg) = req.messages.first_mut()
            && sys_msg.role == "system" {
                sys_msg.cache_control = Some("ephemeral".to_string());
            }

        // 2. Annotate the last tool schema
        if let Some(last_tool) = req.tools.last_mut()
            && let Some(obj) = last_tool.as_object_mut() {
                obj.insert(
                    "cache_control".to_string(),
                    serde_json::json!({ "type": "ephemeral" }),
                );
            }

        // 3. Annotate the second-to-last user message (multi-turn history caching)
        let mut user_count = 0;
        for msg in req.messages.iter_mut().rev() {
            if msg.role == "user" {
                user_count += 1;
                if user_count == 2 {
                    msg.cache_control = Some("ephemeral".to_string());
                    break;
                }
            }
        }
    }
}

// ── OpenAI Cache Adapter ─────────────────────────────────────────────────────

pub struct OpenAiCacheAdapter;

impl PromptCacheManager for OpenAiCacheAdapter {
    fn optimize(&self, req: &mut CompletionRequest) {
        // OpenAI automatically caches segments of prompts longer than 1024 tokens
        // and matches on 128-token boundaries.
        // We pad the system_static block (the first system message) to the nearest
        // 128-token boundary using the Model's active tokenizer to maximize hits.
        if let Some(sys_msg) = req.messages.first_mut()
            && sys_msg.role == "system" && !sys_msg.content.is_empty() {
                let model = &req.model;
                let tokens = count_tokens(model, &sys_msg.content);
                
                if tokens > 0 {
                    let remainder = tokens % 128;
                    if remainder > 0 {
                        let pad_tokens = 128 - remainder;
                        let target_tokens = tokens + pad_tokens;
                        let mut padded_content = sys_msg.content.clone();
                        
                        // Iteratively pad with spaces until count_tokens matches target_tokens
                        for _ in 0..1000 {
                            let current_toks = count_tokens(model, &padded_content);
                            if current_toks >= target_tokens {
                                break;
                            }
                            padded_content.push(' ');
                        }
                        sys_msg.content = padded_content;
                    }
                } else {
                    // Character fallback: pad to 512-character boundary
                    let len = sys_msg.content.len();
                    let remainder = len % 512;
                    if remainder > 0 {
                        let padding_len = 512 - remainder;
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
        // Left as a stub for future integration when the networking/REST layer supports it.
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
    let lower = model_id.to_lowercase();
    if lower.starts_with("anthropic/") || lower.contains("claude") {
        Box::new(AnthropicCacheAdapter)
    } else if lower.starts_with("openai/") || lower.contains("gpt") || lower.contains("o1") || lower.contains("o3") {
        Box::new(OpenAiCacheAdapter)
    } else if lower.starts_with("google/") || lower.contains("gemini") {
        Box::new(GeminiCacheAdapter)
    } else {
        Box::new(FallbackCacheAdapter)
    }
}
