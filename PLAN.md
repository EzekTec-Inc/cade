## 2026-04-17T00:20:00Z — M5: Closed as already-implemented

**Task:** Expose `AgentMetrics` via an HTTP endpoint.

**Discovery:** The endpoint `GET /v1/agents/:id/metrics` already exists:
- Route: `crates/cade-server/src/server/api/mod.rs:76`
- Handler: `crates/cade-server/src/server/api/agents.rs::get_agent_metrics` (lines 260–268)
- Returns `state.agent_metrics[agent_id]` as JSON; `AgentMetrics` derives `serde::Serialize`.
- All five counters are incremented in production code:
  - `tool_outputs_compacted` — `context.rs:388`
  - `consolidation_runs`, `chars_summarised`, `chars_produced` — `consolidation.rs:511-513`
  - `inflation_guard_hits` — `consolidation.rs:340`
- M3's eager-consolidation path calls `consolidate_agent` which already bumps `consolidation_runs`, so no additional metric wiring is needed.

**Decision:** User chose to close M5 as done rather than add test coverage or 404-on-unknown behaviour. The 5-task context-loss fix (M4 → M2 → M1-revised → M3-revised → M5) is now complete.

**Files modified:** none.

**Rollback:** N/A.

---

## 2026-04-17T00:15:00Z — M3-revised: Lower idle threshold + eager turn-count trigger

**Task:** Close the gap where interactive sessions never cross the 60-second idle timer between turns, leaving consolidation un-triggered until context had already overflowed. Lower the Sleeptime idle threshold 60 s → 20 s, and add an eager trigger that fires consolidation every N turns (configurable via `EAGER_CONSOLIDATION_TURN_THRESHOLD = 20`) when `needs_consolidation` is set.

**Files modified:**
- `crates/cade-server/src/server/consolidation.rs`
  - Added `pub(crate) const EAGER_CONSOLIDATION_TURN_THRESHOLD: i64 = 20`.
  - Added `pub(crate) fn should_eager_consolidate(current, last, threshold) -> bool` (pure, saturating).
  - Added 7 `m3_*` unit tests.
- `crates/cade-server/src/server/state.rs`
  - Added `pub last_consolidation_turn: i64` field to `AgentActivity`.
  - Updated doc comment to reflect 20 s idle + eager turn-count path.
