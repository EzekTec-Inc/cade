# CADE Amnesia Root Cause Analysis

**Date:** 2026-05-07
**Investigator:** CADE self-analysis
**Symptom:** Agent loses track of what it's working on mid-flight, re-investigates completed tasks, claims work was done that wasn't, and enters hallucination loops across sessions.

---

## Executive Summary

Nine root causes identified across four subsystems: **consolidation** (3), **context assembly** (2), **memory lifecycle** (3), and **reflection** (1). Two are P0 (directly cause observed amnesia), three are P1, four are P2.

The primary amnesia loop is: **RC-2 (auto_update_active_goal) overwrites the agent's own task tracking with an LLM-hallucinated summary derived from lossy compressed history**. This is the single biggest contributor to "claims work was done that wasn't done."

---

## ROOT CAUSE 1: Consolidation Budget Mismatch — Double-Vision

**Files:**
- `crates/cade-server/src/server/consolidation.rs:244` (40% char budget)
- `crates/cade-server/src/server/api/messages/context.rs:557-671` (token-based walker)

**Pattern:** B (re-investigates completed tasks), E (consolidation destroys nuance)

**Mechanism:** Consolidation decides which turns are "dropped" using a fixed `HISTORY_BUDGET_FRACTION = 0.40` of the model's context window, calculated as `char_budget * 0.40`. But `build_context` uses a precise BPE token-based walk (`turn_cost_chars()`) with system overhead subtracted, an 85% compaction threshold, and tool-result truncation. These two algorithms produce different "in context" vs "dropped" boundaries.

Result: consolidation can summarize turns that `build_context` still includes at full fidelity. The agent then sees BOTH the raw conversation AND a compressed summary of the same events — with potential contradictions between them. The summary may say "task X completed" while the raw turn shows "task X in progress."

**Severity:** P1

**Fix direction:** Synchronize the consolidation budget threshold to match `PROACTIVE_CONSOLIDATION_THRESHOLD` (70%) and use precise token counts in consolidation loop instead of raw chars.

---

## ROOT CAUSE 2: auto_update_active_goal Blindly Overwrites Agent's Own State

**Files:** `crates/cade-server/src/server/consolidation.rs:723-808`

