# ADR 16: Server-Driven TUI Buffer Compaction and Layout Caching

* **Status**: Accepted
* **Decided on**: 2026-07-04

## Context

As conversational histories in `cade-tui` grow during long-running developer sessions, the cost of repeatedly traversing, wrapping, and laying out several thousand lines of verbose tool outputs scales linearly. 

We must address two critical challenges:
1. **Performance Bottleneck**: Repeated text wrapping and rendering of large historical logs on every frame redraw.
2. **State Consistency**: Ensuring that the client-side TUI's visible message buffer matches the exact context state remembered by the LLM (so the user and the model share a single source of truth).

## Decision

We decided to implement a dual strategy of **Local Layout Caching** and **Server-Driven Buffer Compaction**:

### 1. Server-Driven TUI Buffer Compaction
Instead of implementing complex, client-side, ad-hoc truncation logic in `cade-tui`, the visible message history compaction will be strictly driven by the server's background **Consolidation/Compaction** process:
* When the server near-exhausts the context window, it runs compaction, compresses older conversational history, and broadcasts a compacted message list.
* The TUI client naturally consumes this updated history, discarding old, verbose `RenderLine` entries and replacing them with a single consolidated/summary line.
* This guarantees absolute alignment between the LLM's active memory and the user's terminal viewport.

### 2. Line-Level and Layout-Segment Caching (`PreparedCache`)
To eliminate redundant CPU cycles on frame redraws, we will cache pre-wrapped text lines and pre-calculated layout dimensions:
* Extend `PreparedCache` to cache individual layout heights on a per-timeline-item basis rather than only caching the global timeline state.
* If a timeline item's internal content and terminal width have not changed, reuse its pre-wrapped layout and visual rows during redrawing, dropping frame-render cost from $O(N)$ text-wrapping calculations to $O(1)$ lookups.

## Consequences

### Positive (Pros)
* **Single Source of Truth**: Eliminates desynchronization between what the model remember (context window) and what the user sees in the TUI.
* **Peak Performance**: Layout-level caching makes redrawing thousands of historical chat lines extremely cheap and smooth, even during rapid mouse scrolling or typewriter updates.
* **Code Simplicity**: Avoids writing intricate, localized chat-pruning state machines in the TUI client.

### Negative (Cons)
* **Cache Invalidation Complexity**: Requires robust cache invalidation keys when terminal width changes or individual timeline elements are toggled (folded/expanded).
