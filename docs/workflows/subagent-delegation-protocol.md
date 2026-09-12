# Subagent Delegation & Verification Protocol

## Purpose
Standardize the end-to-end lifecycle for delegating complex, compute-intensive, or multi-file tasks from a primary supervisor agent to autonomous, specialized, or parallel subagents. Enforce strict sandboxing, recursion controls, human-in-the-loop steering, automated verification gates, and atomic result merging to prevent context pollution and workspace corruption.

## Trigger
- **Run when**:
  - A task requires reading/searching across >5 files or consuming >15k context tokens (e.g., deep exploration, codebase-wide audits).
  - Multiple independent tasks can be evaluated concurrently (e.g., parallel test generation, multi-crate doc updates, multi-file linting).
  - A task involves high-risk code refactoring or isolated prototyping requiring git worktree sandboxing.
  - An independent, unbiased review or specialized persona is required (e.g., security auditor, domain modeler, performance profiler).
- **Do not run when**:
  - The task is a single-step, low-context query or trivial edit (<= 2 files).
  - Current nesting depth is >= `CADE_SUBAGENT_MAX_DEPTH` (default 3).
  - Global subagent concurrency is saturated (`CADE_MAX_SUBAGENTS`, default 4).
  - The active session lacks `Capability::Agentic`.

## Roles
| Role | Responsibilities |
|---|---|
| **Supervisor (Parent Agent)** | Deconstructs user requests, bounds task contracts, specifies budgets and verification gates, monitors execution telemetry, audits outputs, and commits merged results. |
| **Subagent (Worker/Specialist)** | Executes focused task in an isolated context window; operates within tool filters and token/turn budgets; invokes `finish(status, summary)` upon completion. |
| **Verification Gatekeeper (System/Test Harness)** | Executes automated test commands (`test_command`), runs linters, and checks diff integrity in isolated worktrees before reporting back to the supervisor. |
| **Human Reviewer (User)** | Grants or denies tool approval requests (`/approve`, `/deny <id> [feedback]`); provides dynamic mid-flight steering (`/steer <id> <message>`). |

## Inputs
- **Task Contract**: Explicit task prompt with defined scope, constraints, and non-goals (Required).
- **Execution Mode**: `plan` (read-only exploration/scouting) or `build` (full mutating access) (Required).
- **Budget Limits**: `max_tokens_budget` (tokens) and turn limits (`max_iters`, default 20) (Required).
- **Verification Command**: Automated validation test command (e.g., `cargo test -p <crate>`) (Optional, recommended for `build` mode).
- **Isolation Strategy**: In-tree execution vs. isolated git worktree (`worktree = true`) (Required).
- **Model Override**: Fast model (e.g., `claude-3-5-haiku-latest`, `gemini-2.5-pro`) for exploration or frontier model for deep reasoning (Optional).

## Outputs
- **Subagent Outcome**: Structured execution result (`Done`, `Blocked`, `Failed`, or `Exhausted`) containing summary and modified file list.
- **Verification Report**: Test execution logs, compiler diagnostics, and unified git diff.
- **Consolidated Workspace**: Atomically merged git working tree state, verified against project standards.
- **Updated Memory**: Promoted domain discoveries and architectural facts merged into parent persistent memory.

## Workflow

### Phase 1: Delegation Planning & Contract Formulation
1. **Deconstruct Task** — Owner: Supervisor
   - Action: Analyze user goal, verify `Capability::Agentic` is active, and assess depth against `CADE_SUBAGENT_MAX_DEPTH`. Deconstruct work into vertical, independent slices.
   - Produces: Formal task contract with clear boundaries.
   - Done when: Scope, inputs, outputs, and acceptance criteria are explicitly documented.

2. **Establish Pre-Delegation Checkpoint** — Owner: Supervisor
   - Action: Create a working-tree snapshot via `create_checkpoint` before spawning any mutating subagent.
   - Produces: Checkpoint record with ID and restore point.
   - Done when: Checkpoint ID is registered in supervisor session state.

3. **Configure Subagent Harness** — Owner: Supervisor
   - Action: Formulate invocation parameters (`run_subagent` or `run_parallel_subagents`):
     - Set `mode: "plan"` for read-only research; `mode: "build"` for edits.
     - Set `worktree: true` if modifying files that could conflict with parent workspace.
     - Attach `test_command` enforcing automated pass/fail verification.
     - Set `max_tokens_budget` to prevent runaway consumption.
   - Produces: Validated subagent invocation payload.
   - Done when: All guard parameters (mode, budget, isolation, test command) are populated.

### Phase 2: Autonomous Execution & Telemetry
4. **Spawn Execution** — Owner: Supervisor / Subagent Harness
   - Action: Dispatch subagent task asynchronously or synchronously.
   - Produces: Active subagent process, tracker card in UI, and event stream (`TurnStarted`, `ToolExecuting`, `Progress`).
   - Done when: Subagent ID is active in CADE telemetry.

