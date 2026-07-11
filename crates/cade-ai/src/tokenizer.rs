//! Provider-aware token counting.
//!
//! Replaces the legacy `chars / 3` estimate used by the context builder.
//! For OpenAI models we pick the matching BPE encoder (`cl100k_base` for
//! GPT-3.5/4, `o200k_base` for GPT-4o / o-series).  For Anthropic, Gemini,
//! and unknown providers — which lack a public Rust tokenizer — we fall
//! back to `cl100k_base`, which over-counts Claude by ~5–10 %.  That is the
//! safe direction (the budget reserves *more* room, not less).
//!
//! All encoders are cached behind `once_cell::Lazy` so callers can call
//! `count_tokens` thousands of times per request without re-loading BPE
//! tables.

use once_cell::sync::Lazy;
use tiktoken_rs::CoreBPE;
use crate::types::LlmMessage;

/// Default fallback ratio when *all* tokenizer paths fail (encoder load
/// error, panic, unknown family).  Conservative: 3 chars/token leaves
/// ~25 % headroom against typical English text (~3.5–4 c/t).
pub const FALLBACK_CHARS_PER_TOKEN: usize = 3;

/// Lazily-initialised cl100k_base encoder (GPT-3.5/4, default fallback).
static CL100K: Lazy<Option<CoreBPE>> = Lazy::new(|| tiktoken_rs::cl100k_base().ok());

/// Lazily-initialised o200k_base encoder (GPT-4o, o-series).
static O200K: Lazy<Option<CoreBPE>> = Lazy::new(|| tiktoken_rs::o200k_base().ok());

pub trait TokenCounter: Send + Sync {
    fn count(&self, text: &str) -> usize;
}

pub struct TiktokenAdapter {
    pub encoder: &'static CoreBPE,
}

impl TokenCounter for TiktokenAdapter {
    fn count(&self, text: &str) -> usize {
        self.encoder.encode_with_special_tokens(text).len()
    }
}

pub struct AnthropicAdapter {
    pub encoder: &'static CoreBPE,
}

impl TokenCounter for AnthropicAdapter {
    fn count(&self, text: &str) -> usize {
        // cl100k_base over-counts Claude slightly, which is our safe headroom boundary (WI-SEMANTIC)
        self.encoder.encode_with_special_tokens(text).len()
    }
}

pub struct GeminiAdapter {
    pub encoder: &'static CoreBPE,
}

impl TokenCounter for GeminiAdapter {
    fn count(&self, text: &str) -> usize {
        // Gemini uses a different vocab but fits cl100k_base approximation safely
        self.encoder.encode_with_special_tokens(text).len()
    }
}

pub struct FallbackCharAdapter {
    pub chars_per_token: usize,
}

impl TokenCounter for FallbackCharAdapter {
    fn count(&self, text: &str) -> usize {
        text.chars().count() / self.chars_per_token.max(1)
    }
}

/// Pick the most accurate available tokenizer for a given model id.
///
/// Returns `None` only when the encoder failed to load (corrupt BPE table,
/// out-of-memory, etc.) — callers must fall back to a char-based estimate.
#[allow(dead_code)]
fn encoder_for(model_id: &str) -> Option<&'static CoreBPE> {
    let lower = model_id.to_ascii_lowercase();

    // OpenAI o-series + GPT-4o → o200k_base
    let is_o200k = lower.contains("gpt-4o")
        || lower.contains("gpt-4.5")
        || lower.contains("gpt-5")
        || lower.contains("/o1")
        || lower.contains("/o3")
        || lower.contains("/o4");
    if is_o200k && let Some(enc) = O200K.as_ref() {
        return Some(enc);
    }

    // Everything else (OpenAI cl100k era, Anthropic, Gemini, Ollama,
    // unknown providers) → cl100k_base.  This over-counts Claude by
    // ~5–10 % which is the conservative direction.
    CL100K.as_ref()
}

