# Specification: SubagentSession Harness & Lifecycle Sandboxing

**Status**: Draft  
**Version**: 1.0.0  
**Related ADR**: [ADR-0021](../../docs/adr/0021-subagent-session-harness-and-lifecycle-sandboxing.md)  
**Target Module**: `crates/cade-agent::subagents::session`  

---

## 1. Overview & Problem Statement

Subagents in CADE perform focused, ephemeral tasks (code analysis, test generation, refactoring, research) with isolated tool filters to prevent bloating the parent agent's context window. 

Currently, the orchestration logic in `crates/cade-server/src/server/api/run/subagent.rs` mixes HTTP endpoints, git worktree filesystem manipulation, tool schema filtering, loop iteration bounds, and event broadcasting in a single ~1,900 line file. This causes:
1. **Coupling**: Subagents cannot be executed headlessly or within the CLI/SDK without starting the Axum HTTP server.
2. **Resource Leakage**: Unhandled panics or client disconnections can leave orphaned temporary workspace directories or stale git worktrees.
3. **Runaway Loops**: Confused models emitting repetitive non-tool prose at the turn limit lack explicit convergence guarantees.

This specification formalizes the **`SubagentSession`** deep module: a standalone, reusable execution harness in `crates/cade-agent` that manages workspace lifecycles, loop convergence, tool sandboxing, and real-time event streaming.

---

## 2. Core Requirements

### 2.1 Decoupled Execution Harness
- `SubagentSession` MUST be implemented in `crates/cade-agent/src/subagents/session.rs`.
- MUST NOT depend on `axum`, HTTP routing types, or server daemon state.
- MUST accept an abstract `LlmProvider` and `CapabilityMesh` reference.

### 2.2 RAII Workspace Isolation (`IsolatedWorkspaceGuard`)
- When running in an isolated mode (`mode = "build"` with `worktree = true` or ephemeral directory cloning), the session MUST initialize an `IsolatedWorkspaceGuard`.
- On completion with status `Done`, changes staged in the temporary workspace MUST be atomically applied back to the parent repository.
- On error, panic, or cancellation (`Drop`), temporary directories and worktrees MUST be automatically unlinked and pruned without leaving filesystem artifacts.

### 2.3 Loop Convergence & Dual Budget Clamping
- The session MUST inject a canonical `finish(status, summary)` tool into the subagent's toolset.
- Execution MUST immediately halt when `finish` is invoked.
- Dual Guard Enforcers:
  1. `max_iters`: Hard iteration turn limit (default 20).
  2. `max_tokens_budget`: Cumulative prompt + completion token usage limit.
- If limits are reached without a `finish` call, the session MUST return `SubagentOutcome::Exhausted` rather than looping indefinitely or assuming success.

### 2.4 Real-time Event Streaming & HITL Approvals
- MUST support an asynchronous `SubagentEventEmitter` channel to stream events (`TurnStarted`, `ToolExecuting`, `Progress`, `ApprovalRequired`, `OutputChunk`, `Finished`).
- When `human_review = true` is set on a subagent configuration, mutating tool calls MUST pause execution and await user confirmation over an approval receiver.

---

## 3. Data Schemas & Contracts

### 3.1 Subagent Config & Context
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSessionConfig {
    pub subagent_id: String,
    pub parent_agent_id: String,
    pub mode: String, // "plan", "build", or custom subagent role
    pub model: Option<String>,
    pub max_iters: usize,
    pub max_tokens_budget: Option<usize>,
    pub isolated_workspace: bool,
    pub human_review: bool,
}
```

### 3.2 Structured Outcome
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SubagentOutcome {
    Done {
        summary: String,
        iterations: usize,
        tool_calls_count: usize,
        token_usage: usize,
    },
    Blocked {
        reason: String,
        questions: Vec<String>,
    },
    Failed {
        error: String,
    },
    Exhausted {
        reason: String,
        iterations: usize,
        tokens_used: usize,
    },
}
```

### 3.3 Event Telemetry
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SubagentEvent {
    TurnStarted { turn: usize, max_turns: usize },
    ToolExecuting { tool_call_id: String, tool_name: String, arguments: serde_json::Value },
    ToolCompleted { tool_call_id: String, tool_name: String, is_error: bool },
    Progress { percent: f64, message: Option<String> },
    ApprovalRequired { tool_name: String, arguments: serde_json::Value, approval_id: String },
    OutputChunk { text: String },
    Finished { outcome: SubagentOutcome },
}
```

---

## 4. User Stories & Scenarios

### US-1: Headless Fast Subagent Execution
**As a** CLI user or parent agent,  
**I want** to invoke a read-only research subagent with a 5-turn limit,  
**So that** it executes without starting an HTTP daemon, reports findings, and consumes minimal tokens.

### US-2: Isolated Code Refactoring with Auto-Cleanup
**As an** AI agent refactoring complex crates,  
**I want** my subagent to compile and test changes inside an isolated temporary worktree,  
**So that** if the subagent crashes or tests fail, my main working tree remains clean and untouched.

### US-3: Human-In-The-Loop Approval Gate
**As a** security-conscious developer,  
**I want** high-risk subagent mutations to trigger an `ApprovalRequired` event and await confirmation,  
**So that** destructive file edits or shell scripts cannot execute autonomously without oversight.

---

## 5. Acceptance Criteria & Test Matrix

| ID | Test Scenario | Expected Outcome |
|---|---|---|
| **AC-1** | Run subagent with `mode = "plan"` | Only read-only tools are exposed; mutating tools return an error or are omitted from schema. |
| **AC-2** | Subagent calls `finish(status="done", summary="All tests pass")` | Session terminates immediately with `SubagentOutcome::Done`. |
| **AC-3** | Subagent reaches `max_iters` without calling `finish` | Session terminates cleanly with `SubagentOutcome::Exhausted`. |
| **AC-4** | Subagent reaches `max_tokens_budget` | Session terminates immediately with `SubagentOutcome::Exhausted`. |
| **AC-5** | Session aborts / drops mid-task with `isolated_workspace = true` | Temporary directories and git worktrees are automatically unlinked and deleted on Drop. |
| **AC-6** | `human_review = true` on mutating tool | `ApprovalRequired` event emitted; execution pauses until approval or denial is transmitted. |
