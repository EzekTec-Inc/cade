# ADR 9: Adaptive Typewriter Governor for TUI Streaming

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

CADE's Terminal User Interface (`cade-tui`) features a smooth, character-by-character typewriter reveal animation for streaming incoming text. While this provides a pleasing conversational flow for standard assistant prose, it creates significant performance bottlenecks when handling high-velocity tool outputs (such as massive compiler logs, parallel subagent event traces, or raw directory dumps).

A constant, slow reveal rate causes the typewriter to fall significantly behind the real-time server stream. This introduces artificial buffer lag, blocks prompt interaction, and forces the user to wait unnecessarily.

## Decision

We decided to implement a **Velocity-Based Adaptive Typewriter Governor** inside `TuiApp`'s draw loop (`crates/cade-tui/src/app/mod.rs`):

1. **Backlog Evaluation**: On every redraw frame, the typewriter evaluates the distance (`behind`) between the fully received text and the currently revealed character count.
2. **Multi-Stage Acceleration**:
   * **Standard Pace (0 - 50 characters behind)**: Animates smoothly at a comfortable rate of `8` characters per tick (~160 chars/second).
   * **Moderate Catch-up (51 - 150 characters behind)**: Accelerates to `20` characters per tick.
   * **Rapid Catch-up (151 - 500 characters behind)**: Executes a swift `behind / 2` reveal rate to catch up in a few frames.
   * **Bypass/Snap Spike (500+ characters behind)**: Completely bypasses the typing animation, **snapping instantly** to the end of the buffer. This ensures that large outputs (like compile logs or file dumps) are displayed instantaneously without lag or screen jitter.

3. **Locked-Step Viewport Following**:
   The autoscroll `follow` mechanism stays locked to the bottom of the *revealed* content height rather than jumping ahead to the unrevealed buffer end, maintaining visual synchronization.

## Consequences

### Positive (Pros)
* **Sluggishness Protection**: Completely eliminates artificial latency during massive data streams while preserving the aesthetic value of standard chat typing.
* **Flicker Reduction**: Snapping huge blocks instantly prevents rapid, successive window reflows and visual scroll jitter.
* **Resource Preservation**: Drops redraw cycles early once caught up, reducing CPU rendering cycles under peak parallel logging workloads.

### Negative (Cons)
* **Transition Abruptness**: Transitioning instantly from a typing state to an instantly popped text block can occasionally feel slightly abrupt, though it represents a mathematically necessary trade-off for performance.