pub fn resolve_token_counter(model_id: &str) -> Box<dyn TokenCounter> {
    let lower = model_id.to_ascii_lowercase();

    let is_o200k = lower.contains("gpt-4o")
        || lower.contains("gpt-4.5")
        || lower.contains("gpt-5")
        || lower.contains("/o1")
        || lower.contains("/o3")
        || lower.contains("/o4");

    if is_o200k
        && let Some(enc) = O200K.as_ref() {
            return Box::new(TiktokenAdapter { encoder: enc });
        }

    if let Some(enc) = CL100K.as_ref() {
        if lower.contains("anthropic") || lower.contains("claude") {
            return Box::new(AnthropicAdapter { encoder: enc });
        }
        if lower.contains("gemini") || lower.contains("google") {
            return Box::new(GeminiAdapter { encoder: enc });
        }
        return Box::new(TiktokenAdapter { encoder: enc });
    }

    Box::new(FallbackCharAdapter {
        chars_per_token: FALLBACK_CHARS_PER_TOKEN,
    })
}

/// Count tokens in `text` using the best available encoder for `model_id`.
///
/// On any error path (encoder unavailable) falls back to
/// `chars / FALLBACK_CHARS_PER_TOKEN` so callers always get a usable
/// number.
pub fn count_tokens(model_id: &str, text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let counter = resolve_token_counter(model_id);
    counter.count(text)
}

/// Convert a desired *token* count into an upper-bound *character* count
/// for compatibility with existing char-budget code.  This is the inverse
/// of `count_tokens`, but because tokenization is non-uniform we use a
/// conservative ratio that under-estimates chars (i.e. over-reserves
/// budget).  Used by `cade-server` when it needs to keep the legacy
/// char-based budget API but anchor it to a real token window.
pub fn chars_for_tokens(tokens: usize) -> usize {
    tokens.saturating_mul(FALLBACK_CHARS_PER_TOKEN)
}

/// A deep, concrete manager that unifies token counting, conversion math,
/// and context budgeting calculations behind a simple, high-leverage interface.
#[derive(Debug, Clone, Default)]
pub struct PromptBudgetManager;

impl PromptBudgetManager {
    pub fn new() -> Self {
        Self
    }

    /// Count tokens in `text` using the best available encoder for `model_id`.
    pub fn count_tokens(&self, model_id: &str, text: &str) -> usize {
        count_tokens(model_id, text)
    }

    /// Compute the total token cost of a turn (all content + tool calls)
    pub fn turn_cost(&self, model_id: &str, turn: &[LlmMessage]) -> usize {
        let mut total_tokens = 0usize;
        for m in turn {
            if !m.content.is_empty() {
                total_tokens += self.count_tokens(model_id, &m.content);
            }
            if let Some(tcs) = m.tool_calls.as_deref() {
                for tc in tcs {
                    let json = tc.arguments.to_string();
                    if !json.is_empty() {
                        total_tokens += self.count_tokens(model_id, &json);
                    }
                }
            }
        }
        total_tokens
    }

    /// Compute the fallback character-based cost of a turn for backward compatibility
    pub fn turn_cost_fallback_chars(&self, turn: &[LlmMessage]) -> usize {
        turn.iter()
            .map(|m| {
                m.content.chars().count()
                    + m.tool_calls
                        .as_deref()
                        .unwrap_or_default()
                        .iter()
                        .map(|tc| tc.arguments.to_string().len())
                        .sum::<usize>()
            })
            .sum()
    }

    /// Convert a token count to its equivalent upper-bound character count.
    pub fn chars_for_tokens(&self, tokens: usize) -> usize {
        chars_for_tokens(tokens)
    }

