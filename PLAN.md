## 2026-05-03T21:30:00Z — refactor(its): remove vaporware reranker, exempt CADE tools from compression

**Task:** Clean up the Intelligent Tool Selection system — remove non-existent reranker documentation, exempt all CADE-owned tools from description compression, keep compression only for MCP (third-party) tools.

**Files modified:**
- `crates/cade-server/src/server/api/messages/context.rs` (compression now targets only MCP tools identified by `__` in name; CADE-owned tools always keep full descriptions)
- `crates/cade-server/src/server/api/messages/mod.rs` (updated ALWAYS_INCLUDE_TOOL_NAMES doc comment to reflect new role as desktop prune safety net)
- `crates/cade-server/src/server/api/messages/tests.rs` (updated always-include test to cover load_skill/unload_skill; added new test verifying CADE tools never match MCP `__` pattern)
- `docs/intelligent-tool-selection.md` (replaced vaporware reranker docs with accurate 2-layer ITS documentation)

**Previous behavior:**
- ITS compressed descriptions of ALL non-recent, non-always-include tools (including CADE meta tools like unload_skill) to 80 chars on long sessions.
- docs/intelligent-tool-selection.md described a cross-encoder reranker (ONNX, Cohere, Voyage, Jina) that did not exist in the codebase.
- ALWAYS_INCLUDE_TOOL_NAMES only covered 7 memory tools — other meta tools could be compressed.

**New behavior:**
- CADE-owned tools (no `__` in name) are NEVER compressed — full descriptions always sent to LLM.
- Only MCP tools (name contains `__`) are compressed when unused in the recent 20-message window.
- desktop_* pruning unchanged (working correctly).
- docs/intelligent-tool-selection.md accurately describes the 2-layer system that actually runs.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git revert HEAD
```

---

## 2026-05-03T20:55:00Z — fix(skills): ensure unload_skill is always visible to LLM

**Task:** Fix unload_skill tool not being callable by the LLM due to ITS description compression; add missing sequential-tool classification in headless mode.

**Files modified:**
- `crates/cade-server/src/server/api/messages/mod.rs` (added load_skill and unload_skill to ALWAYS_INCLUDE_TOOL_NAMES)
- `crates/cade-cli/src/cli/headless.rs` (added unload_skill to is_sequential_tool match)

**Previous behavior:**
- unload_skill description was compressed to 80 chars on long sessions, making it invisible to the LLM.
- unload_skill could run in parallel with other skill mutations in headless mode.

**New behavior:**
- unload_skill always keeps its full description (never compressed by ITS).
- unload_skill classified as sequential in headless mode (no parallel execution race).

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git revert 0ce5a7fc
```

---

## 2026-05-03T20:49:00Z — refactor(scroll): unified viewport scroll for idle + processing states

**Task:** Unify scroll handling so all scroll keys (Shift+K/J, PageUp/PageDown, mouse wheel) work identically whether idle or during agent processing, with smooth-scroll animation in both states.

**Files modified:**
- `crates/cade-tui/src/app/state.rs` (shared `handle_scroll_key()`, `handle_scroll_mouse()`; `push()` respects `follow` flag)
- `crates/cade-tui/src/app/input.rs` (idle loop rewired to shared handlers; removed duplicated scroll logic)
- `crates/cade-cli/src/cli/repl/turn_loop/agent.rs` (tick task rewired to shared handlers; added PageUp/PageDown; smooth-scroll via `scroll_target`)

**Previous behavior:**
- Tick task used direct `scroll` mutation (instant, jarring jumps).
- Only Shift+K (10 lines) and Shift+J (snap) worked during processing; no PageUp/PageDown.
- Mouse wheel scrolled ±1 line during processing vs ±3 (smooth) when idle.
- `push()` always force-reset scroll to bottom, even when user had scrolled up.

**New behavior:**
- All scroll keys route through `handle_scroll_key()` / `handle_scroll_mouse()` (single source of truth).
- PageUp/PageDown work during agent processing.
- Mouse and keyboard scroll use `scroll_target` for smooth animation in both states.
- `push()` respects `follow` flag: scrolled-up users see "↓ N new" badge instead of viewport hijack.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git revert 7e5fa109
```

---

## 2026-05-03T20:36:00Z — refactor(theme): deduplicate brighten logic + color helper tests

**Task:** Address code-review warnings: deduplicate inline brighten closure and add unit test coverage for color helper functions.

**Files modified:**
- `crates/cade-core/src/resources/themes.rs` (replaced inline closure with `brighten_color()`; added 10 unit tests)

**Previous behavior:**
- Inline `brighten` closure in spinner gradient duplicated `brighten_color()` logic.
- No unit tests for `brighten_color()`, `dim_color()`, or `from_theme()` fallback chains.

**New behavior:**
- Spinner gradient uses `brighten_color()` helper (no duplication).
- 10 new tests: saturation clamping, underflow clamping, Reset passthrough, spinner derivation, thinkingXhigh→error fallback, bashMode→warning fallback, ctx_bar derivation.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git revert b3059bbc
```

