//! Metadata-selected token counting. Encoder kinds are supported Rust adapters;
//! model/provider assignments and fallback ratios live in editable runtime JSON.
//! Approximation is not a claim that a provider uses the selected vocabulary.
//!
//! All encoders are cached behind `once_cell::Lazy` so callers can call
//! `count_tokens` thousands of times per request without re-loading BPE
//! tables.

use crate::runtime::{ModelMetadata, RuntimeRegistry, TokenizerKind};
use crate::types::LlmMessage;
use once_cell::sync::Lazy;
use tiktoken_rs::CoreBPE;

/// First-use compatibility snapshot of the configured fallback character ratio.
/// Conversion/counting functions obtain current policy through central metadata.
pub static FALLBACK_CHARS_PER_TOKEN: Lazy<usize> = Lazy::new(|| {
    crate::runtime::shared_registry()
        .read()
        .fallback
        .character_ratio()
});

/// Lazily initialized supported BPE vocabulary.
static CL100K: Lazy<Option<CoreBPE>> = Lazy::new(|| tiktoken_rs::cl100k_base().ok());

/// Lazily initialized supported BPE vocabulary.
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

/// Source-compatible names for the same metadata-selected BPE adapter.
pub type AnthropicAdapter = TiktokenAdapter;
pub type GeminiAdapter = TiktokenAdapter;

pub struct FallbackCharAdapter {
    pub chars_per_token: usize,
}

impl TokenCounter for FallbackCharAdapter {
    fn count(&self, text: &str) -> usize {
        text.chars().count().div_ceil(self.chars_per_token.max(1))
    }
}

/// Compatibility test access to the metadata-selected encoder. Character-only
/// policy and unavailable BPE tables both return None.
#[cfg(test)]
fn encoder_for(model_id: &str) -> Option<&'static CoreBPE> {
    encoder_for_metadata(&crate::catalogue::metadata_for_model(model_id))
}

fn encoder_for_metadata(metadata: &ModelMetadata) -> Option<&'static CoreBPE> {
    match metadata.tokenizer {
        Some(TokenizerKind::O200kBase) => O200K.as_ref().or_else(|| CL100K.as_ref()),
        Some(TokenizerKind::Cl100kBase) => CL100K.as_ref(),
        Some(TokenizerKind::Characters) | None => None,
    }
}

pub(crate) fn bpe_counter_with_metadata(metadata: &ModelMetadata) -> Option<TiktokenAdapter> {
    encoder_for_metadata(metadata).map(|encoder| TiktokenAdapter { encoder })
}

pub fn resolve_token_counter_with_metadata(metadata: &ModelMetadata) -> Box<dyn TokenCounter> {
    if let Some(counter) = bpe_counter_with_metadata(metadata) {
        return Box::new(counter);
    }
    Box::new(FallbackCharAdapter {
        chars_per_token: metadata.character_ratio(),
    })
}

pub fn resolve_token_counter(model_id: &str) -> Box<dyn TokenCounter> {
    let metadata = crate::catalogue::metadata_for_model(model_id);
    resolve_token_counter_with_metadata(&metadata)
}

pub fn count_tokens_with_registry(registry: &RuntimeRegistry, model_id: &str, text: &str) -> usize {
    resolve_token_counter_with_metadata(&registry.metadata_for_id(model_id)).count(text)
}

/// Count using the configured vocabulary or an upward-rounded character estimate.
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
    let ratio = crate::runtime::shared_registry()
        .read()
        .fallback
        .character_ratio();
    tokens.saturating_mul(ratio)
}

pub fn chars_for_tokens_for_model(model_id: &str, tokens: usize) -> usize {
    let metadata = crate::catalogue::metadata_for_model(model_id);
    tokens.saturating_mul(metadata.character_ratio())
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
        let counter = resolve_token_counter(model_id);
        Self::turn_cost_with_counter(counter.as_ref(), turn)
    }

    fn turn_cost_with_counter(counter: &dyn TokenCounter, turn: &[LlmMessage]) -> usize {
        let mut total_tokens = 0usize;
        for m in turn {
            if !m.content.is_empty() {
                total_tokens = total_tokens.saturating_add(counter.count(&m.content));
            }
            if let Some(tcs) = m.tool_calls.as_deref() {
                for tc in tcs {
                    let json = tc.arguments.to_string();
                    if !json.is_empty() {
                        total_tokens = total_tokens.saturating_add(counter.count(&json));
                    }
                    if let Some(signature) = &tc.thought_signature {
                        total_tokens = total_tokens.saturating_add(counter.count(signature));
                    }
                }
            }
        }
        total_tokens
    }

    /// Compute the fallback character-based cost of a turn for backward compatibility
    pub fn turn_cost_fallback_chars(&self, turn: &[LlmMessage]) -> usize {
        turn.iter().fold(0usize, |total, m| {
            let tools =
                m.tool_calls
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .fold(0usize, |total, tc| {
                        total
                            .saturating_add(tc.arguments.to_string().chars().count())
                            .saturating_add(
                                tc.thought_signature
                                    .as_deref()
                                    .map_or(0, |s| s.chars().count()),
                            )
                    });
            total
                .saturating_add(m.content.chars().count())
                .saturating_add(tools)
        })
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
        let metadata = crate::catalogue::metadata_for_model(model_id);
        self.calculate_budget_with_metadata(
            &metadata,
            turns,
            system_overhead_tokens,
            max_context_chars,
        )
    }

    pub fn calculate_budget_with_registry(
        &self,
        registry: &RuntimeRegistry,
        model_id: &str,
        turns: &[Vec<LlmMessage>],
        system_overhead_tokens: usize,
        max_context_chars: usize,
    ) -> ContextBudgetResult {
        self.calculate_budget_with_metadata(
            &registry.metadata_for_id(model_id),
            turns,
            system_overhead_tokens,
            max_context_chars,
        )
    }

    fn calculate_budget_with_metadata(
        &self,
        metadata: &ModelMetadata,
        turns: &[Vec<LlmMessage>],
        system_overhead_tokens: usize,
        max_context_chars: usize,
    ) -> ContextBudgetResult {
        let counter = resolve_token_counter_with_metadata(metadata);
        let ratio = metadata.character_ratio();
        let system_overhead_chars = system_overhead_tokens.saturating_mul(ratio);
        let message_budget = max_context_chars.saturating_sub(system_overhead_chars);

        let mut selected: Vec<Vec<LlmMessage>> = Vec::new();
        let mut budget_used_chars: usize = 0;
        let mut total_tokens_used: usize = system_overhead_tokens;

        for turn in turns.iter().cloned().rev() {
            let tokens = Self::turn_cost_with_counter(counter.as_ref(), &turn);
            let fallback_chars = self.turn_cost_fallback_chars(&turn);

            let turn_chars = if tokens == 0 && fallback_chars > 0 {
                fallback_chars
            } else {
                tokens.saturating_mul(ratio)
            };

            if selected.is_empty() {
                // Always include the most recent turn regardless of size
                selected.push(turn);
                budget_used_chars = budget_used_chars.saturating_add(turn_chars);
                total_tokens_used = total_tokens_used.saturating_add(tokens);
            } else if budget_used_chars.saturating_add(turn_chars) <= message_budget {
                selected.push(turn);
                budget_used_chars = budget_used_chars.saturating_add(turn_chars);
                total_tokens_used = total_tokens_used.saturating_add(tokens);
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
            total_chars_used: budget_used_chars.saturating_add(system_overhead_chars),
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