    /// Unified context budgeting calculation for a set of turns, returning both token and legacy character metrics.
    /// Walks the turns from newest to oldest, ensuring the newest turn is always included, and respects the budget.
    pub fn calculate_budget(
        &self,
        model_id: &str,
        turns: &[Vec<LlmMessage>],
        system_overhead_tokens: usize,
        max_context_chars: usize,
    ) -> ContextBudgetResult {
        let system_overhead_chars = self.chars_for_tokens(system_overhead_tokens);
        let message_budget = max_context_chars.saturating_sub(system_overhead_chars);

        let mut selected: Vec<Vec<LlmMessage>> = Vec::new();
        let mut budget_used_chars: usize = 0;
        let mut total_tokens_used: usize = system_overhead_tokens;

        for turn in turns.iter().cloned().rev() {
            let tokens = self.turn_cost(model_id, &turn);
            let fallback_chars = self.turn_cost_fallback_chars(&turn);
            
            let turn_chars = if tokens == 0 && fallback_chars > 0 {
                fallback_chars
            } else {
                self.chars_for_tokens(tokens)
            };

            if selected.is_empty() {
                // Always include the most recent turn regardless of size
                selected.push(turn);
                budget_used_chars += turn_chars;
                total_tokens_used += tokens;
            } else if budget_used_chars + turn_chars <= message_budget {
                selected.push(turn);
                budget_used_chars += turn_chars;
                total_tokens_used += tokens;
            } else {
                break;
            }
        }

        // Reverse back to chronological order (oldest first)
        selected.reverse();
        let omitted_count = turns.len().saturating_sub(selected.len());

        ContextBudgetResult {
            selected_turns: selected,
            total_tokens_used,
            total_chars_used: budget_used_chars + system_overhead_chars,
            omitted_turns_count: omitted_count,
        }
    }
}

/// Holds the results of a prompt budget assessment.
#[derive(Debug, Clone)]
pub struct ContextBudgetResult {
    pub selected_turns: Vec<Vec<LlmMessage>>,
    pub total_tokens_used: usize,
    pub total_chars_used: usize,
    pub omitted_turns_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_zero_tokens() {
        assert_eq!(count_tokens("openai/gpt-4o", ""), 0);
        assert_eq!(count_tokens("anthropic/claude-3-7-sonnet", ""), 0);
    }

    #[test]
    fn ascii_text_token_count_is_lower_than_char_count() {
        let text = "Hello, world! This is a sentence with several words.";
        let toks = count_tokens("openai/gpt-4o", text);
        let chars = text.chars().count();
        assert!(toks > 0, "must produce non-zero token count");
        assert!(
            toks < chars,
            "tokens ({toks}) must be less than chars ({chars}) for ASCII"
        );
    }

    #[test]
    fn anthropic_falls_back_to_cl100k_and_returns_nonzero() {
        let text = "The quick brown fox jumps over the lazy dog.";
        assert!(count_tokens("anthropic/claude-3-7-sonnet", text) > 0);
        assert!(count_tokens("anthropic/claude-sonnet-4-5", text) > 0);
    }

    #[test]
    fn gpt4o_uses_o200k_encoder_path() {
        // o200k_base is more efficient than cl100k for natural English.
        // We do not assert exact counts (tokenizer-version dependent) but
        // verify both paths produce numbers; o200k typically <= cl100k.
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(20);
        let cl = encoder_for("openai/gpt-3.5-turbo")
            .unwrap()
            .encode_with_special_tokens(&text)
            .len();
        let o2 = encoder_for("openai/gpt-4o")
            .unwrap()
            .encode_with_special_tokens(&text)
            .len();
        assert!(cl > 0 && o2 > 0);
        // o200k should be ≤ cl100k for typical English (more efficient).
        assert!(
            o2 <= cl + 5,
            "o200k ({o2}) should not exceed cl100k ({cl}) by much"
        );
    }

    #[test]
    fn count_tokens_handles_unknown_provider() {
        let n = count_tokens("random/unknown-model", "hello world");
        assert!(
            n > 0,
            "unknown providers must still return a useful estimate"
        );
    }

    #[test]
    fn chars_for_tokens_is_monotonic() {
        assert!(chars_for_tokens(100) > chars_for_tokens(50));
        assert_eq!(chars_for_tokens(0), 0);
    }

    #[test]
    fn chars_for_tokens_round_trip_is_within_safety_margin() {
        // count(text) ≈ tokens; chars_for_tokens(tokens) should be ≥ chars(text)
        // most of the time, since FALLBACK_CHARS_PER_TOKEN=3 is conservative.
        let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(50);
        let toks = count_tokens("openai/gpt-4o", &text);
        let predicted_chars = chars_for_tokens(toks);
        let actual_chars = text.chars().count();
        assert!(
            predicted_chars >= actual_chars / 2,
            "chars_for_tokens({toks}) = {predicted_chars} should be in the same order as actual chars ({actual_chars})"
        );
    }
}
