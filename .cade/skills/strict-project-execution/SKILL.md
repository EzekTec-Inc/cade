---
name: strict-project-execution
description: Enforce a strict, low-risk workflow for software project execution with minimal changes, no guessing, no unauthorized dependencies, and explicit stop/approval rules. Use when the user wants highly controlled coding help, especially for Rust projects, session policies, patching existing codebases, or exact-scope execution.
---

# Strict Project Execution

Use this skill when the user wants tightly controlled implementation with strong safeguards and minimal-risk changes.

## Core Mode

Operate with these defaults unless the user explicitly relaxes them:

- Implement only what is requested
- Make the smallest viable change
- Preserve existing structure, style, and behavior
- Do not guess missing facts
- Do not refactor, optimize, redesign, or generalize unless asked
- Do not add dependencies, files, or abstractions without approval
- Do not modify unrelated code
- Keep all changes reversible

If a requested change would violate any of the above, stop, explain why, and ask for approval.

## Required Tooling

For Rust coding sessions:

- Use **Context7** mcp server for documentation and API reference lookups when documentation is needed
- Use **cade-rag** mcp server for context and memory management, including preserving session facts, constraints, and known vs unknown information across the conversation

Tool use never overrides the rules in this skill.

If either required tool is unavailable:

1. Stop
2. State that the tooling requirement cannot be met
3. Ask the user to provide the missing docs, snippets, or project context manually

Ignore the Context7/cade-rag requirement for Neovim configuration work.

## Execution Rules

### Do

- Follow the user’s instructions literally
- Preserve backward compatibility unless explicitly told otherwise
- Match the project’s existing patterns
- Keep surface area minimal
- Maintain correct imports, types, and error handling
- Ask for clarification when required information is missing

### Do Not

- Invent files, functions, versions, config, env vars, or architecture
- Assume framework version, runtime, OS, deployment model, or crate behavior
- Reformat large sections of code
- Rename symbols without instruction
- Update crates by default
- Change `Cargo.toml` or dependency manifests without explicit approval

## Missing Information Rule

If safe execution requires facts that are not verified, say:

> I do not have enough verified information to proceed safely.

Then request only the missing details. Do not fill gaps with assumptions.

## Minimal Change Policy

All changes must:

- Touch the fewest files possible
- Modify the fewest lines possible
- Avoid cross-cutting edits
- Avoid hidden behavior changes

No silent refactors. No cleanup passes. No unrelated fixes.

## Dependency Policy

Do not add dependencies unless the user explicitly approves it.

If a new dependency appears necessary:

1. Explain why
2. Offer a no-new-dependency alternative
3. Ask for approval
4. Wait

## Compatibility Policy

Unless the user approves a breaking change, do not break:

- Public interfaces
- Function signatures
- API contracts
- Data models
- Serialization formats
- Routes
- Expected return types

If a breaking change is unavoidable, explain:

1. Why it is required
2. Exact impact
3. Migration steps

Then wait for approval.

## Code Quality Requirements

Generated code must be:

- Complete and copy-paste ready
- Syntactically correct
- Type-correct
- Consistent with the project’s async/sync model
- Consistent with existing error-handling patterns
- Free of deprecated APIs when version information is verified
- Free of `unwrap` / `expect` unless the codebase already uses them intentionally

## Edge Handling

Consider only the edge cases needed for the requested scope:

- `None` / null
- Empty input
- Invalid input
- Error propagation
- Concurrency, if relevant
- Async cancellation, if relevant
- Type safety

Do not expand scope to add extra robustness unless required. If proper handling would materially expand scope, explain the tradeoff and ask first.

## Change Log Requirement

For project modifications, call the `finish_task` tool when done.

`finish_task(summary, reason)` automatically:
- Records the current `git status` (modified files)
- Appends a timestamped audit entry to `CADE_AUDIT.md` (local-only, gitignored)

Rules:
- Call `finish_task` exactly once at the end of the modification.
- Provide a clear `summary` and `reason` as arguments.
- If code is deleted, justify it in the `reason`.
- `CADE_AUDIT.md` is gitignored — it serves as a local development log, not a committed artifact.

## Conflict Protocol

Stop and ask for clarification if:

- The instructions conflict with project state
- The request would require guessing
- Required information is missing
- The request conflicts with this workflow

Do not proceed under uncertainty.

## Rust-Specific Guidance

Apply these only for Rust work unless the user says otherwise.

### Session Start

Before changing code, request what is needed for verified work:

- Relevant files
- Repo structure or file tree
- Crate versions when version-specific behavior matters
- Environment details only if actually needed

If these are not provided, do not proceed with code changes.

### Rust Project Defaults

- Preserve existing crate choices and versions
- Use the project’s existing `Result` alias and error taxonomy
- Keep IDs strongly typed if the project already uses typed IDs
- Avoid raw `String` / `&str` IDs across boundaries when typed IDs are the established pattern

### Comments and Layout

- Keep comments brief unless more detail is necessary
- Skip comments when names are already clear
- Break long blocks into smaller readable units with spacing
- Add module-level comments only where useful
- Use region or grouping comments only in large files

### rust10x References

If the user says the project follows `rust10x` or provides those docs, follow them.

If the user references rust10x material but does not provide the relevant content, say:

> Not available in the provided input — please paste the relevant sections.

Do not assume the contents of external links.

The rust10x directory location on this machines is at: `~/.aipack-base/pack/installed/pro/rust10x/` 

### SQLite Time Policy

Apply this only when SQLite time storage is in scope.

Default rules:

- Store timestamps in UTC
- Use one storage format consistently across the app

Approved formats:

1. `TEXT` using ISO 8601 / RFC 3339
2. `INTEGER` using Unix microseconds as `i64`

If using integer microseconds, prefer a typed wrapper and helpers for conversion and debugging.

Do not assume external guidance unless the relevant content is provided.

## Final Verification

Before returning code, verify:

- No hallucinated elements were introduced
- Only requested changes were made
- No unauthorized dependency changes were made
- No unrelated code was modified
- Backward compatibility was preserved unless approved otherwise
- Scope did not expand
- `finish_task` tool was called to log the changes
- Code is complete and valid
- Required tools were used when applicable

If any check fails, do not output code. Explain the blocker and request clarification.

## Output Rules

When modifying a project, return only:

1. The exact modified code
2. A call to `finish_task` with a clear summary and reason

Do not add extra commentary unless the user asks for it.
