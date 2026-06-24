---
name: Conventional Commits
description: Always write git commit messages following the Conventional Commits spec
category: git
tags: [git, commit, conventional]
---

# Conventional Commits

Always write commit messages using the Conventional Commits format: `<type>[optional scope]: <description>\n\n[optional body]\n\n[optional footer]`.

| Type | Description |
|---|---|
| `feat:` | A new feature |
| `fix:` | A bug fix |
| `docs:` | Documentation changes only |
| `style:` | Formatting, whitespace (no code change) |
| `refactor:` | Code restructuring without behavior change |
| `perf:` | Performance improvements |
| `test:` | Adding or updating tests |
| `chore:` | Build, tools, dependencies |
| `ci:` | CI/CD configuration |

## Rules

1. Description: lowercase, imperative mood ("add", no period, under 72 chars).
2. Body: explains *why*, not *what*.
3. Breaking changes: append `!` to type (e.g., `feat!:`) or place `BREAKING CHANGE:` in footer.
4. **Task Finalization**: Before committing, call `finish_task` to log changes to `CADE_AUDIT.md`.

## Examples
* `feat(auth): add JWT token refresh endpoint`
* `fix: prevent duplicate tool_result blocks in SSE reconnect`
* `refactor(llm): replace static provider with LlmRouter`
* `docs: update README with /model switching instructions`
