# ADR 10: Decoupled Async Subagent Executor Trait Seam

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

CADE features a server-side subagent and team parallel execution pipeline (`crates/cade-server/src/server/api/run/subagent.rs`). Previously, this loop was governed by a tight concrete struct `SubagentExecutor` that was coupled to static function signatures and global state pointers. 

To improve codebase health and fulfill **Candidate 1: Deepening the Subagent & Team Orchestrator**, we need to decouple this execution logic. This creates a clean, pluggable, and mockable **seam** that isolates subagent lifecycle states (database writes, file locking, and LLM completions) from downstream API handlers and allows standalone testing.

## Decision

We decided to refactor the subagent execution model by elevating `SubagentExecutor` into an asynchronous, trait-based seam:

1. **The Abstract Seam**: Defined the `SubagentExecutor` trait utilizing the `#[async_trait]` macro:
   ```rust
   #[async_trait]
   pub trait SubagentExecutor: Send + Sync {
       async fn execute(self: Box<Self>, args: &serde_json::Value) -> ToolResult;
   }
   ```
2. **Concrete Implementation**: Created a concrete **`CadeSubagentExecutor`** struct that implements this trait, keeping its internal state (including the SSE event emitter and parent agent ID) fully encapsulated:
   ```rust
   pub struct CadeSubagentExecutor {
       pub state: AppState,
       pub parent_agent_id: String,
       pub tool_call_id: String,
       pub emitter: Box<dyn SubagentEventEmitter>,
   }
   ```
3. **Consumption Interface**: The standard Axum endpoint handler `handle_run_subagent_tool` now simply instantiates `Box<dyn SubagentExecutor>` and delegates tool evaluation dynamically, completely decoupling the HTTP/tool boundary from the core execution engine.

## Consequences

### Positive (Pros)
* **High Modularity**: Converts a previously coupled monolithic execution path into a clean, polymorphic seam.
* **Stand-Alone Testability**: Enables writing mock executors and hock-ins to verify routing and loop iteration behaviors without launching full Axum or database server dependencies.
* **Extensibility**: Lay the foundations for alternative subagent runtimes (e.g. running subagents entirely inside containerized environments or remote servers).

### Negative (Cons)
* **Allocation Overhead**: Consuming the executor via `Box<Self>` introduces a minor heap allocation per subagent task, though this overhead is mathematically negligible compared to LLM API latency.
