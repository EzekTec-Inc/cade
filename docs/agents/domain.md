# Domain docs

This repo uses a **single-context** layout.

## Locations

- **Domain model & glossary**: `CONTEXT.md` at the repo root.
- **Architectural Decision Records (ADRs)**: `docs/adr/*.md`.

## Consumer rules

- When a skill says "read the domain context", read `CONTEXT.md`.
- When a skill says "check past architectural decisions", read the latest relevant ADRs in `docs/adr/`.
- When recording a new architectural decision, add a new sequentially numbered ADR in `docs/adr/`.
