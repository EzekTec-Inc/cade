# CADE Memory Architecture — Proposed Overhaul

> Based on 7 deficiencies identified in `docs/memory-deficiency-report.md`.
> User has approved architectural changes.

## Design Philosophy

The current memory system fails because it's **passive** — it writes data, hopes the
budget fits, and silently drops what doesn't. The fix isn't bigger budgets. The fix
is making the system **active**: it should know what matters, tell the agent what it
can't see, and never silently lose data.

**Three architectural principles:**

1. **Never silently drop** — if context can't fit, tell the agent explicitly what
   was dropped and how to recover it
2. **Ground before acting** — the agent must verify state from filesystem/DB before
   claiming something is true, not from stale memory
3. **Structured over prose** — replace free-form memory blocks with typed, queryable
   facts that survive compression better than paragraphs

---

## Architecture Changes

### A1: Replace Fixed Budget Percentages with Adaptive Packing

**Current:** Hard 2%/8%/1.5% splits for pinned/short/long regardless of actual content.

**Proposed:** Single unified memory budget (11.5% → 15% of context window) with
**priority-ordered greedy packing**:

```
Priority order:
  1. Core identity (persona, human, project) — always first, never dropped
  2. active_goal + recent_edits + session_summary — always second
  3. Loaded skills — next (these are explicitly requested)
  4. User-pinned blocks — next
  5. Short-term blocks — by recency
  6. Long-term excerpts — whatever fits
```

When the budget fills, emit a **manifest** of what was excluded:

```
# Context Overflow
The following memory blocks were excluded from this context:
- [session_summary_3] (4,200 chars) — use search_memory("session_summary_3") to retrieve
- [skill:rust] (3,100 chars) — use load_skill("rust") to reload
- [old_decisions] (1,800 chars, archived) — use search_memory("old_decisions")
```

This replaces the useless `[…3 block(s) omitted]` with actionable recovery instructions.

**Files:** `crates/cade-server/src/server/api/messages/context.rs` (assemble_system_prompt_memory)
**Lines:** ~1083-1260
**Effort:** Medium (rewrite packing loop, add manifest generation)

### A2: Write-Ahead Verification for Memory Blocks

**Current:** `update_memory` returns "updated" even if truncated. Agent hallucinates
about what's stored.

**Proposed:** Two changes:
1. `upsert_memory_block` returns `(was_truncated: bool, stored_chars: usize, requested_chars: usize)`
2. The meta-tool handler includes this in the response:

```
Memory block 'active_goal' updated (2,000 / 2,000 chars).
⚠️ WARNING: Content was truncated from 3,500 to 2,000 chars.
Consider splitting into multiple blocks or using archival_memory_insert for overflow.
```

**Files:**
- `crates/cade-store/src/sqlite/memory.rs` (upsert_memory_block return type)
- `crates/cade-server/src/server/api/run/meta_tools.rs` (handle_update_memory)
- `crates/cade-agent/src/tools/runtime/memory.rs` (CLI path)
**Effort:** Low

### A3: Rich Archived Excerpts with Retrieval Hints

**Current:** 80-char prefix from block value. Useless for deciding relevance.

**Proposed:** When a block transitions from short → long (archived), compute and store:
1. **250-char excerpt** (first 250 chars, not 80)
2. **Keywords** — top 5 most distinctive terms (TF-IDF-like, excluding stop words)
3. **Block metadata** — creation date, last write date, approximate size

The archived section then shows:
```
# Archived Memory
[session_summary_3]: Implemented Phase 3 subagent refactoring — split run.rs into…
  keywords: subagent, run.rs, meta_tools, Phase3, extraction | 4,200 chars | written 2026-05-02
[old_decisions]: CLI skills duplication closed as working as designed…
  keywords: skills, CLI, duplication, Arc<Mutex>, decision | 1,800 chars | written 2026-04-20
```

Now the agent can make an **informed decision** about whether to search_memory.

**Files:**
- `crates/cade-store/src/sqlite/memory.rs` (get_long_term_excerpts, add keyword extraction)
- `crates/cade-server/src/server/api/messages/context.rs` (render archived section)
**Effort:** Medium

### A4: Ground-Truth Verification Directive

**Current:** System prompt says "NEVER hallucinate" but provides no mechanism.

**Proposed:** Add a **Ground Truth Protocol** section to the system prompt:

