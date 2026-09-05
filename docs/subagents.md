# Subagents

A **subagent** is a CADE agent the main agent can spawn programmatically
to handle isolated work. Subagents have their own context, their own
tools (filtered subset of the parent's), and report only their final
answer back — keeping the main agent's context window clean.

## When to use one

- **Deep codebase exploration** — searching, reading many files
- **Large file rewrites** — concentrated edits on one component
- **Code review** — independent assessment with no prior bias
- **Background memory maintenance** — reflection, summarisation
- **Long-running tasks** — anything > a few minutes that doesn't need
  user interaction

## Spawning one (from the LLM)

```
run_subagent(
  mode="worker",              # which subagent definition to use; matches by name
  prompt="<task>",
  description="<short label shown in TUI>",
  model="anthropic/claude-haiku-4-5",   # optional override
  background=false,            # spawn detached; result delivered next turn
  test_command="cargo test"    # optional verification command
)
```

Returns the subagent's final assistant message. Intermediate text and
tool calls are **not** streamed back to the parent — only the result.

### Selection by `mode`

The `mode` argument is matched against the discovered subagent
definitions (built-in + global `~/.cade/subagents/` + project
`.cade/subagents/`).  Resolution order:

1. **Exact name match** — `mode="rust-dev-worker"` selects
   `~/.cade/subagents/rust-dev-worker.md` if it exists.
2. **Fallback to `worker`** — when `mode` doesn't match any definition
   (e.g. legacy callers passing `mode="build"` or `mode="plan"`), the
   built-in `worker` is used.
3. **Default prompt** — only if neither the named def nor `worker` are
   present, an inline default system prompt runs.

The optional `agent_id` argument is independent: it deploys an existing
**stateful agent** (a server-stored agent row) as the subagent, bypassing
the definition lookup entirely.

## Built-in subagents

Defined in `crates/cade-agent/src/subagents/mod.rs::builtin_subagents`.

| Name | Tools | What it does |
|---|---|---|
| `worker` | All | Highly capable unified worker — explore, plan, implement, review |
| `reflection` | `update_memory`, `read_file`, `glob` | Background memory maintenance |
| `recall` | Read-only | Search past conversations and files for context |

Discover all visible subagents (built-in + global + project):

```bash
/subagents
```

## Custom subagents

Drop a Markdown file into `~/.cade/subagents/` (global) or
`.cade/subagents/` (project):

```markdown
---
name: bug-hunter
description: Inspect a stack trace and find the root cause
model: anthropic/claude-sonnet-4-5
tools:
  - read_file
  - glob
  - grep
---

You are a bug-hunting agent. Given a stack trace and a workspace,
identify the root cause...
```

Frontmatter fields:

- `name` (required) — id used in `run_subagent(agent_id=...)`
- `description` (required) — shown in `/subagents`
- `model` (optional) — override the parent's model
- `tools` (optional) — `all`, `readonly`, or a specific list

Same-name conflict: project > global > built-in.

## Defence layers (recursion safety)

CADE caps subagent runaway via six defences:

1. **Depth cap** — `CADE_SUBAGENT_MAX_DEPTH` (default 3). Every nested
   `run_subagent` increments depth; over the cap, the call refuses
   before any LLM is hit.
2. **Tool-list filter** — subagents never see `run_subagent` in their
   own toolset (so they cannot recurse via the tool).
3. **Re-entry guard** — depth is bumped at each level even if the tool
   filter is bypassed.
4. **Global semaphore** — `CADE_MAX_SUBAGENTS` (default 4) caps
   concurrent subagent runs across the whole server.
5. **Per-level iteration cap** — `CADE_SUBAGENT_MAX_ITERS` (default 10)
   limits agentic-loop iterations inside a single subagent.
6. **DB-pollution watchdog** — server tests verify subagent runs do not
   leak agent rows or message history into the parent's DB.

All six are tunable via env vars; see [configuration.md](configuration.md).

## Memory & state

A subagent runs securely via the **`SubagentSession`** harness (ADR-0021) in `crates/cade-agent`, decoupled from the HTTP server daemon:

- **Autonomous Execution Harness:** `SubagentSession` manages prompt-and-tool loops to convergence, injects a canonical `finish(status, summary, questions)` tool, and returns a structured `SubagentOutcome` (`Done`, `Blocked`, `Failed`, `Exhausted`).
- **Dual Budget Bounds:** Execution is dual-clamped by turn limit (`max_iters`, default 20) and cumulative token consumption (`max_tokens_budget`). If boundaries are reached without an explicit `finish`, execution halts cleanly with `SubagentOutcome::Exhausted`.
- **RAII Workspace Isolation (`IsolatedWorkspaceGuard`):** When isolated execution is active, the session runs inside an ephemeral cloned directory or git worktree. On completion with `Done`, modified files are committed and merged back atomically; on error, cancellation, or panic, the `Drop` implementation unlinks and deletes temporary directories automatically with zero filesystem residue.
- **Hierarchical Memory Mounting:** During subagent initialization, CADE automatically copies and mounts the parent agent's core memory blocks (`project`, `persona`, and `active_goal`) directly into the subagent's sandboxed namespace.
- **Real-Time Telemetry & Event Streaming:** Structured events (`TurnStarted`, `ToolExecuting`, `Progress`, `ApprovalRequired`, `OutputChunk`, `Finished`) stream over an asynchronous `SubagentEventEmitter` channel to live TUI inspectors and server SSE clients without blocking execution turns.
- **Human-In-The-Loop Review Gates:** When `human_review = true` is configured, mutating tool invocations pause and emit an `ApprovalRequired` event, awaiting interactive confirmation before proceeding.
- **Active Memory Self-Correction (Doom-Loop Protection):** When the rolling `DoomLoopDetector` state-machine detects tool stagnation (loops), the system intervention explicitly instructs the subagent to use `update_memory(label='active_goal', value=...)` to rewrite its strategy before running further tools.
- **Smart Memory Merge:** When the subagent completes, discovered facts are intelligently merged back into the parent agent's context, preserving memory taxonomy and confidence levels.

## Subagent Steering & Human-in-the-Loop Redirection

To maintain strict, safe bounds over autonomous execution, CADE provides a powerful **Human-in-the-Loop Steering and Redirection protocol**:

### 1. Denial Feedback (`/deny <id> [feedback...]`)
When a background subagent requests permission to execute a potentially destructive or unsafe tool (like `bash` or `write_file`), it suspends its thread asynchronously and waits. 
*   If you wish to reject the tool call, you can run `/deny <id>`.
*   Additionally, you can supply **custom redirection feedback** directly with your denial:
    ```bash
    /deny app-1234 use the existing standard library instead of creating a new file!
    ```
*   The CADE server attaches this feedback to the approval record. When the subagent resumes, instead of receiving a generic `PermissionDenied` error, it receives your exact, formatted feedback as a **system intervention message** (e.g. `[System Note: Tool call 'write_file' was denied by the user with feedback: "..."]`). The subagent is instructed to immediately revise its plan, preserving execution safety.

### 2. Dynamic Steering (`/steer <subagent_id> <message>`)
If a subagent is running a long-horizon task or is suspended awaiting permission, you can inject active instructions to dynamically steer its execution at any time:
```bash
/steer sa_e83bc4 please stop compiling everything in release mode, run cargo check instead!
```
*   The server intercepts this command via the `/subagents/:id/steer` endpoint, pushes it to the subagent's active instruction stream, and triggers a dynamic LLM redirection turn.
*   This allows you to change constraints, correct early mistakes, or redirect subagents mid-flight without having to cancel, kill, or restart the entire task.

## Isolation & Harness Execution (`AgentHarness`)

The **`AgentHarness`** module (`crates/cade-agent/src/subagents/harness.rs`) provides unified isolation and execution policies:

| Policy | Mechanism | Use Case |
|---|---|---|
| `IsolationPolicy::InProcess` | Runs in-process with shared host environment | Fast read-only tasks & local worker turns |
| `IsolationPolicy::ReadOnly` | Restricts all filesystem mutation tools | Safe exploration and inspection passes |
| `IsolationPolicy::WorktreeBranch` | Ephemeral git worktree with dedicated branch | Parallel code refactoring with isolated rollback |
| `IsolationPolicy::VirtualSandbox` | In-memory copy-on-write filesystem overlay | Sensitive file transforms without host touch |
| `IsolationPolicy::Docker` | Isolated container instance | Untrusted scripts and build sandboxing |

The harness handles RAII teardown, signal cancellation, and automatic background task reaping.

## Background runs

Set `background: true` to detach. The call returns immediately with a
run id; query progress via `GET /v1/runs/:id` or re-attach with
`/v1/runs/:id/stream`.

## Performance tips

- **Pick a cheaper model** for read-heavy work (`worker` defaults to the
  parent's model; override per-call for cost).
- **Filter tools** when authoring custom subagents — fewer tools means
  smaller schemas in the prompt.
- **Use `description`** generously — shown in TUI cards, helps you
  monitor running subagents at a glance.
