## 2026-04-12T02:45:00Z — Context Efficiency: P4-A Compaction Markers
**Summary:** Implemented compaction markers — DB-level sentinel messages (`role = 'compaction'`) that `get_context_window()` uses as a boundary to skip pre-summarized history. Addresses all 6 identified risks: LLM provider rejection (filtered in `db_row_to_llm`), FTS pollution (filtered in `search_messages`), consumer breakage (filtered in `list_messages_page`), recursive summarization (excluded via list filter), timestamp ordering (marker uses boundary message's timestamp), and backward compatibility (COALESCE falls back to 0 when no markers exist).
**Files modified:**
- `crates/cade-server/src/server/api/messages/persist.rs` — `db_row_to_llm()` returns empty vec for `role = "compaction"`
- `crates/cade-server/src/server/consolidation.rs` — Inserts compaction marker after writing session_summary, anchored to boundary message timestamp
- `crates/cade-store/src/sqlite/messages.rs` — `get_context_window()` SQL uses CTE boundary to scan only messages after latest marker; `list_messages_page()` excludes compaction markers; 4 new tests
- `crates/cade-store/src/sqlite/tools.rs` — `search_messages()` excludes compaction markers from FTS results
**Reason:** `get_context_window()` previously scanned ALL messages in the conversation (up to 500) on every request. With compaction markers, it only scans messages AFTER the most recent marker — drastically reducing the scan set for long sessions.
**Previous behavior:** Every `build_context()` call loaded and budgeted all messages from conversation start. Long sessions with 200+ messages had high DB query overhead.
**New behavior:** After Sleeptime consolidation runs, a `role = 'compaction'` sentinel is inserted at the boundary. Subsequent `get_context_window()` queries only scan messages inserted after that sentinel. Pre-marker messages remain in the DB for `conversation_search` recovery.
**Tests:** 4 new compaction marker tests (list exclusion, boundary stop, backward compat, multiple markers). 73 cade-store tests, 32 cade-server tests, 15 regression tests — all pass. Full cargo check clean.
**Rollback steps:** Revert to checkpoint `cp-1f990c6b` or remove compaction marker code from the 4 files.

## 2026-04-12T01:30:00Z — Context Efficiency: Full Phase 1-3 Implementation
**Summary:** Implemented all six planned context efficiency improvements (P1-A through P3-A). Changes derived from industry research comparing OpenCode, Gemini CLI, Aider, and MemGPT approaches.
**Files modified:**
- `crates/cade-server/src/server/consolidation.rs` — Structured 7-section compaction template (P1-A), inflation guard (P1-B), weak-model resolution for consolidation (P1-C)
- `crates/cade-server/src/server/api/messages/context.rs` — Proactive overflow signal at 80% usage (P2-B), surgical tool-output pruning integration (P2-A)
- `crates/cade-server/src/server/api/messages/mod.rs` — Per-tool output limits static map (P3-A)
- `crates/cade-store/src/sqlite/mod.rs` — DB migration v2: `compaction_model` column on `agents` table (P1-C)
- `crates/cade-store/src/sqlite/agents.rs` — `AgentRow.compaction_model` field, `update_agent_compaction_model()`, updated SELECTs
- `crates/cade-store/src/sqlite/messages.rs` — `compact_old_tool_outputs()` DB function (P2-A)
- `crates/cade-store/src/sqlite/{conversations,evidence,memory/tests,runs,tools}.rs` — `compaction_model: None` in all `AgentRow` test constructors
**Reason:** Industry research showed CADE's within-session token efficiency had gaps vs. competing agents. Six changes address: compaction quality (structured template), safety (inflation guard), cost (weak model), proactiveness (80% threshold), context reclamation (surgical pruning), and proportional limits (per-tool caps).
**Previous behavior:** Free-form consolidation prompt; no inflation guard; consolidation on main model only; reactive-only overflow detection; no surgical tool-output pruning; single global 8192-char tool result cap.
**New behavior:** Structured 7-section template; summaries ≥80% of source size rejected; configurable `compaction_model` per agent (falls back to main model); proactive consolidation at 80% context usage; old tool outputs beyond 120k-char protect window replaced with placeholder; per-tool output limit map (bash 4k, read_file 12k, grep 3k, memory 2k, default 8k).
**Tests:** 5 new inflation-guard unit tests, 2 compaction_model CRUD tests, 3 compact_old_tool_outputs tests. 69 cade-store tests pass, 32 cade-server tests pass, 15 regression tests pass. Full workspace cargo check clean.
**Rollback steps:** Revert via `git stash pop stash@{0}` from checkpoint `cp-d7ae709e` or revert the individual files.

## 2026-04-10T16:45:00Z — OpenRouter Architecture & Reasoning Stream Stability
**Summary:** Resolved severe stability, parsing, and context retention bugs when interfacing with OpenRouter and reasoning-capable models (e.g., qwen3.6-plus).
**Files modified:** `crates/cade-ai/src/openai.rs`, `crates/cade-cli/src/cli/repl/turn_loop/stream.rs`, `crates/cade-cli/src/cli/repl/turn_tools/runner.rs`, `crates/cade-server/src/server/api/messages/mod.rs`
**Reason:** The system panicked on SSE streams, stripped required model org prefixes resulting in 400 errors, failed to request reasoning tokens natively, discarded reasoning content from SQLite persistence, failed to flush reasoning to the TUI if the assistant returned no other content, and infinite-looped when encountering 429 rate limit errors.
**Previous behavior:** Crashed with slice indexing bounds panic; OpenRouter models failed to load; 429 errors created an infinite loop; reasoning streams were lost between turns.
**New behavior:** Safely parses SSE streams; injects `include_reasoning`, `HTTP-Referer`, and `X-Title` headers; preserves `google/` prefixes; flushes and persists reasoning streams in `<reasoning>` XML tags; exits gracefully on empty API responses.
**Rollback steps:** `git revert 0f3e290`
