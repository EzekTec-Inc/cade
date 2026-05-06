# CADE Memory Architecture Rework Spec

> **Status:** ALL 5 PHASES COMPLETE
> **Target:** Address structural amnesia and hallucination root causes.

## Implementation Status

| Phase | Items | Status |
|-------|-------|--------|
| **Phase 1** | A1 (truncation), A2 (ground truth), A3 (provenance) | ✅ DONE |
| **Phase 2** | A4 (rich excerpts), A5 (chunking), A6 (chunk search) | ✅ DONE |
| **Phase 3** | A7 (greedy packing), A8 (overflow manifest), A9 (proactive injection) | ✅ DONE |
| **Phase 4** | A10 (access tracking), A11 (decay scoring), A12 (staleness nudge) | ✅ DONE |
| **Phase 5** | A13 (expanded observations), A14 (session eviction), A15 (subagent write-back) | ✅ DONE |

## 1. Gap Analysis

### Known Budget Gaps (D1–D7)
- **D1:** Pinned budget too small — **fixed by A7** (unified budget, greedy packing)
- **D2:** `active_goal` freshness — **to be addressed by A12** (staleness nudge)
- **D3:** Archived excerpts too short — **fixed by A4** (250 chars + keywords)
- **D4:** `conversation_search` session-scoped — out of scope
- **D5:** Observation trail too small — **to be addressed by A13**
- **D6:** Session ring lossy compression — **to be addressed by A14**
- **D7:** Silent truncation — **fixed by A1** (truncate + warn)

### Structural Gaps (G1–G5)
- **G1:** No provenance — **fixed by A3** (source_turn + source_te_id stamped on every write)
- **G2:** Flat blob injection — **fixed by A5** (sentence-boundary chunking for blocks > 500 chars)
- **G3:** Reactive retrieval — **fixed by A9** (proactive injection of top-3 chunks per user message)
- **G4:** No temporal decay — **to be addressed by A10/A11**
- **G5:** Forked subagent memory — **to be addressed by A15**

---

## 2. Architecture Changes (Detailed)

### Phase 1: Provenance & Write-Ahead Verification ✅
- **A1:** `upsert_memory_block` truncates to `max_chars` and returns `was_truncated = true`. Handler surfaces `⚠️ WARNING`.
- **A2:** Ground Truth Protocol already in `BASE_SYSTEM_PROMPT`.
- **A3:** `stamp_provenance()` records turn counter + tool_call_id. Migration 12 adds `source_turn INTEGER`.

### Phase 2: Semantic Chunking & Rich Archival ✅
- **A4:** `get_long_term_excerpts` returns 250-char excerpts + top-5 keywords (already existed).
- **A5:** `memory_chunks` table (migration 13). `chunk_text()` splits at sentence boundaries with 50-char overlap. `rechunk_block()` called from all 4 meta_tools handlers. Per-chunk embeddings supported.
- **A6:** `search_memory` now also queries `memory_chunks` for chunk-level keyword hits.

### Phase 3: Proactive Retrieval & Context Assembly ✅
- **A7:** Priority-ordered greedy packing: P0 (identity) → P1 (orchestration) → P2 (skills) → P3 (pinned) → P4 (short) → P5 (long excerpts). Already existed.
- **A8:** Context overflow manifest with actionable recovery instructions (`load_skill()`, `search_memory()`). Already existed.
- **A9:** `recall_chunks()` searches `memory_chunks` against latest user message keywords. Top 3 injected as `# Recalled Context` section. Only runs on user messages (not tool returns).

### Phase 4: Temporal Decay & Freshness ✅
- **A10:** `access_count` + `last_access_turn` columns already exist (migration 9). `bump_block_access` increments both on every search hit. Already existed.
- **A11:** `recency_frequency_score()` computes `recency × frequency` composite: `frequency = 1 + log2(access_count + 1)`, `recency = 1 / (1 + turns_idle × 0.02)`. `search_memory` Phase 1 now ranks by this composite score instead of raw `updated_at`.
- **A12:** Server-side nudge already implemented: `ACTIVE_GOAL_NUDGE_INTERVAL = 5`, injects `⚠️` system message when `active_goal` stale. Already existed.

### Phase 5: Multi-Agent Consolidation ✅
- **A13:** Observation budget already scales by model context window (linear from 32k baseline). Turn offsets with `[turn N, Xm ago]` already in `render_observations_section`. Already existed.
- **A14:** Session eviction already uses 500-char `truncate_head_to` excerpts and backs up full evicted content to `archival_memory` with `evicted-session-summary` tag. Already existed.
- **A15:** `write_back_subagent_memory()` extracts custom memory blocks from the subagent before deletion. System blocks (persona, human, project, etc.) and skill blocks are excluded. Facts are written to parent with `subagent:` label prefix. `subagent_complete` SSE event includes `writeback_facts` count.

## 3. Test Coverage

24 new tests across Phases 1–5:
- Phase 1: 6 tests (3 truncation + 3 provenance)
- Phase 2: 7 tests (3 chunking logic + 3 chunk storage + 1 chunk search)
- Phase 3: 3 tests (keyword recall + empty query + deduplication)
- Phase 4: 4 tests (3 scoring math + 1 end-to-end ranking)
- Phase 5: 4 tests (custom block copy + system block exclusion + empty skip + zero-block)

Full workspace: 1,570+ tests, 0 failures, 0 clippy warnings.
