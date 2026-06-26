# ADR 4: Adaptive Memory Retention and Archiving Based on Model Capabilities

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

CADE employs a tiered memory system where memory blocks are automatically archived to SQLite after 80 idle turns, and unused short-term/archived memories decay in confidence by 5% every 20 idle turns.

However, LLM models vary drastically in their maximum context window sizes (e.g., local models may have 8k tokens, whereas commercial models like Gemini 1.5 Pro support up to 2,000k tokens). A static 80-turn archiving and decay threshold is highly inefficient:
1. On tight context windows, it risks overflowing the budget before 80 turns elapse.
2. On massive context windows, it prematurely archives valuable in-context information, forcing unnecessary and latency-inducing search queries.

## Decision

We decided to replace the hardcoded 80-turn archiving and decay parameters with an **Adaptive Memory Retention** scheduler. The thresholds for memory archiving (`ARCHIVE_IDLE_TURNS`) and confidence decay rates will be computed dynamically at runtime based on the active model's context window limit.

### Archiving Scale Formula
The archiving idle threshold will scale proportionally with the active model's context length:

$$\text{Archive Idle Turns} = \text{Clamp}\left(\frac{\text{Context Window (Tokens)}}{2500}, \ \text{min} = 15, \ \text{max} = 500\right)$$

* For a local **8k** token model: memory blocks are archived after **15 idle turns** to protect the context window.
* For a **200k** token model (e.g., Claude): memory blocks are archived after **80 idle turns** (matching the previous default).
* For a **2,000k** token model (e.g., Gemini): memory blocks are retained for up to **500 idle turns** to leverage massive context recall.

The confidence decay rate will similarly adapt, decaying slower on large-context models and faster on restricted-context models to trigger faster consolidation.

## Consequences

### Positive (Pros)
* **Optimal Context Utilization**: Fully leverages the strength of long-context models without manual configuration.
* **Robust Local Performance**: Prevents out-of-memory/context overflow crashes when running lightweight local models.
* **Reduced Latency**: Reduces the frequency of expensive `search_memory` roundtrips on models that can easily hold the facts in-context.

### Negative (Cons)
* **Variable Prompt Behavior**: The active system prompt's composition becomes more dynamic and variable, which may make prompt debugging slightly more complex across different backends.
