# Memory System

CADE's memory is a tiered, persistent key-value store. Blocks live in
SQLite and are surfaced to the LLM through the system prompt.

## Tiers

| Tier | Where | Aging | Use for |
|---|---|---|---|
| **Pinned** | Always in the prompt | Never aged | `persona`, `human`, `project`, `working_set` |
| **Short-term** | In the prompt | Archived after 80 idle turns | Active task notes, in-flight decisions |
| **Long-term (archived)** | Replaced by `label + excerpt` | Restored on demand | Old context, historical reference |

A block becomes archived automatically once 80 turns elapse without it being
read or written. The system prompt then carries only its label and a short
FTS snippet — the full body is fetched again the moment it's matched by
`search_memory()` or referenced by name. Unused short-term and archived memories
will gradually decay in confidence over time (losing 5% confidence per 20 turns of idleness).

> **Tip:** Use `/memory pin <label>` to make any block permanently active (immune to archival). Pinned blocks are always injected into the agent's prompt.

## Typed Relationships & Provenance

Each memory block carries strong semantic metadata:
- **`memory_type`**: Classifies the memory (e.g., `decision`, `convention`, `project_fact`, `constraint`).
- **`confidence`**: A floating-point score (0.1 to 1.0) indicating certainty.
- **`source_turn_id` & `source_tool_id`**: Verifiable provenance tracking exactly when and by which tool the fact was recorded.

## Built-in blocks

| Block | Pinned | Purpose |
|---|---|---|
| `persona` | ✅ | Agent identity, working style, communication preferences |
| `human` | ✅ | Facts about the user (name, role, preferences) |
| `project` | ✅ | Current project context, tech stack, conventions |
| `working_set` | ✅ | Active task, files modified, next steps |
| `session_summary` | (auto) | Generated summary of older conversation turns |
| `skills` | ✅ | Static index of available skills (P1 cache-anchored) |

The `working_set` block is reset by `/init` or `/new`, and is the
recommended write target for "what am I doing right now" context.

## Reading & writing memory

```bash
# In a session
/memory                   # list all blocks
/memory view <label>      # show full content
/memory set <label> <v>   # upsert a value
/memory edit <label>      # open in $EDITOR
/memory delete <label>    # remove
/memory history <label>   # last 5 revisions

# As a tool call (from the LLM)
update_memory(label="active_goal", value="...")
search_memory(query="...")
archival_memory_search(query="...")
archival_memory_insert(content="...")  # offload to long-term
```

The `update_memory` tool is one of CADE's **meta tools** — always
available, never filtered out.

## Sleeptime consolidation

When the context window crosses ≈ 98% utilisation, the server runs a
background **consolidation pass** in
`crates/cade-server/src/server/consolidation.rs`:

1. Identify the oldest dropped turns since the last consolidation.
2. Extract key artifacts — file paths, function names, error IDs — as
   "search anchors".
3. Call the **compaction model** (cheaper than the main model — see
   [cost-and-pricing.md](cost-and-pricing.md)) to summarise those turns.
4. Append the result to `session_summary`. If `session_summary` would
   exceed `SESSION_SUMMARY_MAX_CHARS`, the previous value is rotated into
   `session_summary_N` (long-term tier — Phase C).
5. **Auto-Extract Facts**: The compaction model automatically scans the dialogue summary and extracts durable knowledge (decisions, constraints, conventions) into typed memory blocks, ensuring long-term project context survives without manual agent action.

You can trigger this manually with `/compact` (or `/consolidate`).

## Adaptive guardrails

- **Inflation guard** — if the new summary's character count exceeds the
  raw turns it summarises, consolidation is skipped (the summary would
  cost more than the dropped content).
- **Per-agent compaction model** — `/compaction-model anthropic/claude-3-5-haiku-latest`
  pins a cheap summariser. Default resolver picks the cheapest model in
  the same provider family as the agent's main model.

## Centralized Knowledge Graph (PI-Style)

Rather than storing isolated, separate text memory blocks that must be constantly synchronized or merged, CADE implements a centralized, durable **Unified Knowledge Graph** stored in the SQLite database (`knowledge_edges` table):
- **Durable Grounding**: All agents and active subagents read and write structured knowledge edges (e.g., `["main.rs", "calls", "setup_panic_hook"]`) directly to and from the centralized graph store. This acts as a single, concurrent, non-ephemeral source of truth.
- **Semantic Vector Search**: When a knowledge edge is inserted, CADE calculates its semantic vector embedding and packs it as a binary f32 BLOB. CADE can then perform high-performance semantic vector searches using local cosine-similarity checks, retrieving relevant facts instantly.

## Memory-related env vars

| Variable | Effect |
|---|---|
| `CADE_CONTEXT_BUDGET` | Override the default context budget (chars) |
| `CADE_MAX_CONTEXT_BUDGET` | Hard upper cap regardless of model |

See [configuration.md](configuration.md) for the full env var list.

## Searching dropped history

Older conversation turns get truncated from the visible window. To
retrieve them:

- `/search <query>` — server-side FTS5 across all messages
- `conversation_search(query)` — same, but as a tool call from the LLM
- `archival_memory_search(query)` — search the long-term tier

When matched, archived blocks are **auto-promoted** back into active
memory for the next prompt.

### Semantic Search (optional)

`search_memory(query="...", memory_type="...")` supports filtering by semantic relation types (e.g., searching only for `decision` blocks) and uses a hybrid ranking pipeline. Keyword and fuzzy matching are always available. Cosine-similarity ranking is enabled when the `cade-store/semantic-search` feature is compiled in via the root `semantic-search` feature:

```bash
cargo build --release --features semantic-search
```

1. **Phase 1 — Keyword (LIKE)**: Exact substring matching against block labels and values
2. **Phase 2 — Fuzzy word-match**: Splits query into words, matches blocks containing any word ≥3 chars
3. **Phase 3 — Cosine similarity**: When enabled, embeds the query via `fastembed` (AllMiniLML6V2, 384-dim) and searches `sqlite-vec` virtual tables for nearest neighbors

Results from all available phases are merged via **Reciprocal Rank Fusion** (k=60), which boosts blocks that appear in multiple result sets.

Embeddings are computed and stored whenever a memory block is written via `update_memory` while semantic search is enabled. The embedding model downloads on first use. Without the feature, Phases 1 and 2 still run — semantic ranking is additive, not required.