```
## Ground Truth Protocol (MANDATORY)

Before asserting any fact about the codebase, workspace, or prior work:
1. If claiming a file exists/was modified → verify with read_file or glob
2. If claiming a commit was made → verify with git log
3. If claiming a task is complete → verify with the test/build command
4. If recalling prior conversation → verify with conversation_search or search_memory

NEVER state something as fact based solely on memory blocks. Memory blocks are
*notes to yourself* — they may be stale, truncated, or from a different session.
Always verify against the source of truth (filesystem, git, DB).
```

This is a **behavioral change** — it shifts the agent from "trust memory" to "verify
from source." Combined with A1 (the manifest telling the agent what it can't see),
the agent knows when it needs to search and is trained to verify.

**Files:** `src/bootstrap/prompt.rs` (BASE_SYSTEM_PROMPT)
**Effort:** Low

### A5: Server-Side Memory Freshness Enforcement

**Current:** Client-side hook blocks writes when active_goal is stale.

**Proposed:** Move to the server's agentic loop (`run/mod.rs`). After every N tool
calls (default: 5) where the agent hasn't called `update_memory(label='active_goal')`,
inject a **system nudge** into the next LLM turn:

```json
{
  "role": "system",
  "content": "⚠️ Your active_goal memory block has not been updated in 7 tool calls.
  Update it now with your current task, status, and next steps to prevent context loss."
}
```

This is not a hard block — the agent can still proceed — but it surfaces as an
in-band reminder that the LLM will process. Much more reliable than a hook that
only works in CLI mode.

**Files:**
- `crates/cade-server/src/server/api/run/mod.rs` (run_agent_loop, track tool call count)
- Add a counter: `active_goal_write_distance: usize` reset on memory write
**Effort:** Medium

### A6: Observation Window Scaling

**Current:** Fixed 30 observations / 2,000 chars.

**Proposed:** Scale observation budget with context window:
- 128k models: 50 obs / 4,000 chars
- 200k models: 75 obs / 6,000 chars
- 1M models: 150 obs / 12,000 chars

Add **turn offset** to each observation so the agent knows how old it is:

```
# Recent Observations (turns 42-55)
[turn 55, 2m ago] write_file(src/main.rs) → ok (importance: 4)
[turn 54, 3m ago] cargo test → 1501 passed (importance: 4)
[turn 42, 18m ago] read_file(Cargo.toml) → 45 lines (importance: 2)
```

**Files:**
- `crates/cade-server/src/server/api/messages/context.rs` (observation injection)
- `crates/cade-store/src/sqlite/observations.rs` (render_observations_section)
**Effort:** Low

### A7: Session Continuity Improvements

**Current:** Ring of 8 summaries, oldest evicted to 1-line excerpt in session_index.

**Proposed:**
1. Increase excerpt on eviction from ~100 chars → 500 chars
2. Increase `session_index` cap from 5,000 → 10,000 chars
3. Before eviction, write the full session_summary_N content to **archival memory**
   with tag `evicted-session-summary` so it's recoverable via search
4. session_index entries include the archival memory key for recovery

**Files:**
- `crates/cade-server/src/server/consolidation.rs` (rotate_and_archive_session_summary)
**Effort:** Medium

---

## Implementation Order

**Phase 1 — Immediate impact, low risk (1-2 hours):**
- A2: Write-ahead verification (warn on truncation)
- A4: Ground Truth Protocol in system prompt
- A6: Observation window scaling

**Phase 2 — Structural improvements (2-4 hours):**
- A1: Adaptive packing with overflow manifest
- A3: Rich archived excerpts with keywords

**Phase 3 — Behavioral enforcement (2-3 hours):**
- A5: Server-side memory freshness nudge
- A7: Session continuity improvements

---

## Expected Outcomes

| Deficiency | Fix | Expected Impact |
|-----------|-----|-----------------|
| D1 (budget too small) | A1 | Pinned blocks no longer silently dropped; overflow is explicit |
| D2 (stale active_goal) | A5 | Server reminds agent every 5 tool calls; works in all modes |
| D3 (80-char excerpts) | A3 | 250-char + keywords; agent can decide relevance without searching blind |
| D4 (session-scoped search) | A7 | Evicted summaries preserved in archival; recoverable |
| D5 (observation trail short) | A6 | Scales with model; 2-4× more observations visible |
| D6 (lossy eviction) | A7 | 500-char excerpts + archival backup on eviction |
| D7 (silent truncation) | A2 | Agent warned immediately; can split or archive |
| NEW (hallucination root cause) | A4 | Agent trained to verify from source, not trust memory |
