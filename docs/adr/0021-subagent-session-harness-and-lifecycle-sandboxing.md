# ADR-0021: SubagentSession Harness and Lifecycle Sandboxing

## Status

Accepted

## Context

In CADE, Subagents are spawned to execute isolated, token-efficient tasks (e.g. testing, refactoring, code review, or research).
Prior to this decision:
- The subagent execution logic in `crates/cade-server/src/server/api/run/subagent.rs` spanned ~1,900 lines of monolithic code.
- Workspace cloning (`IsolatedWorkspace`), tool filtering (`SubagentTools`), finish tool injection, token budgeting, and event streaming were directly intertwined with server HTTP endpoints.
- Temporary workspaces risked resource leakage on unexpected task cancellation or process panics.
- Subagent runners in CLI or embedded SDKs could not execute subagents without pulling in the entire Axum server execution infrastructure.

## Decision

We establish the **`SubagentSession`** deep module in `crates/cade-agent/src/subagents/session.rs` as the autonomous execution harness for all subagents:

1. **Crate Placement & Decoupling**:
   - `SubagentSession`, its configuration, and outcome models reside in `crates/cade-agent/src/subagents/session.rs`.
   - `crates/cade-server` consumes `SubagentSession` as a client, delegating all subagent execution loops to this harness.

2. **Interface Contract**:
   ```rust
   pub struct SubagentSession {
       pub session_id: String,
       pub config: SubagentConfig,
       pub workspace_guard: Option<IsolatedWorkspaceGuard>,
   }

   impl SubagentSession {
       pub fn new(config: SubagentConfig) -> Self;

       pub async fn run(
           &mut self,
           task: &str,
           cx: &mut SubagentExecutionContext,
       ) -> Result<SubagentOutcome, SubagentError>;
   }
   ```

3. **RAII Workspace Isolation (`IsolatedWorkspaceGuard`)**:
   - When an isolated workspace is required, the session creates an `IsolatedWorkspaceGuard`.
   - On clean completion with status `Done`, changes are atomically staged and merged back into the parent working tree.
   - On error, cancellation, or panic, the `Drop` implementation deletes temporary directories and cleans up worktrees automatically.

4. **Loop Convergence & Dual Budget Clamping**:
   - Injects a canonical `finish(status, summary)` tool.
   - Dual-guarded by `max_iters` (turn counter) and `max_tokens_budget` (cumulative token counter).
   - Halts immediately upon calling `finish`, returning structured outcome states:
     - `SubagentOutcome::Done { summary, tool_calls_count, token_usage }`
     - `SubagentOutcome::Blocked { reason, questions }`
     - `SubagentOutcome::Failed { error }`
     - `SubagentOutcome::Exhausted { reason }`

5. **Asynchronous Event Streaming & HITL Intercept**:
   - Emits real-time progress and output chunks over an async `SubagentEventEmitter` channel.
   - Supports Human-In-The-Loop (HITL) review through a dedicated approval channel when `human_review = true`.

## Consequences

### Positive
- **High Locality**: Workspace management, loop convergence, tool filtering, and token clamping are encapsulated in one place.
- **High Leverage**: Subagents can be executed uniformly across Server, CLI, TUI, and embedded SDK workflows.
- **Testability**: `SubagentSession` can be unit tested and integration tested in-memory without starting an HTTP daemon or mocking database tables.

### Negative
- Requires migrating existing `subagent.rs` HTTP routes in `cade-server` to call `SubagentSession::run`.
