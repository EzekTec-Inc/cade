# CADE Memory Architecture Rework Spec

> **Status:** Phases 1–3 COMPLETE, Phases 4–5 pending
> **Target:** Address structural amnesia and hallucination root causes.

## Implementation Status

| Phase | Items | Status |
|-------|-------|--------|
| **Phase 1** | A1 (truncation), A2 (ground truth), A3 (provenance) | ✅ DONE |
| **Phase 2** | A4 (rich excerpts), A5 (chunking), A6 (chunk search) | ✅ DONE |
| **Phase 3** | A7 (greedy packing), A8 (overflow manifest), A9 (proactive injection) | ✅ DONE |
| **Phase 4** | A10 (access tracking), A11 (decay scoring), A12 (staleness nudge) | ⏳ Next |
| **Phase 5** | A13 (expanded observations), A14 (session eviction), A15 (subagent write-back) | ⏳ Pending |

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

### Phase 4: Temporal Decay & Freshness (Next)
- **A10:** Add `access_count` + `last_reinforced_at` to memory blocks. (Schema columns already exist from migration 9.)
- **A11:** Modify search scoring to weight Recency × Frequency alongside cosine similarity.
- **A12:** Server-side nudge: inject system warning if `active_goal` stale for 5+ tool calls.

### Phase 5: Multi-Agent Consolidation (Pending)
- **A13:** Expand observation trail to 50 obs/4000 chars (128k) or 75 obs/6000 chars (200k). Add turn offsets.
- **A14:** Improve session eviction: 500-char `session_index` excerpts + archival_memory backup.
- **A15:** Subagent write-back: background reflection pass extracts typed facts to parent memory.

## 3. Test Coverage

16 new tests across Phases 1–3:
- Phase 1: 6 tests (3 truncation + 3 provenance)
- Phase 2: 7 tests (3 chunking logic + 3 chunk storage + 1 chunk search)
- Phase 3: 3 tests (keyword recall + empty query + deduplication)

Full workspace: 1,570+ tests, 0 failures, 0 clippy warnings.
