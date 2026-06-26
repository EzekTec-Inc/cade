# ADR 13: Pluggable Polymorphic Token Counters

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

CADE relies on highly precise, provider-aware token counting to assemble context budgets, manage memory compaction limits, and calculate metrics. Previously, token counting inside `crates/cade-ai/src/tokenizer.rs` relied on a static `encoder_for` helper that selected between OpenAI `cl100k_base` and `o200k_base` encoders, falling back to a uniform character-division ratio when unavailable.

This lack of modularity made it incredibly difficult to add new specialized or custom token-counting behaviors (such as specialized API-based counting for Anthropic or Gemini, or specific local fallback strategies). It also tightly coupled the client and server interfaces to raw tiktoken BPE crates.

## Decision

We decided to implement a polymorphic, pluggable **`TokenCounter`** trait and corresponding adapter pipeline inside CADE's AI module (`crates/cade-ai/src/tokenizer.rs`):

1. **The Polmorphic Seam**: Defined the `TokenCounter` trait:
   ```rust
   pub trait TokenCounter: Send + Sync {
       fn count(&self, text: &str) -> usize;
   }
   ```
2. **Pluggable Adapters**: Created four distinct adapters implementing this trait:
   * **`TiktokenAdapter`**: Wraps native `tiktoken_rs::CoreBPE` (GPT-3.5/4/4o) for high-performance, local Byte-Pair Encoding counting.
   * **`AnthropicAdapter`**: Custom wrapper approximating Claude tokenizations (currently leveraging `cl100k_base` safely, leaving headroom).
   * **`GeminiAdapter`**: Custom wrapper approximating Gemini structures.
   * **`FallbackCharAdapter`**: A lightweight character-based division fallback.
3. **Dynamic Resolver**: Created `resolve_token_counter(model_id: &str) -> Box<dyn TokenCounter>` which automatically parses the active model ID prefix (e.g. `openai/`, `anthropic/`, `gemini/`) and resolves the optimal, specialized counter adapter.

Both client and server codebases now delegate count actions seamlessly via:
```rust
pub fn count_tokens(model_id: &str, text: &str) -> usize {
    let counter = resolve_token_counter(model_id);
    counter.count(text)
}
```

## Consequences

### Positive (Pros)
* **High Extensibility**: Adding a new AI provider (like Cohere, Mistral, or local LLaMA tokenizer models) simply requires writing a new struct that implements `TokenCounter` and registering it in the resolver.
* **Provider-Level Accuracy**: Prepares CADE for integrating true vendor-native tokenizer APIs safely without breaking backward compatibility or complicating the call surface.
* **Tighter Abstractions**: Fully encapsulates BPE loading and character fallbacks behind clean traits.

### Negative (Cons)
* **Allocation Overhead**: Dynamic dispatch (`Box<dyn TokenCounter>`) introduces a minor pointer redirection, though this is negligible compared to actual API latency.