**Pattern:** A (claims work done that wasn't), C (memory drifts from filesystem truth)

**Mechanism:** After every consolidation, a cheap LLM (haiku/gpt-4o-mini) is asked to derive `active_goal` from the lossy 8,000-char summary. If the LLM returns anything other than "UNCHANGED", the existing `active_goal` is blindly overwritten — even if the agent wrote a carefully accurate `active_goal` 2 turns ago.

The compaction model (max 400 output tokens) works from a lossy summary. A task completed in recent turns — but those turns were in the "dropped" window or truncated — becomes "still in progress" in the regenerated `active_goal`.

**This is the primary amnesia vector.** On next session start, the agent reads the LLM-hallucinated `active_goal`, trusts it, and begins re-investigating already-completed work.

**Severity:** P0 — This is the #1 cause of observed amnesia.

**Fix direction:**
1. Add recency guard: if agent wrote `active_goal` within last 15 turns, skip auto-update
2. Add status regression guard: never overwrite "completed" with "in-progress" without evidence

---

## ROOT CAUSE 3: No active_goal Completion Lifecycle

**Files:**
- `crates/cade-agent/src/tools/runtime/memory.rs` (memory tools)
- `crates/cade-server/src/server/reflection.rs:56-57` (skips tool results)

**Pattern:** A (claims work done that wasn't), D (trusts memory over filesystem)

**Mechanism:** `active_goal` has no lifecycle management. There is no `complete_task` tool. The agent is expected to call `update_memory("active_goal", "completed...")` voluntarily, but:

1. The system prompt says to update it "after every significant code change" but provides no enforcement
2. Reflection explicitly skips tool results, so git commits, test passes, and file writes never trigger automatic `active_goal` updates
3. The reflection prompt says "Only extract PERSISTENT facts (not transient task steps)" — meaning it wouldn't update `active_goal` even if it saw completion evidence
4. P7 auto-update (RC-2) is the only automated mechanism, but it hallucinates

Net effect: `active_goal` gets stuck in "in-progress" forever unless the agent proactively updates it. And even if it does, RC-2 can overwrite it.

**Severity:** P0

**Fix direction:**
1. Include tool results (at least tool names + exit codes, up to 150 chars) in the reflection input
2. Update the reflection prompt to extract task completion status into `active_goal`

---

## ROOT CAUSE 4: Session Summary Ring Silently Discards History

**Files:** `crates/cade-server/src/server/consolidation.rs:813-935`

**Pattern:** B (re-investigates completed tasks across sessions), E (nuance destroyed)

**Mechanism:** Live `session_summary` (8,000 chars) is rotated into `session_summary_1` through `session_summary_8`. When the ring of 8 fills, the oldest is evicted into `session_index` (a 10,000-char FIFO). But ring entries in `long` tier show as 250-char excerpts in context (3% of original), and `session_index` is FIFO — oldest lines simply dropped.

**Severity:** P2

**Fix direction:** Increase excerpt size for archived ring entries. Consider structured project timeline instead of FIFO. Make ring summaries hierarchically compressed.

---

## ROOT CAUSE 5: promote_stale_blocks Has No Tool-Return Guard

**Files:** `crates/cade-server/src/server/api/messages/context.rs:1108`

**Pattern:** C (memory drifts from truth)

**Mechanism:** `promote_stale_blocks` is called on every `build_context` invocation — including tool returns. Although the turn counter isn't incremented for tool returns (correct), the promote call still runs. During a multi-tool-call turn (5 parallel tool calls = 5 build_context calls), the promote logic fires 5 times with the same turn counter.

**Severity:** P2

**Fix direction:** Guard with `if !is_tool_return { ... }`.

---

## ROOT CAUSE 6: Reflection Produces "Persistent Facts" Instead of Task State

**Files:** `crates/cade-server/src/server/reflection.rs:98-130`

**Pattern:** A (claims work done), D (trusts memory over filesystem)

**Mechanism:** The reflection prompt explicitly instructs: "Only extract PERSISTENT facts (not transient task steps)". This means reflection creates blocks like `rust_error_handling_convention: "uses anyhow"` but NEVER creates blocks like `task_X_completed: true` or updates `active_goal`.

**Severity:** P1

**Fix direction:** Update reflection prompt to parse tool results and extract completion status into `active_goal`.

---

## ROOT CAUSE 7: No Contradiction Detection

**Files:** N/A — this is an architectural gap

**Pattern:** C (memory drifts), D (trusts memory over filesystem)

**Mechanism:** There is no mechanism to detect when a memory block contradicts filesystem state. The agent can have `active_goal: "implementing feature X in file Y"` when `git log` shows feature X was merged 50 turns ago. Nothing flags this.

**Severity:** P1

**Fix direction:** Add a `verify_active_goal` hook that runs at session start to check mentioned files against `git status`/`git log` and flag contradictions.

---

## ROOT CAUSE 8: search_memory Promotes But Serves Stale Excerpts

**Files:** `crates/cade-store/src/sqlite/memory.rs:923-964`

**Pattern:** C (memory drifts from truth)

**Mechanism:** When `search_memory` finds a match in an archived `long` block, it promotes the block back to `short` tier. But the search result returned to the agent is the 250-char excerpt from the long tier, not the full block. The full block only appears on the NEXT context build.

**Severity:** P2

**Fix direction:** Return the full block content in the search result, not just the excerpt.

---

## ROOT CAUSE 9: Consolidation LLM Failures Are Silent

**Files:** `crates/cade-server/src/server/consolidation.rs:772-780`

**Pattern:** A (stale active_goal)

**Mechanism:** If the compaction LLM (P7) returns malformed output, a partial response, or an error, the only signal is a `tracing::debug!` log.

**Severity:** P2

**Fix direction:** Validate the LLM output before writing: check for minimum structure (has "Current task", "Status", "Next steps").