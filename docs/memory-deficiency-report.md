# CADE Memory System — Deficiency Report

## Executive Summary

After auditing the full memory pipeline (write → store → retrieve → inject → LLM context),
I've identified **7 root causes** of hallucination and amnesia. They fall into two categories:

- **Structural deficiencies** — fundamental gaps in the architecture
- **Budget & tuning issues** — correct design but miscalibrated parameters

---

## Root Cause Analysis

### D1: Pinned Memory Budget is Critically Small (HALLUCINATION + AMNESIA)

**Severity:** 🔴 Critical

**Problem:** The pinned memory budget is only **2% of the context window** (in chars).
For Claude Sonnet 4 (200k tokens), that's only **16,000 chars** for ALL pinned blocks
combined (`persona`, `human`, `project`, `working_set`, `session_index`, any skill blocks,
`active_goal`, etc.).

In practice, a typical CADE session has:
- `persona`: ~2,000 chars
- `human`: ~3,000 chars
- `project`: ~3,000 chars (often much larger with MCP rules, skills list)
- `session_index`: ~5,000 chars
- `working_set`: ~2,000 chars
- Loaded skills: ~5,000+ chars each

**Total easily exceeds 20,000+ chars.** When that happens, the code in
`assemble_system_prompt_memory` (context.rs:1136-1149) **silently drops pinned blocks
that don't fit**, incrementing `active_omitted` — but only showing a vague
`[…N block(s) omitted — memory budget reached]` message.

**Impact:** The LLM loses critical context (project rules, skill instructions, session
history) and starts hallucinating because it literally cannot see the information. The
agent doesn't know *which* blocks were dropped.

**BUT:** `active_goal`, `recent_edits`, and `session_summary` are carved out as
"dynamic" blocks that bypass the budget system entirely (context.rs:1127-1132). This
means the actual dynamic-critical blocks survive. The issue is with skill blocks and
user-added pinned blocks.

### D2: `active_goal` Has No Staleness Enforcement Before Writes (HALLUCINATION)

**Severity:** 🟡 High

**Problem:** The system prompt tells the agent to update `active_goal` "after every
significant code change." But there's no server-side enforcement. The hook you saw
(`[BLOCKED: Your active_goal memory block is empty or stale...]`) is a **client-side
hook** configured in `.cade/hooks.json` or `.scripts/hooks/` — it's not a server-level
invariant.

When the hook isn't configured (or in server-mode/GUI runs), the agent can execute
dozens of write operations without ever updating `active_goal`. Then when context
rotates, all knowledge of what was done is lost.

**Impact:** The auto-update in consolidation (`auto_update_active_goal`, line 723) fires
only during sleeptime (60s of inactivity). If the agent is continuously active, the
`active_goal` block may hold stale information from many turns ago.

### D3: Archived Block Excerpts Are Only 80 Characters (AMNESIA)

**Severity:** 🟡 High

**Problem:** When a memory block is archived (tier='long'), the context only shows a
**first-80-character excerpt** (memory.rs:637):

```rust
let excerpt: String = value.chars().take(80).collect();
```

For a 5,000-char block containing critical project decisions, the LLM sees:
```
[my_important_block]: This block contains information about the database schema and mi…
```

That's useless for determining whether to call `search_memory` to retrieve it. The
agent has no way to know if the block contains relevant information without calling
search on every single archived block.

**Impact:** Critical context vanishes behind truncated labels. The agent doesn't search
for what it doesn't know exists.

### D4: `conversation_search` Cannot Search Across Session Boundaries (AMNESIA)

**Severity:** 🟡 High

**Problem:** `conversation_search` searches the `messages` table, which stores the
current conversation's messages. When CADE restarts (new session), a new conversation
is typically created. The old conversation's messages are still in the DB but
`conversation_search` defaults to searching only the current conversation unless
explicitly told otherwise.

The consolidation system creates `session_summary` and `session_summary_N` blocks,
but these are **LLM-generated summaries** — lossy by definition. The full original
messages are cached to archival memory (F2, consolidation.rs:370), but the agent
would need to know to call `archival_memory_search` with the right tag.

**Impact:** Cross-session context is reduced to a lossy LLM summary. Specific details
(exact file paths, error messages, design decisions) are lost unless the agent
proactively stored them in a memory block.

### D5: Observation Trail is Small and Only Recent (AMNESIA)

**Severity:** 🟠 Medium

**Problem:** The observation system records tool calls with summaries. But the context
injection (context.rs:1232-1247) only fetches the **last 30 observations with
importance ≥ 3**, capped to **2,000 characters**. That's roughly 20-25 observation
lines.

