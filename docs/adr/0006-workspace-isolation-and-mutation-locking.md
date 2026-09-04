# ADR 6: Workspace Isolation and File Mutation Locking for Parallel Subagents

* **Status**: Proposed
* **Decided on**: 2026-06-25

## Context

When CADE runs multiple subagents concurrently under a parallel team execution flow, all subagents execute tool calls on the same host filesystem directory. 

This presents significant platform engineering risks:
1. **Race Conditions**: If multiple subagents attempt to modify the same file concurrently, they may clobber each other's edits, causing corrupt source states.
2. **Build Contention**: If one subagent initiates a test suite run (`cargo test`) while another subagent is in the middle of a partial file write, the build will fail or test in inconsistent states.
3. **Implicit Conflicts**: Subagents are unaware of concurrent file modifications being made by their peers, leading to logical code merge conflicts.

We need a design that guarantees workspace integrity and file-level write safety during concurrent multi-agent executions.

## Decision

We decided to implement a dual-layer workspace safety architecture:

### 1. File-Level Mutation Locking
CADE will introduce a centralized, thread-safe File Lock Manager. Any tool that mutates a file (such as `write_file`, `edit_file`, or `apply_patch`) must acquire an exclusive write-lock on the target file's absolute path before accessing the disk.
* Other agents requesting writes to the locked file are blocked until the lock is released or a timeout is reached.
* Read-only operations (`read_file`, `grep`) are allowed to proceed concurrently without holding locks.

### 2. Ephemeral Branch Sandboxing (Workspace Isolation)
For heavy parallel team executions, CADE will run subagents in **isolated directory branches** (clones of the workspace in ephemeral temp folders, or localized Git branches).
* Subagents apply mutations, compile, and run tests independently within their sandboxed workspace branches.
* Upon completion, CADE's parent coordinator aggregates all sandboxed branch modifications, runs a unified diff comparison, performs conflict resolution, and presents a cohesive "merge request" to the user for final approval before committing to the main directory.

## Consequences

### Positive (Pros)
* **Zero Disk Clobbering**: Eliminates the risk of file-corruption and concurrent write race conditions.
* **Isolated Compilation**: Subagents can run build/test validations independently without interference from peer agent modifications.
* **Safer Automation**: Users can review aggregated branch diffs and conflict-resolution results in a single, clean merge pass.

### Negative (Cons)
* **Storage and I/O Overhead**: Cloning large workspaces into ephemeral temp folders increases disk space usage and initial I/O startup times.
* **Lock Management Overhead**: File locks introduce a slight latency penalty on rapid successive writes.
