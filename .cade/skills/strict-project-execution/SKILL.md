---
id: strict-project-execution
name: strict-project-execution
description: Enforce a strict, low-risk workflow for software project execution with minimal changes, no guessing, and explicit audit logs.
---

# Strict Project Execution

Ensure tightly controlled implementation with strong safeguards and minimal-risk changes.

## Core Rules & Guardrails
*   **Minimal Changes**: Implement ONLY what is requested. Touch the fewest lines and files. Keep all changes reversible.
*   **Zero Guessing**: Do not guess or invent files, functions, variables, or configs. If facts are not verified, ask!
*   **Dependencies**: Do not add new dependencies to `Cargo.toml` without explicit user approval.
*   **Compatibility**: Never break public interfaces, APIs, routes, or serialization contracts.
*   **Quality**: Output copy-paste ready, syntactically correct, warning-free, and clippy-compliant code. No unwrap/expect.

## Required Tooling (For Rust sessions)
*   Use **Context7** for API reference search.
*   Use **cade-rag** for indexing and reading semantic memory.
*   Use **serena** for ALL edits/reads on `.rs` and `.lua` files (generic tools like edit_file/write_file are blocked by project hooks).

## Audit Trail Requirement
When modifying code, always call the `finish_task(summary, reason)` tool at the very end of the task. This automatically logs a timestamped entry with `git status` output directly to `CADE_AUDIT.md` (local-only, gitignored).

## SQLite Time Policy
If SQLite time storage is in scope, consistently store timestamps in UTC as either:
1.  `TEXT` using ISO 8601 (RFC 3339).
2.  `INTEGER` using Unix microseconds as `i64` with a typed wrapper.