- `crates/cade-server/src/server/api/messages/context.rs`
  - Added eager-trigger block inside the existing `omitted_turns > 0 || needs_proactive…` branch:
    - Reads `sqlite::get_turn_counter` under the `agent_activity` write lock.
    - If `should_eager_consolidate(current, entry.last_consolidation_turn, THRESHOLD)` is true:
      - Stamps `entry.last_consolidation_turn = current`.
      - Clears `entry.needs_consolidation` (so Sleeptime doesn't re-fire).
      - Spawns `consolidate_agent` via `tokio::spawn` after the lock is released.
  - Added `last_consolidation_turn: 0` to the existing `AgentActivity` literal.
- `crates/cade-server/src/server/api/messages/mod.rs`
  - Added `last_consolidation_turn: 0` to the two existing `AgentActivity` literals (send_message + stream_message).
- `src/bin/cade-server.rs`
  - Lowered Sleeptime idle threshold 60 → 20 seconds.
  - Updated block comment.

**Reason:** Before M3, consolidation relied solely on the 60-second Sleeptime timer. A continuous interactive session (short pauses between turns) could easily complete 80+ turns without triggering the timer — `promote_stale_blocks` would then demote `working_set` and `session_summary` to `long` before consolidation could pin them. M1 partially addressed this for `working_set`; M3 closes the remaining gap by guaranteeing consolidation fires at least once per 20 turns when dropped turns occur.

**Previous behaviour:**
- Sleeptime task fired consolidation only after 60 s of agent inactivity.
- No turn-count-driven trigger.

**New behaviour:**
- Sleeptime task fires after 20 s of inactivity.
- An eager consolidation spawns from `build_context` whenever:
  - Older turns were dropped (`omitted_turns > 0` or proactive signal), AND
  - The turn counter has advanced ≥ 20 turns since the last eager run for this agent.
- Decision is made under the `agent_activity` write lock → concurrent requests cannot double-fire.

**Test results:**
- `cargo test -p cade-server` → 113/113 pass (+7 new M3 tests).
- `cargo test -p cade-store --lib` → 95/95 pass.
- `cargo test --test context_memory_regression` → 15/15 pass.
- `cargo build --workspace` → clean.
- `cargo clippy -p cade-server --lib` → no new warnings.
- M4 round-trip and all M1/M2 tests remain green.

**Security / privacy review (tdd-guide §3–5):**
- `should_eager_consolidate` and `EAGER_CONSOLIDATION_TURN_THRESHOLD` are `pub(crate)`; no new public surface.
- `current_turn` is read from the DB (`agents.memory_turn_counter`, an `i64` counter controlled by the server); no user data.
- **Race-safety (§5.2):** eager-trigger decision is made under the same `agent_activity.write()` lock that updates the state, so two concurrent requests for the same agent serialize — the second observes the updated `last_consolidation_turn` and correctly returns `false`.
- **Resource cap:** a given agent can spawn at most one eager `consolidate_agent` every 20 turns regardless of request rate; the `tokio::spawn` is not unbounded per-agent.
- No PII in logs — `tracing::info!` only includes the opaque `agent_id`.

**Rollback steps:**
1. `git checkout -- crates/cade-server/src/server/consolidation.rs \
      crates/cade-server/src/server/state.rs \
      crates/cade-server/src/server/api/messages/context.rs \
      crates/cade-server/src/server/api/messages/mod.rs \
      src/bin/cade-server.rs`
2. Or revert this commit once committed.

---
## 2026-04-17T00:10:00Z — M1-revised: Auto-pin `working_set` on first non-empty write

**Task:** Close the race where `working_set` could be demoted to `long` tier by `promote_stale_blocks` before `consolidate_agent` had a chance to pin it. Modify `upsert_memory_block` so that writing a non-empty value to label `working_set` promotes the block to `pinned` tier in the same write.

**Files modified:**
- `crates/cade-store/src/sqlite/memory.rs`
  - Added `is_nonempty_working_set` flag (`label == "working_set" && !final_value.trim().is_empty()`).
  - UPDATE path: dynamic `tier_sql` — `'pinned'` when flag set, else existing `CASE WHEN tier = 'pinned' THEN 'pinned' ELSE 'short' END`.
  - INSERT path: dynamic `insert_tier` — `"pinned"` when flag set, else `"short"`.
- `crates/cade-store/src/sqlite/memory/tests.rs` — appended 5 `m1_*` unit tests:
  - `m1_working_set_auto_pins_on_first_nonempty_write`
  - `m1_working_set_empty_seed_stays_short`
  - `m1_working_set_whitespace_only_value_stays_short`
  - `m1_other_labels_are_not_auto_pinned`
  - `m1_working_set_remains_pinned_on_subsequent_writes`

**Reason:** The original design seeds `working_set` as `short` so it can age out when the agent moves to a new task. Pre-M1, the agent writing real task state (e.g. `update_memory(label="working_set", value=…)`) left the block in `short` tier — a long interactive session without consolidation firing could then archive the block via `promote_stale_blocks` (threshold 80 turns) before `consolidate_agent` re-pinned it.

**Previous behaviour:**
- First non-empty write to `working_set` → block tier remained `short`.
- Block relied on `consolidate_agent` at line 333 to later re-pin it — race window open for up to 80 idle turns.

**New behaviour:**
- First non-empty write to `working_set` → block tier set to `pinned` immediately.
- Empty / whitespace-only values leave the tier at `short` (preserves `r06_working_set_is_short_not_pinned` and `DEFAULT_MEMORY_BLOCKS` seeding invariant).
- Other labels unchanged — auto-pin rule is scoped to `working_set` only.

**Test results:**
- `cargo test -p cade-store` → 95/95 pass (+5 new M1 tests).
- `cargo test --test context_memory_regression` → 15/15 pass (`r06_working_set_is_short_not_pinned` still green).
- `cargo test -p cade-server` → 106/106 pass (M4 round-trip still green).
- `cargo build --workspace` → clean.

**Security / privacy review (tdd-guide §3–4):**
- No new public-facing surface; label `"working_set"` is a compile-time string literal, not user input.
- `format!` builds SQL from two fixed string literals (`"'pinned'"` and the prior `CASE` expression); no user-controlled data enters SQL. Bind params retained. No injection risk.
- No changes to logs, error messages, or PII handling.

**Rollback steps:**
1. `git checkout -- crates/cade-store/src/sqlite/memory.rs crates/cade-store/src/sqlite/memory/tests.rs`
2. Or revert this commit once committed.

---
## 2026-04-17T00:05:00Z — M2: Per-role preview limits + drop noisy-tool filter

**Task:** Replace the flat 600-char preview cut in `consolidate_agent` with per-role limits (assistant 1200 / tool 800 / user 400) so the summariser sees full assistant technical content. Also drop the `len < 15 && no-digit && no-slash` noisy-tool-skip heuristic, which was incorrectly dropping short legitimate confirmations like `"ok"` and `"done"`.

**Files modified:**
- `crates/cade-server/src/server/consolidation.rs`
  - Added helpers `preview_limit_for_role(role: &str) -> usize` and `should_skip_noisy_tool(_role: &str, _trimmed: &str) -> bool`.
  - Replaced inline 600-char truncation with `preview_limit_for_role(role)`.
  - Replaced inline `len < 15 && …` skip with `should_skip_noisy_tool(role, trimmed)` (now returns `false` always; placeholder for future heuristics).
  - Updated section-3 doc comment from "600-char preview cut" to "per-role preview cut".
  - Added 7 unit tests (`m2_*`).

**Reason:** Assistant turns were losing file-edit detail (>600 chars) before the summariser saw them. Short tool confirmations like `"ok"` were being silently discarded, making the summariser believe those tools never ran. User chose to drop the filter entirely (vs. tightening the threshold) in the clarification turn — `MAX_SUMMARY_INPUT_CHARS = 24_000` is the sole remaining safeguard.

**Previous behaviour:**
- Flat 600-char cap on every message regardless of role.
- Tool messages with `len < 15 && !contains('/') && !any_ascii_digit` were skipped.

**New behaviour:**
- Per-role limits: assistant → 1200, tool → 800, user/other → 400.
- Tool noisy-skip filter removed (function now always returns `false`; empty/whitespace-only content already filtered earlier by `trimmed.is_empty()`).

**Test results:**
- `cargo test -p cade-server` → 106/106 pass (+7 new M2 tests).
- `cargo test --test context_memory_regression` → 15/15 pass.
- M4 round-trip test still green → pipeline behaviour unchanged from caller's perspective.

**Rollback steps:**
1. `git checkout -- crates/cade-server/src/server/consolidation.rs`
2. Or revert this single commit once committed.

**Notes:**
- `should_skip_noisy_tool` is intentionally kept as a function (not inlined) to preserve a named extension point for future noise heuristics without re-touching the hot path.
- `preview_limit_for_role` uses a `match` rather than a `HashMap` to stay allocation-free in the inner loop (rust10x lean-deps/zero-alloc guidance).

---
## 2026-04-17T00:00:00Z — M4: End-to-end consolidation round-trip regression test

**Task:** Protect the pipeline `dropped turns → consolidate_agent → session_summary written → pinned` with a regression test that exercises the real code path via an in-process mock LLM.

**Files modified:**
- `crates/cade-server/Cargo.toml` — added `async-trait.workspace = true` to `[dev-dependencies]`
- `crates/cade-server/src/server/consolidation.rs` — appended to existing `mod tests`:
  - `MockSummaryLlm` struct implementing `LlmProvider`
  - Helpers `mk_state()` and `seed_turns()`
  - Test `m4_consolidation_round_trip_writes_pinned_session_summary`

**Reason:** Prior to M4, no test verified that `consolidate_agent` actually writes a usable, pinned `session_summary` block. Rotation, turn-grouping, and inflation-guard pieces were covered in isolation but the end-to-end contract was unverified. This closes that gap before refactors touch the pipeline.

**Previous behaviour:** 98 tests in `cade-server`. Consolidation round-trip was only validated manually.

**New behaviour:** 99 tests in `cade-server` (+1). Test asserts:
1. `LlmProvider::complete` called exactly once when dropped turns exist.
2. `session_summary` block contains the mocked summary verbatim.
3. `session_summary` block ends up in `pinned` tier (survives `promote_stale_blocks`).

**Test results:** `cargo test -p cade-server` → 99/99 pass. `cargo test --test context_memory_regression` → 15/15 pass. No regressions.

**Rollback steps:**
1. `git checkout -- crates/cade-server/Cargo.toml crates/cade-server/src/server/consolidation.rs`
2. Or restore checkpoint `cp-5fa830c4-d999-4971-84ce-60a2fbeabf82` (label `M4-before-failing-test`).

**Checkpoint ID:** `cp-5fa830c4-d999-4971-84ce-60a2fbeabf82` (label: `M4-before-failing-test`).

---
## 2026-04-16T01:15:00Z — feat: install_skill supports bare repo URLs and skill selection

**Summary:** Enhanced `install_skill` tool to support the `npx skills add` ecosystem pattern. Users can now install skills from bare GitHub repo URLs (e.g., `https://github.com/github/awesome-copilot`) and `owner/repo` shorthand by providing a `skill` parameter to select which skill to install from a multi-skill repository.

**Files modified:**
- `crates/cade-core/src/skills/watcher.rs` — Added `resolve_github_repo_skill_url()` function; updated `install_skill_from_url()` signature to accept `skill_name: Option<&str>`; added resolution chain: repo+skill → tree/blob → direct URL
- `crates/cade-core/src/skills/tests.rs` — Added 8 new tests for `resolve_github_repo_skill_url` (bare URL, shorthand, trailing slash, missing skill, non-GitHub, invalid owner/repo, path traversal)
- `crates/cade-agent/src/tools/meta.rs` — Added `skill` parameter to `install_skill` tool schema
- `crates/cade-agent/src/tools/runtime/skills.rs` — Extract and pass `skill` parameter to `install_skill_from_url()`

**Previous behavior:** `install_skill` only accepted GitHub tree/blob URLs or direct SKILL.MD URLs. Bare repo URLs like `https://github.com/github/awesome-copilot` would fail.
**New behavior:** `install_skill(url="https://github.com/github/awesome-copilot", skill="rust-mcp-server-generator")` resolves to the raw SKILL.md URL and installs it. Also supports `owner/repo` shorthand.
**Rollback:** Revert commit or restore checkpoint `before-install-skill-enhancement`.

## 2026-04-12T21:09:00Z — TUI: Nerd Font icons for tool calls and results

**Summary:** Added Nerd Font glyph icons for all tool call types (bash, file read/write, git, GitHub, memory, skills, subagents, web, etc.) and tool result status badges (success/error). Icons render automatically when `use_nerd_fonts` is true (default). Falls back to plain ASCII/Unicode (`▶`, `✓`, `✗`) when disabled.
**Files modified:**
- `crates/cade-tui/src/icons.rs` — NEW: const icon map with `tool_icon()`, `success_icon()`, `error_icon()` functions + 5 unit tests
- `crates/cade-tui/src/lib.rs` — registered `icons` module
- `crates/cade-tui/src/app/mod.rs` — added `use_nerd_fonts: bool` field to `TuiApp`, threaded `nerd` through `render_frame` call and test callsites
- `crates/cade-tui/src/app/render.rs` — added `nerd: bool` param to `render_frame`, passed through to timeline rendering
- `crates/cade-tui/src/app/state.rs` — passed `use_nerd_fonts` to `visual_rows_with_state`
- `crates/cade-tui/src/app/timeline/render_item.rs` — `render_tool_call_item` uses `tool_icon()` instead of hardcoded `"▶ TOOL "`; `render_tool_result_item` uses `success_icon()`/`error_icon()`
- `crates/cade-tui/src/app/timeline/mod.rs` — threaded `nerd: bool` through `render_into`, `visual_rows`, `render_with_state`, `visual_rows_with_state`, `prepare_timeline_entries`
**Reason:** Nerd Font icons provide instant visual differentiation of tool call types without reading the tool name.
**Previous behavior:** All tool calls showed `▶ TOOL <name>(...)`. Results showed `✓ OK` / `✗ ERR`.
**New behavior:** Tool calls show a type-specific Nerd Font icon (e.g. `` for bash, `` for file read, `` for git). Results show `` / `` in nerd mode. ASCII fallback preserved when `use_nerd_fonts = false`.
**Tests:** 26/26 cade-tui tests pass (5 new icon tests). Binary size unchanged (15M release).
**Rollback steps:** `git revert HEAD`

## 2026-04-12T20:51:00Z — TUI: Rounded borders on all bordered panels

**Summary:** Applied `BorderType::Rounded` to all 9 `Borders::ALL` callsites across the TUI. Sidebar panels (`Borders::LEFT` only) intentionally left unchanged — rounding a single edge produces broken glyphs.
**Files modified:**
- `crates/cade-tui/src/overlay.rs` — overlay shell border
- `crates/cade-tui/src/app/mod.rs` — added `BorderType` to ratatui widget import
- `crates/cade-tui/src/app/render.rs` — Todos/plan panel border + added `BorderType` import
- `crates/cade-tui/src/app/layout/toast.rs` — toast notification border
- `crates/cade-tui/src/app/layout/pickers.rs` — theme picker table + filter borders
- `crates/cade-tui/src/skills.rs` — skills table + preview borders
- `crates/cade-tui/src/mcp_picker.rs` — MCP servers table + config preview borders
**Reason:** Rounded borders (╭╮╰╯) are the modern TUI standard; sharp borders (┌┐└┘) look dated.
**Previous behavior:** All bordered blocks used default sharp corners (`BorderType::Plain`).
**New behavior:** All `Borders::ALL` blocks use `BorderType::Rounded`. `Borders::LEFT`-only sidebar blocks unchanged.
**Tests:** 14/14 cade-tui tests pass. Binary size unchanged (15M release).
**Rollback steps:** `git revert 596a208`

## 2026-04-12T20:51:00Z — TUI: PageUp/PageDown viewport-aware scroll

**Summary:** Added `PageUp`/`PageDown` key handlers to the main conversation timeline. Scroll step equals the actual viewport content height (terminal height minus fixed UI rows), matching user expectation for page-based navigation. Extracted `scroll_page_up()` and `scroll_page_down()` pure functions with 7 unit tests covering all edge cases.
**Files modified:**
- `crates/cade-tui/src/app/input.rs` — Added `PageUp`/`PageDown` match arms in `handle_key_input`; added `scroll_page_up()`/`scroll_page_down()` helper functions; added 7 new unit tests; imported `FIXED_ROWS`/`MAX_INPUT_ROWS` constants.
**Reason:** Existing scroll keys (`K`=+10 lines, `J`=snap to bottom) are coarse. PageUp/PageDown provide standard, viewport-proportional scrolling with no keystroke collision risk.
**Previous behavior:** Only `Shift+K` (+10 lines), `Shift+J` (snap to bottom), and mouse wheel (±1 line) for timeline scrolling.
**New behavior:** `PageUp` scrolls up by one viewport height. `PageDown` scrolls down by one viewport height; reaching scroll=0 re-enables auto-follow. Viewport height = terminal rows − FIXED_ROWS − MAX_INPUT_ROWS.
**Tests:** 7 new tests (page_up from_bottom, already_scrolled, zero_viewport; page_down to_bottom, partial, already_at_bottom, zero_viewport). 21/21 cade-tui tests pass. Binary size unchanged (15M).
**Rollback steps:** `git revert HEAD`

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

## 2026-04-12T19:10:00Z — cade.nvim: ghost.lua test coverage
**Summary:** Added 9 plenary tests covering all public functions in ghost.lua. No implementation changes — tests confirm existing behaviour is correct.
**Files modified:**
- `plugins/cade.nvim/spec/ghost_spec.lua` — New. 9 tests: show() state tracking, show(nil/empty) no-op guards, clear() full reset, accept() no-pending guard, accept() full buffer insertion, accept_line() multi-line partial, accept_line() single-line clear, accept_word() leading-space inclusion.
**Previous behavior:** ghost.lua had zero test coverage.
**New behavior:** All 9 ghost behaviours verified. 9/9 pass.
**Rollback steps:** Delete `spec/ghost_spec.lua`.

## 2026-04-12T19:25:00Z — cade.nvim: http.lua test coverage + _parse_sse_line extraction
**Summary:** Extracted SSE parsing logic from the inline stdout callback into a public `_parse_sse_line()` pure function. Added 7 plenary tests covering all parse cases and fetch() guards.
**Files modified:**
- `plugins/cade.nvim/lua/cade/http.lua` — Added `M._parse_sse_line(line)` pure function (stream_delta, [DONE], error, nil-for-noise). Rewired stdout callback to call it. Zero behaviour change.
- `plugins/cade.nvim/spec/http_spec.lua` — New. 7 tests: 5 _parse_sse_line cases + fetch() empty-agent guard + fetch() cancel contract.
**Previous behavior:** SSE parsing was inline and untestable. http.lua had zero test coverage.
**New behavior:** All SSE parse logic testable in isolation. 7/7 pass. Full suite 22/22.
**Rollback steps:** Revert `http.lua` from commit `2482c51`. Delete `spec/http_spec.lua`.

## 2026-04-12T19:45:00Z — cade.nvim: completion latency telemetry
**Summary:** http.lua now records os.clock() timestamps for each fetch() call. status() / :CadeStatus displays a Latency line showing ttft (time-to-first-token) and total duration after at least one completion has fired.
**Files modified:**
- `plugins/cade.nvim/lua/cade/http.lua` — Added `M._last_request_at`, `M._last_first_token`, `M._last_done_at` module-level fields. Set in fetch(): request_at on entry, first_token on first delta, done_at on stream end or error.
- `plugins/cade.nvim/lua/cade/init.lua` — status() reads http telemetry fields and appends "Latency: ttft=Xms total=Xms" or "(no data)".
- `plugins/cade.nvim/spec/http_spec.lua` — +1 test: _last_request_at is a number after fetch() fires.
- `plugins/cade.nvim/spec/status_spec.lua` — +2 tests: Latency "(no data)" when no fetch, ttft=/total= when telemetry present.
**Previous behavior:** No timing data available. :CadeStatus showed no latency.
**New behavior:** After each completion, ttft and total latency visible in :CadeStatus. Full suite: 25/25.
**Rollback steps:** Revert `http.lua` and `init.lua`. Remove telemetry tests from specs.

## 2026-04-12T20:05:00Z — cade.nvim: customizable keymaps
**Summary:** Keymaps are now driven by config. Users can override individual keys or set keymaps=false to disable all bindings. plugin/cade.lua replaced hardcoded imap calls with a config-driven loop.
**Files modified:**
- `plugins/cade.nvim/lua/cade/config.lua` — Added `keymaps` table to M.defaults with 5 keys: accept, accept_line, accept_word, dismiss, toggle. Defaults match previous hardcoded values.
- `plugins/cade.nvim/plugin/cade.lua` — Replaced 5 hardcoded keymap calls with a loop over cfg.keymaps. Guards: `if cfg.keymaps ~= false` for the block, `if lhs` per binding (nil keys are skipped).
- `plugins/cade.nvim/spec/config_spec.lua` — +3 tests: default keys present, partial merge, keymaps=false.
**Previous behavior:** Keymaps were hardcoded. No way to remap or disable without editing the plugin file.
**New behavior:** Pass keymaps={accept="<C-y>"} to override one key; keymaps=false to disable all. Full suite: 28/28.
**Rollback steps:** Revert `config.lua` and `plugin/cade.lua`. Remove keymap tests from config_spec.

---

## 2026-04-12 — TUI: Refactor sidebar into SidebarState

**Summary:** Eliminated the 21-argument `render_sidebar` free-function signature by introducing a `SidebarState<'a>` struct. Extracted three formatting helpers (`format_activity`, `format_context`, `format_plan_summary`) as `pub(crate)` methods on the struct, making them independently unit-testable without a Ratatui frame. Added 7 unit tests covering all formatting branches. Removed the `#[allow(clippy::too_many_arguments)]` suppressor from `render_sidebar`.

**Files modified:**
- `crates/cade-tui/src/app/layout/sidebar.rs` — Added `SidebarState<'a>` struct; rewrote `render_sidebar` signature to `(frame, area, &SidebarState, colors)`; added `#[cfg(test)]` module with 7 tests.
- `crates/cade-tui/src/app/render.rs` — Updated import to include `SidebarState`; replaced 19-argument `render_sidebar(...)` call with `SidebarState { .. }` construction + 4-argument call.

**Reason:** Argument bloat, mixed concerns (formatting logic coupled to frame rendering), and zero unit-test coverage on sidebar formatting logic.

**Previous behaviour → New behaviour:** Identical visual output. `render_sidebar` now delegates formatting to `SidebarState` methods rather than computing strings inline.

**Rollback:** `git revert HEAD` or restore checkpoint `cp-abe2880d` (label: `before-sidebar-refactor`).
- **Timestamp (UTC)**: 2026-04-13T15:34:30Z
- **Summary of change**: Fixed Gemini API payload errors when caching tool schemas.
- **Files modified**: `crates/cade-ai/src/utils.rs`, `crates/cade-ai/src/gemini.rs`, `crates/cade-ai/src/tests.rs`
- **Exact reason**: The Gemini backend rejects JSON schemas with lowercase `type` strings when generating cached content (though it accepts them directly on standard completions). The schemas are now converted to uppercase (e.g. `STRING`, `OBJECT`) to fix `Proto field is not repeating` errors.
- **Previous behavior**: `clean_gemini_schema` mapped schema types to lowercase strings.
- **New behavior**: `clean_gemini_schema` casts schema types to uppercase strings.
- **Rollback instructions**: Revert `crates/cade-ai/src/utils.rs` and `crates/cade-ai/src/tests.rs` using git checkout.
- **Timestamp (UTC)**: 2026-04-13T16:45:54Z
- **Summary of change**: Drafted a comprehensive TUI refactor plan inspired by pi-coding-agent.
- **Files modified**: `docs/tui-refactor-plan.md` (created)
- **Reason**: The user requested a review of pi-coding-agent's TUI and a refactor plan for CADE based on those takeaways.
- **Previous behavior**: N/A (new document)
- **New behavior**: The repository now contains a formal blueprint for modernizing the TUI architecture (IME support, overlay stack, pluggable editor, UI slots).
- **Rollback steps**: Remove `docs/tui-refactor-plan.md`.
- **Timestamp (UTC)**: 2026-04-13T17:02:35Z
- **Summary of change**: Drafted a concise implementation plan for the TUI refactor.
- **Files modified**: `docs/tui-refactor-implementation.md` (created)
- **Reason**: The user requested a concise implementation plan for the TUI refactor.
- **Previous behavior**: N/A
- **New behavior**: The repository contains a 4-phase implementation plan.
- **Rollback steps**: Remove `docs/tui-refactor-implementation.md`.
- **Timestamp (UTC)**: 2026-04-13T17:50:23Z
- **Summary of change**: Implemented Phase 1 of the TUI refactor (hardware cursor sync).
- **Files modified**: `crates/cade-tui/src/app/mod.rs`, `crates/cade-tui/src/app/render.rs`
- **Reason**: The user asked me to implement Phase 1 from the refactor plan.
- **Previous behavior**: The cursor was drawn purely visually by the TUI widget, meaning standard IMEs didn't know where to open candidate windows.
- **New behavior**: CADE now queries the exact visual coordinate of the input prompt during the render step, and emits a `crossterm::cursor::MoveTo(x,y)` command immediately after terminal flush.
- **Rollback steps**: Revert changes to `crates/cade-tui/src/app/mod.rs` and `crates/cade-tui/src/app/render.rs` using git checkout.
- **Timestamp (UTC)**: 2026-04-13T18:36:57Z
- **Summary of change**: Reviewed CADE's UI styling and formatting logic compared to pi-coding-agent.
- **Files modified**: None
- **Reason**: The user asked for a comparison of UI styling and formatting logic between CADE and pi-coding-agent, and to identify parts that can be adopted in CADE.
- **Previous behavior**: N/A
- **New behavior**: N/A
- **Rollback steps**: N/A
- **Timestamp (UTC)**: 2026-04-13T18:42:13Z
- **Summary of change**: Drafted a concise implementation plan for Phase 2 of the TUI refactor.
- **Files modified**: `docs/tui-style-builder-plan.md` (created)
- **Reason**: The user requested an implementation plan to adopt pi's clean theme builder patterns inside CADE.
- **Previous behavior**: N/A
- **New behavior**: The repository contains a migration blueprint for CADE's Style Builders.
- **Rollback steps**: Remove `docs/tui-style-builder-plan.md`.
- **Timestamp (UTC)**: 2026-04-13T18:46:23Z
- **Summary of change**: Migrated CADE TUI to use clean style builder methods (Phase 2).
- **Files modified**: `crates/cade-tui/src/colors.rs`, `crates/cade-tui/src/app/render.rs`, and 14 other layout/component files.
- **Exact reason**: The user requested that CADE adopt the clean theme builder patterns observed in `pi-coding-agent`'s UI architecture.
- **Previous behavior**: The UI rendering logic relied on over 150 verbose instances of `Style::default().fg(colors.token)`.
- **New behavior**: Extended `ThemeColors` with style builders for every semantic color token. Refactored the UI components to use the concise `colors.token()` builder format instead.
- **Rollback instructions**: Use `git checkout` to revert the changes to `crates/cade-tui/src`.
- **Timestamp (UTC)**: 2026-04-13T22:26:06Z
- **Summary of change**: Halted TUI refactoring after Phase 2 to prevent architectural bloat.
- **Files modified**: None (Decision logged)
- **Exact reason**: Evaluated Phase 3 (Pluggable Editor) and Phase 4 (Overlay Stack) and determined they introduce unnecessary complexity (dynamic dispatch, borrow checker event routing, two-pass layout engines) into CADE's performant monolithic Rust loop.
- **Previous behavior**: N/A
- **New behavior**: N/A
- **Rollback instructions**: N/A
- **Timestamp (UTC)**: 2026-04-13T22:38:55Z
- **Summary of change**: Drafted a new implementation plan for a `/summarize` TUI modal.
- **Files modified**: `docs/tui-summarize-plan.md` (created)
- **Reason**: The user requested a plan to implement a pi-coding-agent style summary modal using CADE's existing background consolidation system.
- **Previous behavior**: N/A
- **New behavior**: The repository contains a blueprint for an instantaneous, cost-free conversation summary overlay.
- **Rollback steps**: Remove `docs/tui-summarize-plan.md`.
- **Timestamp (UTC)**: 2026-04-13T23:16:53Z
- **Summary of change**: Implemented an instant, zero-cost `/summarize` TUI modal.
- **Files modified**: `crates/cade-tui/src/app/layout/summary.rs`, `crates/cade-tui/src/app/render.rs`, `crates/cade-tui/src/app/input.rs`, `crates/cade-tui/src/app/mod.rs`, `crates/cade-cli/src/cli/repl/slash.rs`, `crates/cade-cli/src/cli/repl/commands.rs`
- **Exact reason**: The user requested a summarize mechanism similar to pi-coding-agent but built natively using CADE's existing background consolidation system.
- **Previous behavior**: Users had no interactive way to view the background-computed session summary.
- **New behavior**: Typing `/summarize` instantly pulls the `session_summary` memory block from the local SQLite database and displays it in a floating scrollable modal. If the conversation is too short for a summary, a toast notification is shown instead.
- **Rollback instructions**: Revert the commit `feat(tui): implement instant zero-cost /summarize modal` using git.
- **Timestamp (UTC)**: 2026-04-14T00:31:43Z
- **Summary of change**: Drafted an implementation plan to repurpose the `/copy` command as a programmatic clipboard extractor, renaming the original mouse capture toggle to `/mouse`.
- **Files modified**: `docs/copy-command-plan.md` (created)
- **Reason**: The user asked for a plan to make CADE's `/copy` command behave like pi-coding-agent's, which grabs the last message and copies it to the clipboard using OSC 52 and native OS APIs.
- **Previous behavior**: N/A
- **New behavior**: The repository now contains a blueprint for the `/copy` command refactor.
- **Rollback steps**: Remove `docs/copy-command-plan.md`.
## 2026-04-16T01:41:00Z — fix: dual-store file corruption causing agent not auto-loaded

**Summary:** Fixed a critical bug where `SessionStore` and `SettingsManager` both read/wrote `.cade/settings.local.json` with disjoint schemas. Each `save()` overwrote the other's fields, causing agent identity loss across restarts and mid-session agent switches.

**Root cause:** Two independent structs (`Session` with `agent_id`, `conversation_id` etc. and `LocalSettings` with `last_agent`, `pinned_agents` etc.) shared the same JSON file. Last writer won, destroying the other's data.

**Files modified:**
- `crates/cade-agent/src/agent/session.rs` — Moved `SessionStore` from `settings.local.json` to `session.json`; added backward-compat migration from legacy file; added `ensure_gitignore_entry()` helper; 5 new tests
- `crates/cade-mcp/src/watcher.rs` — Added `session.json` to watched filenames
- `crates/cade-core/src/permissions/manager.rs` — Added `session.json` to security guard for config file edits
- `crates/cade-cli/src/cli/repl/commands_agents.rs` — `/agents` Switch and DeleteMany branches now call `session.set_agent()` alongside `settings.set_last_agent()`
- `src/bootstrap/agents.rs` — `--agent` and `--name` branches now persist to both stores; happy-path lookups cross-sync between stores
- `README.md`, `SECURITY.md`, `WINDOWS_SETUP.md` — Updated file layout references

**Previous behavior:** Agent identity was randomly lost depending on which store saved last. `/agents` switch didn't persist to session. `--agent`/`--name` flags were forgotten on restart. Cross-project agent switching could load wrong agent.
**New behavior:** Each store has its own file. All agent resolution branches persist to both stores. Happy-path lookups cross-sync so both stores stay consistent.
**Rollback:** Restore checkpoint `before-dual-store-fix` (cp-ad662ffb).


## 2026-04-16T02:05:00Z — docs: update CHANGELOG.md

**Summary:** Updated `CHANGELOG.md` to reflect the session persistence fixes, the UI interrupt message refactoring, and the security dependency updates.


## 2026-04-16T02:30:00Z — test: add dual-store coexistence integration test

**Summary:** Added integration test proving `SessionStore` (session.json) and `SettingsManager` (settings.local.json) coexist without data loss. Test exercises interleaved writes and reloads from both stores, verifying no cross-contamination or clobbering.

**Files modified:**
- `crates/cade-agent/src/agent/session.rs` — added `dual_store_coexistence_no_data_loss` test

**Reason:** Phase 4 of dual-store file corruption fix. Validates that the file separation introduced in Phase 1 truly prevents the original bug.
**Previous behavior:** No integration test existed for dual-store safety.
**New behavior:** 31 tests total (25 original + 6 session tests), all passing.
**Rollback:** Remove the test function from session.rs.


## 2026-04-16T03:15:00Z — feat(tui): UI/UX polish batch (4 items)

**Summary:** Four low-effort, high-impact UI/UX improvements:

1. **Toast auto-dismiss** — Toasts now expire after their TTL (3s default). Added `Toast::is_expired()`, hooked into `draw()`, the REPL idle input loop, and the turn-loop tick task.
2. **Footer token counter** — Cumulative session token count shown in the footer bar in compact form (e.g. "1.2k↑", "50k↑"). Added `session_tokens` field to TuiApp, `format_token_count()` helper, and REPL sync.
3. **Startup context summary** — On resume, fetches the `working_set` memory block and displays the first 3 lines as a "Context:" line in the startup banner.
4. **Command menu section headers** — `/help` menu headers now include trailing rule lines. Inline command palette shows `[Section]` tags when filtering.

**Files modified:**
- `crates/cade-tui/src/app/mod.rs` — `Toast::is_expired()`, auto-dismiss in `draw()`, `session_tokens` field, test
- `crates/cade-tui/src/app/input.rs` — toast-aware redraw in idle input loop
- `crates/cade-tui/src/app/render.rs` — `session_tokens` param, footer token rendering
- `crates/cade-tui/src/app/layout/helpers.rs` — `format_token_count()` + test
- `crates/cade-tui/src/app/layout/command_palette.rs` — section tag rendering
- `crates/cade-tui/src/menu.rs` — section header rule lines
- `crates/cade-cli/src/cli/repl/mod.rs` — token sync to TuiApp, startup context fetch
- `crates/cade-cli/src/cli/repl/turn_loop/agent.rs` — toast in tick redraw condition

**Previous behavior:** Toasts persisted until overwritten. No token count in footer. No context on startup. Section headers minimal.
**New behavior:** Toasts auto-dismiss after 3s. Footer shows "1.2k↑" token badge. Startup shows "Context: ..." from working_set. Section headers have visual rules.
**Tests:** 574 workspace tests, all passing. New: `test_toast_expires_after_ttl`, `test_format_token_count`.
**Rollback:** Restore checkpoint `before-ui-polish` (cp-412d3888).

## 2026-04-16T05:29:00Z — chore: dependency modernization (security audit fixes)

**Summary:** Upgraded transitive dependencies to resolve 4 `cargo audit` advisories (all transitive). Simplified MCP HTTP transport code by leveraging rmcp 1.4's native auth/header support.

**Upgrades:**
- `scraper` 0.19 → **0.26** — fixes `fxhash` (RUSTSEC-2025-0057) + `rand 0.8` (unsound)
- `ratatui` 0.29 → **0.30** — fixes `lru 0.12.5` (RUSTSEC-2026-0002, unsound) + drops `paste`
- `tui-textarea` 0.7 → **`tui-textarea-2` 0.10** — maintained fork compatible with ratatui 0.30
- `ansi-to-tui` 7 → **8** — compatible with ratatui 0.30 (uses ratatui-core)
- `crossterm` 0.28 → **0.29** — aligned with ratatui 0.30
- `rmcp` 0.2 → **1.4** — fixes `paste` (RUSTSEC-2024-0436, uses `pastey` instead)

**Files modified:**
- `Cargo.toml` — workspace dependency versions (ratatui, crossterm, ansi-to-tui, rmcp)
- `crates/cade-web/Cargo.toml` — scraper 0.19 → 0.26
- `crates/cade-tui/Cargo.toml` — tui-textarea → tui-textarea-2
- `crates/cade-mcp/Cargo.toml` — removed reqwest dep, added http crate
- `crates/cade-mcp/src/lib.rs` — rmcp API migration: unified HTTP transport, builder-pattern CallToolRequestParams, RawContent wildcard arm

**Remaining advisories (accepted risk):**
- `bincode 1.3.3` via syntect — no upstream fix, syntect 5.3.0 is latest
- `rand 0.8.5` via phf_generator → termwiz — platform-gated (not compiled for our target)

**Previous behavior:** 5 cargo audit warnings, separate SSE/Streamable-HTTP code paths in MCP client
**New behavior:** 2 audit warnings (accepted), unified HTTP transport with native auth support
**Rollback:** restore checkpoint `before-dep-upgrades` (cp-4d230378)

---

## 2026-04-16T17:45Z — Task 1 / P1-1: Mandatory authentication

**Summary:** Remove the silent no-op auth branch. Every non-health request now requires a valid `Authorization: Bearer <token>`. When `CADE_API_KEY` is unset, both server and CLI auto-bootstrap a shared persistent token at `~/.cade/api-token` (0o600).

**Files modified:**
- `crates/cade-server/src/server/api/auth.rs` — removed `None => return next.run(req).await`, now returns 401 when no key configured. Doc rewritten.
- `crates/cade-server/src/server/api/auth_test.rs` — new test module (4 tests) covering anonymous rejection, health exemption, valid and invalid tokens.
- `crates/cade-server/src/server/bootstrap.rs` — new module: re-exports cade-core token helpers.
- `crates/cade-server/src/server/mod.rs` — wired `pub mod bootstrap;`.
- `crates/cade-server/src/server/config.rs` — added `resolve_api_key()` private helper; `from_env_with_port` now calls it instead of reading `CADE_API_KEY` directly.
- `crates/cade-server/Cargo.toml` — added `getrandom` runtime dep and `tower` + `tempfile` dev-deps.
- `crates/cade-core/src/bootstrap_token.rs` — new shared module (~150 lines, 6 tests) implementing `default_token_path`, `load_or_create_token`, `read_existing_token`.
- `crates/cade-core/src/lib.rs` — wired `pub mod bootstrap_token;`.
- `crates/cade-core/Cargo.toml` — added `getrandom` workspace dep.
- `crates/cade-core/src/settings/resolver.rs` — `api_key()` now falls back to the shared bootstrap token (read-only if present, create-on-demand otherwise) so the CLI can reach its auto-spawned server on first run.

**Reason:** HIGH-severity finding in security review — with `CADE_API_KEY` unset, any localhost process (browser CSRF, other users on shared host, malicious extension) could hijack the agent, read memory, trigger bash tool execution, and pivot via the SSRF proxy. Auth is now mandatory by default.

**Previous behavior:** `auth_middleware` passed every request through when `config.api_key` was `None`. CLI errored with "No CADE_API_KEY" unless user set env/settings.

**New behavior:**
- Server: non-health requests rejected 401 when no token configured; auto-creates `~/.cade/api-token` on first startup.
- CLI: reads the same token file (creating it if missing) and uses it for `Authorization: Bearer`.
- `CADE_API_KEY` env var still overrides everything.
- `/v1/health` remains public.

**Tests:**
- `cargo test -p cade-server --lib server::api::auth::tests` — 4 green.
- `cargo test -p cade-core --lib bootstrap_token` — 6 green.
- `cargo test -p cade-core --lib` — 199 green.
- `cargo test -p cade-server --lib` — 62 green.
- `cargo build --workspace` — clean.

**New dependencies:**
- `getrandom` (workspace dep) added to cade-core and cade-server runtime deps.
- `tower` 0.5 + `tempfile` added to cade-server dev-deps only (already transitively present via axum).

**Rollback:** `restore_checkpoint cp-0e65ca6a-f36e-4a87-bc73-141aac431452` (label `pre-security-remediation`).

---

## 2026-04-16T18:18Z — Task 2 / P1-2: Global request body size limit (8 MiB)

**Summary:** Applied `DefaultBodyLimit::max(8 * 1024 * 1024)` at the Axum router root so every request body is capped at 8 MiB regardless of which extractor (or raw body access) a handler uses.

**Files modified:**
- `crates/cade-server/src/server/api/mod.rs` — imported `axum::extract::DefaultBodyLimit`, added `.layer(DefaultBodyLimit::max(8 * 1024 * 1024))` to the router; added test module wiring.
- `crates/cade-server/src/server/api/router_test.rs` — new test module (3 tests) covering oversize rejection (>8 MiB → 413), medium-body acceptance (3 MiB, between Axum default 2 MiB and our 8 MiB cap, must pass), and small-body acceptance (sanity).

**Reason:** HIGH-severity finding in security review — no explicit global body cap meant streaming / raw-body handlers (e.g. the proxy stream) could buffer unbounded data. Axum's `Json` extractor has an implicit 2 MiB default, but the project needed a uniform explicit cap across all routes for defense-in-depth.

**Previous behavior:** Only `Json` extractors capped requests (at Axum's 2 MiB default). Raw-body / streaming handlers had no limit.

**New behavior:** Every route enforces a uniform 8 MiB body cap; requests over the cap return 413 Payload Too Large. Bodies under the cap behave as before.

**Tests:**
- `cargo test -p cade-server --lib server::api::tests` — 3 green.
- `cargo test -p cade-server --lib` — 65 green (was 62, +3 new).

**New dependencies:** none (DefaultBodyLimit lives in axum, already a dep).

**Rollback:** `restore_checkpoint cp-0e65ca6a-f36e-4a87-bc73-141aac431452` reverts everything in the remediation chain. For task-level revert, delete the `DefaultBodyLimit` layer + import and remove `router_test.rs`.

---

## 2026-04-17T04:10Z — Phase C: `session_summary` rotating ring + `session_index` eviction trail

**Summary:** Implemented the `session_summary_N` rotating ring (cap=5) in `consolidation.rs` so that previous `session_summary` content is no longer discarded when a new consolidation pass would overflow `SESSION_SUMMARY_MAX_CHARS`. Old summaries rotate into long-tier blocks (`session_summary_1` … `session_summary_5`). When the ring fills, the oldest block's first non-empty line is appended to a pinned `session_index` block (FIFO-capped at 3 KB), then the evicted block is deleted.

**Files modified:**
- `crates/cade-server/src/server/consolidation.rs` —
  - Added 3 tunables: `SESSION_SUMMARY_RING_CAP = 5`, `SESSION_SUMMARY_ARCHIVED_MAX_CHARS = 2_000`, `SESSION_INDEX_MAX_CHARS = 3_000`.
  - Replaced the single-line "keep only the latest summary" discard branch in `consolidate_agent()` with a call to `rotate_and_archive_session_summary()` before overwriting the live block.
  - Added private helpers: `rotate_and_archive_session_summary` (AppState-facing shim), `rotate_and_archive_session_summary_db` (Db-only inner, unit-testable), `append_to_session_index_db` (FIFO line-buffer appender), `first_nonempty_line`, `sanitize_index_line`, `truncate_head_to` (tail-preserving char-safe truncation).
  - Added 11 unit tests under `#[cfg(test)] mod tests` — 6 pure-helper tests (truncation, line extraction, whitespace sanitization) and 5 DB-backed ring tests using `cade_store::sqlite::open(":memory:")` (rotation writes slot 1, empty input is noop, slot shifting, eviction trail to `session_index`, FIFO truncation of index, archived-slot char cap).

**Reason:** Before Phase C, when the combined `session_summary + new_summary` exceeded `SESSION_SUMMARY_MAX_CHARS`, the previous summary was silently dropped. Over long-running sessions this destroyed the narrative history of what was done 3+ consolidation cycles ago. Phase C preserves that history in a bounded, predictable way (hard cap: 5 blocks × 2 KB + 1 × 3 KB index = ~13 KB worst case) without schema changes.

**Previous behavior:** `combined.chars().count() > SESSION_SUMMARY_MAX_CHARS` → keep only the latest `summary`; prior content lost forever.

**New behavior:** Same overflow trigger → rotate the prior live value into `session_summary_1` (tail-preserved, capped at 2 KB, tier=long); shift existing `session_summary_N` to `session_summary_{N+1}` for N=4..1; if `session_summary_5` already existed, write its first non-empty line (max 200 chars, whitespace-collapsed) to the pinned `session_index` block (FIFO-evict oldest lines when >3 KB), then delete `session_summary_5`. The live `session_summary` continues to hold only the newest summary. All errors in the rotation path are logged at debug/warn and swallowed — rotation is strictly best-effort and cannot fail the main consolidation.

**Tests:**
- `cargo test -p cade-server --lib server::consolidation` → 31 green (20 pre-existing + 11 new).
- `cargo test -p cade-server` → 79 green, 0 failed.
- `cargo clippy -p cade-server --lib --tests` → no new warnings (only pre-existing ones in unrelated files).

**New dependencies:** none. Uses only existing `cade_store::sqlite` functions (`upsert_memory_block`, `delete_memory_block`, `get_memory_blocks`, `set_memory_tier`, `create_agent`, `open`).

**Schema changes:** none. All state lives in the existing `shared_memory_blocks` / `agent_memory_blocks` tables via standard labels.

**Rollback:** `git revert` the Phase C commit, or restore checkpoint `cp-e5832a63-fdf9-4294-b293-0109921b08d2` (label `before-phase-c-ring`). No migration needed — stray `session_summary_N` / `session_index` blocks on rollback are harmless (they simply stop being written/read).

---

## 2026-04-17T04:22Z — Task 3 / P1-3: SSRF proxy lockdown

**Summary:** Locked down `/v1/stream` so it can no longer be used as a server-side request forgery (SSRF) primitive. Every outbound URL now passes an explicit scheme + IP-literal + host-allow-list validator before any network I/O, the reqwest client is built with redirects disabled, and a 30-second total timeout bounds slow upstreams.

**Files modified:**
- `crates/cade-server/src/server/api/proxy.rs` — rewrote the handler to call `validate_outbound_url()` before any I/O; build `reqwest::Client` with `Policy::none()` for redirects and a 30 s timeout. Added public `validate_outbound_url()` fn returning `Result<Url, UrlRejection>`, public `UrlRejection` enum with `status()` and `message()` helpers. Introduced `ALLOWED_HOSTS_EXACT` (4 entries) and `ALLOWED_HOST_SUFFIXES` (3 entries) constants.
- `crates/cade-server/src/server/api/proxy_test.rs` — new test module, 19 unit tests (5 scheme, 5 IP-literal, 7 host allow/deny, 3 edge cases).

**Threat blocked:**
- `GET /v1/stream?url=file:///etc/passwd` → 400 bad scheme
- `GET /v1/stream?url=http://169.254.169.254/...` (cloud metadata) → 403 ip-literal-host
- `GET /v1/stream?url=http://127.0.0.1:8080/admin` (loopback) → 403 ip-literal-host
- `GET /v1/stream?url=http://[::1]/` (IPv6 loopback) → 403 ip-literal-host
- `GET /v1/stream?url=https://evil.com/` (arbitrary public host) → 403 host-not-allowed
- `GET /v1/stream?url=https://api.anthropic.com.evil.com/` (suffix-match bypass) → 403 host-not-allowed
- Redirect chain from allowed host → blocked host: upstream 302 is NOT followed; caller sees the 302 byte-stream but no second request is issued.

**Allow-list (initial):**
- Exact: `api.anthropic.com`, `api.openai.com`, `generativelanguage.googleapis.com`, `openrouter.ai`
- Suffix (matched via leading dot — `anthropic.com.evil.com` ≠ `*.anthropic.com`): `anthropic.com`, `openai.com`, `googleapis.com`

**Reason:** HIGH/CRITICAL-severity SSRF finding from the security review. The original handler accepted any URL from the query string and proxied it verbatim. An authenticated caller (or any prompt-injection path that reaches an agent tool-call emitting `/v1/stream?url=…`) could reach loopback services, cloud metadata endpoints, or arbitrary schemes.

**Previous behavior:** `stream_http_handler` called `client.get(&params.url).send().await` with zero URL validation and redirects auto-followed.

**New behavior:** Request is rejected before any I/O if the URL fails validation. Valid URLs are fetched with redirects disabled and a 30 s total timeout. The handler's public interface (GET, query param shape, streaming response) is unchanged for legitimate traffic.

**Tests:**
- `cargo test -p cade-server --lib server::api::proxy` → 19 green (all new).
- `cargo test -p cade-server` → 98 green (up from 79, +19).
- `cargo clippy -p cade-server --lib --tests` → no new warnings from proxy.rs (one `manual_contains` lint flagged during dev, fixed before commit).

**New dependencies:** none. Uses `reqwest::Url` (re-export of the `url` crate already pulled in via `reqwest`), `std::net::IpAddr` for IP-literal detection, and `reqwest::redirect::Policy` / `Client::builder()` for the hardened client.

**Rollback:** `restore_checkpoint cp-010fb43b-cf0b-4e1a-871e-db964a1684c6` (label `before-p1-3-ssrf`). For task-level revert: `git revert` the P1-3 commit — restores the pre-lockdown proxy handler. Note: reverting re-opens the SSRF vector.

**Known limitations (deferred):**
- **DNS resolution check not implemented yet.** A host on the allow-list could in principle resolve to a private IP if an attacker controls DNS for that host. Mitigated in practice because the allow-list contains only trusted LLM-provider domains, but a full fix (resolve host → reject if any returned IP is private/loopback/link-local) is a follow-up if an operator widens the allow-list. The `UrlRejection` enum has room for a `ResolvesToPrivateIp` variant.
- **No per-operator extension of the allow-list** (e.g. `CADE_PROXY_ALLOWED_HOSTS` env var). Declined in design question; can be added without breaking changes.

---

## 2026-04-17T04:29Z — Task 4 / P1-4: Filesystem sandbox default-on

**Summary:** Flipped the filesystem-tool sandbox from opt-in (required `CADE_FS_ROOT`) to default-on (active without any configuration). When neither `CADE_FS_ROOT` nor `CADE_FS_NO_SANDBOX` is set, the sandbox root defaults to `std::env::current_dir()` captured once at first use. The only way to disable the sandbox is `CADE_FS_NO_SANDBOX=1` (exact match required so operators cannot accidentally disable it with truthy-looking values like `0`, `true`, or empty strings).

**Files modified:**
- `crates/cade-agent/src/tools/fs.rs` — replaced the old `fs_root()` with a pure policy function `resolve_fs_root(env_root, no_sandbox, cwd) -> Option<PathBuf>` plus a caching wrapper `fs_root()` backed by `std::sync::OnceLock`. Updated module-level comment from "SEC-A opt-in" to "P1-4 default-on". Added 6 unit tests covering the new policy.

**Behavior matrix:**
| CADE_FS_ROOT | CADE_FS_NO_SANDBOX | Result |
|---|---|---|
| (unset) | (unset) | sandbox ACTIVE at cwd |
| `/path` | (unset) | sandbox ACTIVE at /path (canonicalized) |
| `   ` (ws-only) | (unset) | sandbox ACTIVE at cwd (whitespace-only treated as unset) |
| (any) | `1` | sandbox DISABLED |
| (any) | `0`, `true`, `""`, `yes` | sandbox ACTIVE (only exact `"1"` opts out) |

**Reason:** CRITICAL-severity finding in the security review — the filesystem sandbox was opt-in, meaning a user who ran `cade` without setting `CADE_FS_ROOT` had no path confinement at all. A prompt-injection attack that reached a `read_file`, `write_file`, or `apply_patch` tool call could read `/etc/passwd`, write `/etc/cron.d/*`, or similar. Per the user-approved remediation contract, P1-4 ships as default-on with `CADE_FS_NO_SANDBOX=1` as the documented escape hatch.

**Previous behavior:** `fs_root()` returned `Some(root)` only when `CADE_FS_ROOT` was set. When unset, all 4 file tools (read_file, write_file, list_dir, apply_patch) skipped the `ensure_within_root` check entirely and could operate on any path the process could reach.

**New behavior:** `fs_root()` returns `Some(root)` by default (resolved to cwd or the explicit env value), activating `ensure_within_root` on every file-tool call. Returns `None` only when `CADE_FS_NO_SANDBOX=1` is set. The resolved root is cached in `OnceLock` so subsequent calls are cheap and behavior is deterministic across the process lifetime (e.g., a later `cd` in a shelled-out bash tool does not move the sandbox).

**Design notes:**
- **Policy/accessor split:** pure `resolve_fs_root()` takes env + cwd as explicit arguments, making it deterministic and unit-testable without process env mutation (which is racy under parallel tests). The `fs_root()` accessor is a thin caching wrapper that reads env once at first call.
- **Strict escape-hatch matching:** we check `matches!(no_sandbox.as_deref(), Some("1"))` rather than any truthy parse, so unusual values do NOT disable the sandbox. Defense in depth against misconfiguration.
- **Call sites unchanged:** the 4 tools already use `if let Some(root) = &fs_root() { ensure_within_root(...) }`, so the refactor is behavior-compatible at the call site. Only the semantics of what "None" means changed (was: "always, because opt-in"; now: "only when explicitly disabled").

**Tests:**
- `cargo test -p cade-agent --lib tools::fs` → 15 green (9 pre-existing + 6 new P1-4 tests).
- `cargo test -p cade-agent` → 84/84 green, no regressions.
- `cargo clippy -p cade-agent --lib --tests` → no warnings from fs.rs.
- `cargo build --workspace` → clean.

**New dependencies:** none. Uses `std::sync::OnceLock` (stdlib).

**Rollback:** `restore_checkpoint cp-db451c65-b661-4e88-87f9-edbf0247e154` (label `before-p1-4-fs-sandbox`). For task-level revert: `git revert` the P1-4 commit — restores opt-in sandbox (re-opens the CRITICAL gap).

**Operator migration:**
- **Default install:** no change needed — sandbox activates at cwd.
- **Was relying on skip-when-unset:** set `CADE_FS_NO_SANDBOX=1` to restore previous behavior (NOT recommended; advertises the risk).
- **Wanted a specific root:** no change — `CADE_FS_ROOT=/path` still works as before.


---

## 2026-04-17T04:37Z — Task 5 / P2-1: Anchor DB key file at home/.cade/db.key

**Summary:** The DB encryption key file is now read exclusively from the user home directory under .cade/db.key, never from the process cwd. The cwd-based path was a classic trust-the-working-directory vulnerability: cd-ing into a hostile repo (supply-chain, shared devcontainer, malicious checkout) handed the attacker the DB encryption key for every subsequent write.

**Files modified:**
- crates/cade-store/Cargo.toml - added dirs (explicitly approved in the remediation contract).
- crates/cade-store/src/crypto.rs - added pure policy function resolve_db_key_path(home) -> Option<PathBuf>. Rewrote get_root_secret() to use it, hard-error when home is unresolvable and no env var is set, auto-create .cade/ with 0o700 perms on Unix. Updated test helper setup_test_key() to use std::env::set_var (race-free via Once::call_once, P2-1-safe). Added 3 unit tests.
- crates/cade-store/src/sqlite/providers.rs - updated stale comment.
- crates/cade-core/src/permissions/checks.rs - added three new path_is_protected patterns for the new canonical anchor.
- crates/cade-core/src/permissions/tests.rs - 3 new assertions covering the new protected patterns.

**Threat blocked:**
Attacker plants key file in hostile repo; user cds in and runs cade. BEFORE: attacker key is used for all DB writes; attacker can decrypt stolen DB files offline. AFTER: cwd file is ignored entirely; only home-dir anchor or explicit env var is consulted.

**Previous behavior (pre-P2-1):**
1. CADE_DB_KEY env -> use it
2. CADE_MACHINE_SECRET env -> use it
3. cwd key file -> read and use it
4. cade.db exists in cwd -> use machine_uid (legacy)
5. otherwise -> generate random key, write to cwd

**New behavior (P2-1):**
1. CADE_DB_KEY env -> use it (unchanged)
2. CADE_MACHINE_SECRET env -> use it (unchanged)
3. home/.cade/db.key -> read and use it (MOVED)
4. cade.db exists in cwd -> use machine_uid (legacy fallback preserved)
5. otherwise -> generate random key, write to home/.cade/db.key with 0o600 perms inside a 0o700 directory (MOVED)
6. if home unresolvable AND no env var set AND no legacy cade.db -> hard error with clear message

**Tests:**
- cargo test -p cade-store --lib crypto -> 11 green (8 pre-existing + 3 new P2-1 tests).
- cargo test -p cade-core --lib permissions -> 74 green (71 pre-existing + 3 new).
- cargo test --workspace -> 640 green, 0 failed.
- cargo clippy -p cade-store --lib --tests -> no new warnings.
- cargo clippy -p cade-core --lib --tests -> no warnings from touched files.

**New dependencies:** dirs added to cade-store (approved in the remediation contract; already in workspace deps).

**Rollback:** restore_checkpoint cp-368623d5-42fe-4cc5-8cf3-17fb39495f83 (label before-p2-1-db-key). For task-level revert: git revert the P2-1 commit — restores cwd-file reading (re-opens the HIGH-severity gap).

**Operator migration (pre-P2-1 -> P2-1):**
- If CADE_DB_KEY is set in env: no action.
- If home anchor does not exist and old cwd key exists: move it once (mkdir -p ~/.cade && mv <old-path> ~/.cade/db.key && chmod 600 ~/.cade/db.key). Without this, existing encrypted DB values cannot be decrypted until CADE_DB_KEY is set to the original key string.
- Existing cade.db encrypted via legacy machine_uid fallback: no action. The fallback branch still fires when cade.db exists in cwd.
- Fresh install: no action. A new random key auto-generates at home anchor on first use.

**Known limitations (deferred):**
- No auto-migration. Intentional per approved design: reading from cwd is the vulnerability; preserving that code path leaves the surface open.
- The weak 100k-iteration PBKDF2 derivation is unchanged. That is P2-2.


---

## 2026-04-17T04:45Z — Task 6 / P2-2: Replace 100k PBKDF2 with Argon2id

**Summary:** Swapped the KDF used to derive the AES-256-GCM key from PBKDF2-HMAC-SHA256 (100k iterations) to Argon2id with OWASP 2023 recommended defaults (m_cost=19456 KiB, t_cost=2, p_cost=1). New ciphertexts carry a 1-byte version prefix (0x02) so the decrypt path can dispatch correctly; existing pre-P2-2 ciphertexts (unprefixed) still decrypt via the retained PBKDF2 branches.

**Files modified:**
- `Cargo.toml` — added `argon2 = "0.5"` to `[workspace.dependencies]`.
- `crates/cade-store/Cargo.toml` — added `argon2 = { workspace = true }`.
- `crates/cade-store/src/crypto.rs` — replaced the single `derive_key()` with two specialized functions: `derive_key_argon2id()` (new default, used by `encrypt()`) and `derive_key_pbkdf2()` (compat-only, used by legacy decrypt branches). Added `KDF_V2_ARGON2ID = 0x02` version byte, `ARGON2_M_COST = 19_456`, `ARGON2_T_COST = 2`, `ARGON2_P_COST = 1` constants. Rewrote `encrypt()` to prepend the version byte. Rewrote `decrypt()` to dispatch on leading byte: 0x02 -> Argon2id, otherwise fall through to the existing PBKDF2 branches (unprefixed salted >=29 bytes, or static-salt <29 bytes). Added a doc comment to the public `decrypt()` documenting the dispatch table. Cleaned up one pre-existing dangling doc comment that was also getting flagged by clippy after my earlier edit.

**Threat reduced:** the previous 100k-iteration PBKDF2 provides ~10 ms of CPU work per guess on modern hardware. An offline attacker who steals the encrypted DB AND learns the machine secret format (32-byte base64) could brute-force a weak secret in GPU time. Argon2id with the OWASP defaults takes ~50 ms per derivation and is deliberately memory-hard (19 MiB per guess), making GPU/ASIC attacks far less efficient — roughly a 5000x slowdown for an equivalent dollar cost on attacker hardware, and far worse if the attacker has to parallelize across many guesses because of the memory pressure.

**Previous behavior (pre-P2-2):**
- `encrypt()` output layout: `[ salt(16) | nonce(12) | ct+tag ]`, key derived via PBKDF2-HMAC-SHA256 100k iterations.
- `decrypt()` dispatched purely on byte length: >=29 -> salted PBKDF2, else static-salt PBKDF2.

**New behavior (P2-2):**
- `encrypt()` output layout: `[ 0x02 | salt(16) | nonce(12) | ct+tag ]`, key derived via Argon2id.
- `decrypt()` dispatch:
  1. len >= 30 AND data[0] == 0x02 -> Argon2id (current).
  2. len >= 29 -> PBKDF2 with extracted salt (pre-P2-2 legacy, warns).
  3. len >= 12 -> PBKDF2 with hardcoded salt (oldest legacy, warns).
  4. else -> error.

**Tests:**
- 6 new unit tests in `crypto.rs`:
  * `p2_2_argon2_params_match_owasp_profile` - param constants locked to OWASP values.
  * `p2_2_new_ciphertext_starts_with_version_byte` - verifies 0x02 prefix in fresh encrypts.
  * `p2_2_argon2id_roundtrip` - encrypt/decrypt happy path.
  * `p2_2_legacy_pbkdf2_salted_blob_still_decrypts` - hand-crafted pre-P2-2 blob still decrypts.
  * `p2_2_legacy_static_salt_blob_still_decrypts` - oldest format still decrypts for len<29.
  * `p2_2_corrupted_version_byte_fails_cleanly` - XORed version byte returns error, no panic.
- `cargo test -p cade-store --lib crypto` -> 17/17 green (11 pre-existing + 6 new).
- `cargo test --workspace` -> 646 green, 0 failed (up from 640, +6).
- `cargo clippy -p cade-store --lib --tests` -> no new warnings from crypto.rs.

**New dependencies:** `argon2 = "0.5"` (0.5.3) added to workspace + cade-store. Explicitly pre-approved in the remediation contract.

**Rollback:** `restore_checkpoint cp-160fd827-925d-4fe1-b4d9-209b231d83e9` (label `before-p2-2-argon2id`). For task-level revert: git revert the P2-2 commit. Values encrypted after P2-2 land will be unreadable after a revert because the PBKDF2-only dispatch does not recognize the 0x02 prefix; operators would need to manually re-save any providers added between P2-2 and revert.

**Design notes:**
- KDF-version byte chosen over an outer container (e.g. JSON envelope) because (a) it preserves the existing base64-string format callers expect, (b) it adds only 1 byte overhead per value, (c) dispatch is O(1) and unambiguous (0x02 in the first byte of an unprefixed salted blob would require a specific base64 bit pattern we can rule out by checking len AND value).
- `Option<u32>` output len on `Params::new(...)` is set to `Some(32)` to match the `[u8; 32]` AES-256 key size; the default (None) would imply Argon2's internal default (32 bytes) but being explicit avoids silent breakage if argon2 crate defaults change.
- PBKDF2 dep (`pbkdf2 = "0.12"`) is kept as compat-only. It can be removed in a future release once operators confirm all legacy values have been re-saved.

**Known limitations (deferred):**
- No automatic "re-encrypt on read" for legacy blobs. Operators currently see a tracing::warn! log and can re-save values through the UI to upgrade them. A future task could add an opportunistic upgrade inside the decrypt-then-use code path if desired.
- OWASP params are fixed constants. A future task could expose them via env vars (e.g. CADE_ARGON2_M_COST) for constrained environments.

---

## /theme UI/UX Modernisation — 2026-04-16

**Timestamp:** 2026-04-16T06:00:00Z

### Summary
Modernised the `/theme` command, theme picker, and TUI visual layer across 7 implementation steps.

### Files Modified
- `crates/cade-tui/src/colors.rs` — `BorderStyle` enum; 4 new token fields (`border_style`, `bg_card`, `bg_input`, `accent_dim`); refined `dark()` + `light()` palettes; new built-ins `catppuccin_mocha()`, `catppuccin_latte()`, `tokyo_night()`
- `crates/cade-tui/src/lib.rs` — re-exports `BorderStyle`
- `crates/cade-core/src/resources/themes.rs` — `Theme` struct gained `description`, `author`, `variant` fields (all `Option<String>`, `#[serde(default)]`, backward-compatible)
- `crates/cade-cli/src/cli/repl/commands_theme.rs` — built-in theme list extended with metadata + 3 new names; direct-name dispatch for `catppuccin-mocha`, `catppuccin-latte`, `tokyo-night`
- `crates/cade-tui/src/app/layout/pickers.rs` — full theme picker rewrite: colour swatches, built-in/custom grouping, live-preview badge, themed border style, `bg_surface0` background
- `crates/cade-tui/src/app/layout/sidebar.rs` — sidebar outer block now uses `bg_surface0` for a distinct panel background
- `crates/cade-tui/src/app/render.rs` — input area prefix + textarea use `bg_input`; stale `BorderType` import removed
- `crates/cade-tui/src/app/layout/command_palette.rs`, `toast.rs`, `summary.rs`, `overlay.rs`, `mcp_picker.rs`, `skills.rs` — all `BorderType::Rounded` replaced with `colors.border_style.to_ratatui()`

### Previous Behaviour
- 2 built-in themes (dark/light) with flat, low-contrast palettes
- No `BorderStyle`, `bg_card`, `bg_input`, `accent_dim` tokens
- Theme picker: plain table, no swatches, no grouping, hardcoded `BorderType::Rounded`
- Sidebar had no background; input area had no distinct background
- `Theme` struct had no metadata fields

### New Behaviour
- 5 built-in themes: `dark`, `light`, `catppuccin-mocha`, `catppuccin-latte`, `tokyo-night`
- Richer palettes with noticeable layer depth (8–10 pt RGB step between bg levels)
- `BorderStyle` enum controls border character style across all overlays
- Theme picker shows colour swatches (primary/success/error/warning/bg_surface2) per row, groups built-in vs custom, shows live-preview badge
- Sidebar rendered over `bg_surface0`; input area rendered over `bg_input`
- `Theme` supports optional `description`, `author`, `variant` metadata (fully backward-compatible JSON)

### Rollback Steps
1. `git revert HEAD` (single commit covers all changes)
2. Or revert individual files listed above — each change is isolated to its file