5. **Monitor & Steer (Interactive)** — Owner: Human Reviewer / Supervisor
   - Action: Monitor execution card. If subagent requires mutating tool permission (`human_review = true`), review approval prompt.
     - To approve: run `/approve <id>`.
     - To deny with correction: run `/deny <id> <feedback>`.
     - To steer trajectory: run `/steer <id> <guidance>`.
   - Produces: Steered execution path or graceful recovery.
   - Done when: Subagent resolves all approval/steering events without stagnation.

### Phase 3: Verification & Quality Auditing
6. **Execute Automated Verification Gate** — Owner: Verification Gatekeeper
   - Action: On subagent completion, automatically execute the defined `test_command` inside the isolated context/worktree.
   - Produces: Test execution status code, stdout/stderr.
   - Done when: Harness verifies zero test failures and clean exit code.

7. **Audit Diff & Artifacts** — Owner: Supervisor
   - Action: Inspect the subagent's returned summary, list of modified files, and git diff. Verify no unintended files or formatting regressions were introduced.
   - Produces: Audit verdict (Accept / Reject / Rework).
   - Done when: All modified lines are verified to strictly adhere to the initial task contract.

### Phase 4: Integration & Memory Synchronization
8. **Merge Worktree & Update Memory** — Owner: Supervisor
   - Action: If worktree isolation was used, merge changes atomically back to `main`. Extract persistent facts/decisions and record via `update_memory_typed`.
   - Produces: Merged working tree and synchronized memory blocks.
   - Done when: Working tree is coherent, clean, and memory is updated.

9. **Workspace Re-Indexing & Completion** — Owner: Supervisor
   - Action: Trigger `cade-rag-mcp__index_workspace` to refresh semantic embeddings of newly integrated files. Report consolidated outcome to user.
   - Produces: Up-to-date RAG SQLite database and user summary.
   - Done when: RAG indexer completes with zero errors.

## Gates
| Gate | Owner | Pass Criteria | Fail Action |
|---|---|---|---|
| **G1: Pre-Spawn Safety Gate** | Supervisor | Session has `Agentic` capability; nesting depth < 3; concurrency < 4; dirty working tree snapshotted via checkpoint. | Halt delegation. Fall back to in-process single-turn execution or prompt user. |
| **G2: Budget & Turn Gate** | Subagent Harness | Execution completes within `max_tokens_budget` and `max_iters` (<= 20 turns). | Force terminate with `SubagentOutcome::Exhausted`. Roll back worktree; notify supervisor. |
| **G3: Automated Test Gate** | Verification Gatekeeper | Configured `test_command` returns exit code 0 with zero test/compile errors. | Reject subagent result (`SubagentOutcome::Failed`). Do not merge diff; request rework or halt. |
| **G4: Security & Invariant Gate** | Supervisor | No hardcoded credentials; no `.unwrap()` in production Rust paths; `#![forbid(unsafe_code)]` preserved; git history untouched. | Revert diff via checkpoint snapshot; flag violation to human reviewer. |
| **G5: Final Integration Gate** | Supervisor | `cargo check` and `cargo fmt --check` pass cleanly after worktree merge. RAG workspace re-indexed. | Run auto-formatter or rollback merge via pre-delegation checkpoint. |

## Failure Handling
| Condition | Action | Resume / Halt |
|---|---|---|
| **Subagent Token/Turn Exhaustion** | If subagent hits budget without calling `finish`, discard uncommitted worktree state. Checkpoint remains clean. | Halt subagent. Supervisor re-frames task into smaller vertical slices and re-spawns. |
| **Automated Test Failure (`test_command`)** | Capture test failure logs. If iterations remain, feed compiler/test stderr back to subagent via `steer`. If retries exhausted, abort. | Resume subagent with error feedback (up to 2 retries); else Halt and revert worktree. |
| **Tool Execution Blocked by Hook** | Inspect hook diagnostic (`[Blocked by hook: <reason>]`). Address root violation (e.g., path traversal, protected file). | Resume subagent with corrected arguments; do not attempt bypass. |
| **Worktree Merge Conflict** | If concurrent subagents modify overlapping files, abort automatic merge of conflicting branch. | Halt merge. Re-run conflicting subagent sequentially against updated base. |
| **Doom-Loop Detection** | Subagent stagnation detector triggers on repeated tool calls. System intervention prompts strategy rewrite. | Resume with memory re-write (`update_memory(label='active_goal')`). If loop persists, terminate. |
| **User Denies Tool Action (`/deny`)** | Subagent receives formatted denial feedback as a system intervention message. | Resume subagent immediately with revised plan adhering to user guidance. |

## Audit Log
Record the following trace events in the session event log and persistent memory:
- `SubagentSpawned`: Subagent ID, mode, assigned task, target files, budget, worktree path.
- `ApprovalIntervention`: Tool name, arguments, user decision (`Approved`/`Denied`), denial feedback string.
- `VerificationResult`: `test_command` string, execution duration, exit code, test pass count.
- `MergeExecuted`: Git commit SHA or worktree merge commit, list of modified files.
- `SubagentClosed`: Final outcome (`Done`, `Failed`, `Exhausted`), tokens consumed, wall-clock duration.