---

## 2026-05-03T20:26:00Z — fix(theme): auto-derive extended tokens and remove hardcoded colors

**Task:** Fix 4 gaps in the theme system: auto-derive missing extended tokens, add fallback for thinkingXhigh/bashMode, expose extended tokens in JSON schema, and remove hardcoded Color::DarkGray.

**Files modified:**
- `crates/cade-core/src/resources/themes.rs` (11 new optional ThemeTokens fields; `from_theme()` auto-derives ctx_bar_*, spinner_*, accent_dim, border_style; fallback logic for thinkingXhigh/bashMode; `brighten_color()`/`dim_color()` helpers)
- `crates/cade-tui/src/app/render.rs` (replaced `Color::DarkGray` with `colors.text_dim` in subagent drop shadow)

**Previous behavior:**
- `from_theme()` left ctx_bar_* (8 tokens), spinner_* (4 tokens), accent_dim, and border_style at `dark()` defaults for all custom JSON themes.
- All 30 user JSON themes missing `thinkingXhigh` and `bashMode` tokens (resolved to `ColorDef::Reset`).
- Subagent drop shadow used hardcoded `Color::DarkGray`.
- No JSON schema for extended tokens (ctx_bar_*, spinner_*, border_style).

**New behavior:**
- Extended tokens auto-derived from core palette in `from_theme()`.
- `thinkingXhigh` falls back to `error`; `bashMode` falls back to `warning`.
- Drop shadow uses themed `text_dim` color.
- 11 optional extended tokens available in JSON schema (`borderStyle`, `spinnerAccent`, `ctxBar*`).

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git revert d1dceca1
```

---

## 2026-05-03T20:57:00Z — docs: update themes.md and keybindings.md for recent changes

**Task:** Update documentation to reflect theme system optimizations and scroll refactoring.

**Files modified:**
- `docs/themes.md` (added full token reference table with all 60+ tokens; documented extended tokens, auto-derivation, and fallback behavior)
- `docs/keybindings.md` (fixed incorrect Shift+J description; added PageUp/PageDown and mouse wheel; documented scroll-during-processing and follow behavior)
- `PLAN.md` (appended entries for 3 recent commits + this docs update)

**Previous behavior:**
- themes.md JSON example showed only ~20 of 60+ tokens; no mention of extended tokens or auto-derivation.
- keybindings.md incorrectly described Shift+J as "scroll down 10 rows" (it snaps to bottom); no PageUp/PageDown or mouse wheel documented; no mention of scroll during processing.

**New behavior:**
- themes.md has complete token reference table grouped by category with auto-derivation notes.
- keybindings.md accurately describes all viewport scroll keys, mouse wheel, and scroll-during-processing behavior.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git revert HEAD
```

---

## 2026-04-27T21:40:00Z — code-review: cleanup batch (M9r, M6, M5, M1, N1-N3)

**Task:** Apply 6 lower-severity findings from validated code review (M9r, M6, M5, M1, N1-N3).

**Files modified:**
- `crates/cade-server/src/server/api/run.rs` (M9r, M1)
- `crates/cade-server/src/server/consolidation.rs` (M5)
- `crates/cade-agent/src/tools/manager.rs` (M6, N3)
- `crates/cade-agent/src/tools/runtime/mod.rs` (N1, N2)

**Previous behavior:**
- Agentic loop exit tracked as "done" indiscriminately.
- Tool name matching failed on MCP-prefixed names.
- Dead no-op `should_skip_noisy_tool` logic existed.
- Skill load/unload forced string round-tripping.
- Stray comments existed in manager/runtime.

**New behavior:**
- Exit status accurately tracked by `RunExitStatus`.
- Tool name resolution works for all MCP names.
- Cleanup of dead no-op logic.
- Direct JSON argument passing.
- Stripped stray comments.

**TDD record:**
- Tests: `cade-server` 258/258, `cade-agent` 113/113. 
- All passing.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```
