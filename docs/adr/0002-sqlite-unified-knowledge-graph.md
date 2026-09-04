# ADR 2: SQLite-Backed Unified Knowledge Graph for Shared Memory

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

CADE supports running multiple concurrent subagents as a team. When these agents explore the codebase, run tests, or modify files, they acquire valuable knowledge, discover dependencies, and establish facts.

We need a persistent, concurrent, shared memory registry so that:
1. Subagents can share knowledge and facts in real time.
2. The parent agent can retrieve these facts.
3. Memory does not reside purely in ephemeral prompt contexts, which are highly token-expensive and prone to rotation truncation.

Using separate raw text files for each agent would result in massive I/O sync issues, data race locks, and duplicate knowledge entries.

## Decision

We decided to implement a centralized, database-backed **Unified Knowledge Graph** stored inside the main SQLite database (`knowledge_edges` table). All main agents and subagents query and insert facts directly as structured grounding edges. 

The graph is indexed using local cosine-similarity vector embeddings to support semantic searches.

## Consequences

### Positive (Pros)
* **Real-Time Synergy**: Main agents and concurrent subagents have immediate, thread-safe, concurrent access to the same centralized knowledge pool.
* **Token Efficiency**: Instead of injecting thousands of lines of raw text logs into subagent context windows, agents can perform highly targeted semantic query lookups costing ~50 tokens.
* **Persistence**: Discovered workspace relationships (e.g., imports, dependencies, code layouts) survive across conversations and subagent sessions.

### Negative (Cons)
* **SQLite Dependency**: The system couples memory mechanics directly to a relational SQLite database structure.
* **Embedding Overhead**: Vector search requires an active embedding provider (or local vector computation model) to generate binary embeddings.
