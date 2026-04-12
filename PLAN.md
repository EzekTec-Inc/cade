## 2026-04-13T12:00:00Z — CADE-nvim Option B: Inline Completions Implementation
**Summary:** Implemented direct-HTTP inline code completions for the CADE-nvim Neovim plugin. Lua modules call the existing `POST /v1/agents/:id/complete` SSE endpoint — same backend as the VS Code extension — eliminating the MCP round-trip proposed in the original Option A plan.
**Files modified:**
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/config.lua` — NEW: defaults + user config merge (port, agent_id, debounce, hl_group, etc.)
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/ghost.lua` — NEW: extmark ghost-text renderer (virt_text inline for line 1, virt_lines below for remaining)
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/http.lua` — NEW: async curl SSE client via vim.system with cancel() support
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/trigger.lua` — NEW: debounced TextChangedI/CursorMovedI handler with in-flight cancellation
- `~/.local/share/nvim/lazy/CADE-nvim/lua/cade/init.lua` — NEW: public API (setup, accept, accept_line, accept_word, dismiss, toggle)
- `~/.local/share/nvim/lazy/CADE-nvim/plugin/cade.lua` — Extended: append autocmds + keymaps for completions
- `~/.config/nvim/lua/plugins/cade.lua` — NEW: lazy.nvim plugin spec pointing to local CADE-nvim directory
- `CADE-nvim-completions-plan-B.md` — NEW: Option B implementation plan document
**Reason:** The original Option A plan proposed adding completion tools to the MCP server.py and having CADE orchestrate completions via MCP. With the `/v1/complete` endpoint and VS Code extension already built, Option B avoids the MCP round-trip by having Neovim Lua call the HTTP endpoint directly — consistent with the VS Code architecture and lower latency.
**Previous behavior:** CADE-nvim had only socket setup + 3 MCP intercept tools (ide_read_buffer, ide_propose_edit, ide_apply_patch). No code completion support. Plugin was not loaded by lazy.nvim.
**New behavior:** Ghost-text completions appear after 300ms debounce, streamed incrementally via SSE. Accept with Tab (full), C-] (line), M-] (word), or dismiss with C-e. Toggle on/off with leader-ct. All keymaps use expr=true to pass through when no completion is visible.
**Tests:** All 5 Lua modules load cleanly. 3 autocmds registered (TextChangedI, CursorMovedI, InsertLeave). 4 insert-mode keymaps + 1 normal-mode keymap verified. Ghost state functions return correct defaults. Toggle flips enabled state. Full Neovim startup produces no errors.
**Rollback steps:** `cd ~/.local/share/nvim/lazy/CADE-nvim && git reset --hard HEAD~1` and `rm ~/.config/nvim/lua/plugins/cade.lua`

## 2026-04-12T04:15:00Z — Context Efficiency: Polishing P5-B and P4-C
**Summary:** Added proactive consolidation trigger for length (P5-B) and blocking endpoint test coverage (P4-C).
**Files modified:**
- `crates/cade-server/src/server/api/messages/context.rs` — Set `needs_consolidation` if post-marker turns exceed 20, improving summarization sensitivity.
- `crates/cade-server/src/server/api/messages/tests.rs` — Added test to ensure blocking endpoint respects proactive consolidation limits.
**Reason:** Prevent context token bloat in long conversations that have not yet reached the 80% token utilization threshold, and solidify testing coverage.
**Tests:** Existing 129 tests passed cleanly.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-12T03:30:00Z — Context Efficiency: P4-B to P6-B (Completion)
**Summary:** Finalized the remaining context efficiency phases. Reflection (`/reflect`) now respects compaction boundaries (P5-A); `session_summary` is forced to remain pinned across restarts (P5-C); `conversation_search` identifies pre-compaction snippets (P4-B); metrics for efficiency tracking were exposed via `/v1/agents/:id/metrics` (P6-A); and `compaction_model` configuration was exposed via the CLI (`/compaction-model`) and API (P6-B).
**Files modified:**
- `crates/cade-server/src/server/reflection.rs` — Uses `get_context_window` to avoid redundant reflection on compressed history.
- `crates/cade-server/src/server/consolidation.rs` — Sets `session_summary` tier to `pinned`.
- `crates/cade-store/src/sqlite/tools.rs` — Appends note to FTS snippets before compaction markers.
- `crates/cade-server/src/server/state.rs` & `crates/cade-server/src/server/api/agents.rs` — Added `AgentMetrics` and exposed endpoint.
- `crates/cade-tui/src/menu.rs` & `crates/cade-cli/src/cli/repl/slash.rs` — CLI `/compaction-model` command.
**Reason:** Addressed operational gaps identified post-P4-A (stale history scanning, lost session continuity, missing observability, and missing UX for configuration).
**Tests:** Existing 129 tests passed cleanly.
**Rollback steps:** `git revert c81c742`

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

## 2026-04-12T18:21:00Z — cade.nvim: agent_id settings.json fallback
**Summary:** `config.lua` now falls back to `~/.cade/settings.json → last_agent` when `$CADE_AGENT_ID` is unset, making the plugin zero-config for users who already run the CADE TUI.
**Files modified:**
- `plugins/cade.nvim/lua/cade/config.lua` — Added `resolve_agent_id()` function: checks env var first, then reads and decodes `~/.cade/settings.json`, falls back to `""`. `setup()` accepts internal `_settings_path` key for test injection.
- `plugins/cade.nvim/spec/minimal_init.lua` — New. Minimal test init that adds lua/ to rtp and prevents plugin/cade.lua serverstart conflict.
- `plugins/cade.nvim/spec/config_spec.lua` — New. 3 plenary tests: file fallback, env-var priority, missing file graceful fallback.
**Previous behavior:** `agent_id` defaulted to `$CADE_AGENT_ID` only; plugin was silent/inert when the env var was unset.
**New behavior:** `agent_id` resolves via `$CADE_AGENT_ID → settings.json.last_agent → ""`.
**Tests:** 3/3 pass (plenary busted).
**Rollback steps:** Restore `config.lua` from commit `470989d`.

## 2026-04-12T18:35:00Z — cade.nvim: :CadeStatus command
**Summary:** Added `require("cade").status()` function and `:CadeStatus` user command. Displays completion status, agent ID, server reachability (via sync curl probe), API key presence, debounce, and current filetype.
**Files modified:**
- `plugins/cade.nvim/lua/cade/init.lua` — Added `_probe_server()` (uses `vim.system` sync curl) and `status()` (builds info string, calls `vim.notify()`). `_probe_server` is overridable for test injection.
- `plugins/cade.nvim/plugin/cade.lua` — Registered `CadeStatus` user command.
- `plugins/cade.nvim/spec/status_spec.lua` — New. 3 plenary tests: field presence, reachable icon, unreachable icon.
**Previous behavior:** No way to check plugin state or server reachability.
**New behavior:** `:CadeStatus` displays a formatted status block in `vim.notify()`.
**Tests:** 6/6 pass (3 config + 3 status).
**Rollback steps:** Revert `init.lua` and `plugin/cade.lua` from commit `470989d`.
