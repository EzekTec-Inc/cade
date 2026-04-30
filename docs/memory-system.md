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
`search_memory()` or referenced by name.

> **Tip:** Use `/memory pin <label>` to make any block permanently active (immune to archival). Pinned blocks are always injected into the agent's prompt.

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
5. Trigger optional **reflection** subagent to extract durable facts
   (decisions, preferences, conventions) into typed memory blocks.

You can trigger this manually with `/compact` (or `/consolidate`).

## Adaptive guardrails

- **Inflation guard** — if the new summary's character count exceeds the
  raw turns it summarises, consolidation is skipped (the summary would
  cost more than the dropped content).
- **Per-agent compaction model** — `/compaction-model anthropic/claude-3-5-haiku-latest`
  pins a cheap summariser. Default resolver picks the cheapest model in
  the same provider family as the agent's main model.

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

### Semantic Search (default)

`search_memory()` uses a **hybrid ranking** pipeline by default. The
underlying `semantic-search` feature is enabled in `cade-store`'s default
feature set (since 2026-04-30); to disable it, build with
`--no-default-features --features bundled-sqlite` — Phases 1 and 2 still run.

1. **Phase 1 — Keyword (LIKE)**: Exact substring matching against block labels and values
2. **Phase 2 — Fuzzy word-match**: Splits query into words, matches blocks containing any word ≥3 chars
3. **Phase 3 — Cosine similarity**: Embeds the query via `fastembed` (AllMiniLML6V2, 384-dim) and searches `sqlite-vec` virtual tables for nearest neighbors

Results from all three phases are merged via **Reciprocal Rank Fusion** (k=60), which boosts blocks that appear in multiple result sets.

Embeddings are automatically computed and stored whenever a memory block is written via `update_memory`. The embedding model (~50MB) downloads on first use. With the feature disabled, Phases 1 and 2 still run — semantic ranking is purely additive.
