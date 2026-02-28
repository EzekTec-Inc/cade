---
name: Conventional Commits
description: Always write git commit messages following the Conventional Commits spec
category: git
tags: [git, commit, conventional]
---

# Conventional Commits

When writing git commit messages, always use the Conventional Commits format:

```
<type>[optional scope]: <description>

[optional body]

[optional footer]
```

## Types

- `feat:` — a new feature
- `fix:` — a bug fix
- `docs:` — documentation changes only
- `style:` — formatting, whitespace (no code change)
- `refactor:` — code restructuring without behaviour change
- `perf:` — performance improvements
- `test:` — adding or updating tests
- `chore:` — build process, tooling, dependencies
- `ci:` — CI/CD configuration

## Rules

1. Description is lowercase, imperative mood ("add" not "added" or "adds")
2. No period at the end of the description
3. Body explains *why*, not *what*
4. Breaking changes: add `!` after type (e.g. `feat!:`) or `BREAKING CHANGE:` in footer
5. Keep description under 72 characters

## Examples

```
feat(auth): add JWT token refresh endpoint

fix: prevent duplicate tool_result blocks in SSE reconnect

refactor(llm): replace static provider with LlmRouter

docs: update README with /model switching instructions
```