In an active session, 30 observations can represent as few as **5-10 turns** of work.
For a session lasting 50+ turns, the agent loses visibility into what it did on
turns 1-40.

**Impact:** The agent can't recall its own recent actions beyond a very narrow window,
leading to repeated work or contradictory actions.

### D6: Session Summary Ring Overwrites With Lossy Compression (AMNESIA)

**Severity:** 🟠 Medium

**Problem:** The consolidation system maintains a ring of `session_summary_N` blocks
(up to 8). When the ring fills, the oldest is evicted and a **one-line excerpt** is
appended to `session_index`. This means:

```
Session 1: 8,000 chars of summary → compressed to 4,000 chars when rotated
                                   → compressed to ~100 chars when evicted to session_index
```

After 8+ consolidation cycles, the earliest sessions are represented by a single line
in `session_index`. If the agent needs to recall what happened 3 sessions ago, the
detail is gone.

**Impact:** Long-running projects gradually lose project history. The `session_index`
pinned block grows but each entry is too terse to be useful.

### D7: No Verification Loop After Memory Write (HALLUCINATION)

**Severity:** 🟠 Medium

**Problem:** When the agent calls `update_memory`, it gets back "Memory block 'X'
updated" — but there's no mechanism to verify that the block was actually written
correctly, or that it wasn't silently truncated. The `upsert_memory_block` function
has a `max_chars` parameter (default 2,000 for some tiers) that truncates without
warning.

The agent then proceeds assuming the memory block contains what it wrote, when it
may have been truncated. On the next context rotation, the truncated block is
injected, and the agent hallucinates based on partial information.

**Impact:** Silent truncation causes the agent to operate on incomplete data.

---

## Summary Matrix

| ID | Deficiency | Type | Severity | Effort to Fix |
|----|-----------|------|----------|---------------|
| D1 | Pinned budget too small (2%) | Budget | 🔴 Critical | Low (tune constant) |
| D2 | No server-side `active_goal` staleness check | Structural | 🟡 High | Medium (add middleware) |
| D3 | Archived excerpts only 80 chars | Budget | 🟡 High | Low (increase + add keywords) |
| D4 | `conversation_search` scoped to current session | Structural | 🟡 High | Medium (cross-session default) |
| D5 | Observation trail too short (30 obs / 2k chars) | Budget | 🟠 Medium | Low (tune constants) |
| D6 | Session ring lossy eviction | Structural | 🟠 Medium | Medium (better eviction policy) |
| D7 | Silent memory truncation on write | Structural | 🟠 Medium | Low (return warning) |

---

## Proposed Fixes (Priority Order)

### Fix 1: Increase Pinned Memory Budget (D1)
- Change `PINNED_BUDGET_MIN` from 10,000 → 40,000
- Change pinned ratio from 2% → 5% of context window
- For 200k-token models: 16k → 40k chars
- Risk: reduces conversation history window by ~24k chars (negligible on 200k+ models)

### Fix 2: Richer Archived Excerpts (D3)
- Increase excerpt length from 80 → 200 chars
- Add keyword extraction to archived excerpts (top 5 terms from the block's value)
- This helps the agent decide whether to call `search_memory` on a specific block

### Fix 3: Warn on Truncated Memory Writes (D7)
- When `upsert_memory_block` truncates, return a message like:
  `"Memory block 'X' updated (WARNING: truncated from N to M chars)"`
- The agent can then split the block or archive the overflow

### Fix 4: Expand Observation Window (D5)
- Increase from 30 → 50 observations
- Increase budget from 2,000 → 4,000 chars
- Add turn-age to observation lines so the agent knows how recent they are

### Fix 5: Cross-Session conversation_search Default (D4)
- When `conversation_id` is not specified, search ALL conversations (current behavior)
  but also search the archival `dropped-turns` entries
- Add a hint when results are empty: "Try archival_memory_search for older sessions"

### Fix 6: Server-Side active_goal Freshness Check (D2)
- In `intercept_meta_tool`, before dispatching any write tool (write_file, bash, etc.),
  check if `active_goal` has been updated in the last N tool calls
- If stale, inject a system message reminding the agent to update it
- This replaces the client-side hook with a server-side guarantee

### Fix 7: Improve Session Ring Eviction (D6)
- Instead of a one-line excerpt when evicting from the ring, keep a 500-char summary
- Cap `session_index` at 10,000 chars instead of 5,000
- Consider semantic deduplication across session summaries
