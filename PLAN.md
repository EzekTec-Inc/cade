## 2026-05-03T22:45:00Z — fix(tui): comprehensive theme polish — wire unused tokens, fix hardcoded styles

## 2026-05-04T03:40:00Z — feat(tui): theme reload, .tmTheme override, password backdrop, border token split

**Task:** 4 remaining TUI polish items: theme hot-reload, syntax theme file support, password popup backdrop, border_accent/border_muted split.

**Files modified:**
- `crates/cade-cli/src/cli/repl/commands_theme.rs` (add `/theme reload` handler: re-reads theme from disk)
- `crates/cade-core/src/resources/themes.rs` (add `syntax_theme_override`, `border_muted`, `border_accent` fields; auto-discover .tmTheme in from_theme; set defaults for all 5 themes; wire from_theme resolution)
- `crates/cade-tui/src/colors.rs` (generate_syntect_theme loads .tmTheme override; add border_muted/border_accent trait methods)
- `crates/cade-tui/src/app/password.rs` (add backdrop dimming, border_type, surface1 bg, title_style, themed prompt text)
- `crates/cade-tui/src/app/timeline/render_item.rs` (inline │ borders → border_muted)
- `crates/cade-tui/src/app/layout/breadcrumb.rs` (separator → border_muted)
- `crates/cade-tui/src/app/layout/command_palette.rs` (overlay border → border_accent, separator → border_muted)
- `crates/cade-tui/src/app/layout/pickers.rs` (overlay borders → border_accent, separator → border_muted)
- `crates/cade-tui/src/app/layout/question.rs` (separator → border_muted)
- `crates/cade-tui/src/question.rs` (separator → border_muted)
- `crates/cade-tui/src/overlay.rs` (overlay border → border_accent)
- `crates/cade-tui/src/mcp_picker.rs` (overlay borders → border_accent)
- `crates/cade-tui/src/skills.rs` (overlay borders → border_accent)

**Previous behavior:**
- No `/theme reload` in CLI (only in server)
- No .tmTheme syntax highlight override
- Password popup had no backdrop dimming, no themed border_type, no background
- Single `border_base` token for all borders

**New behavior:**
- `/theme reload` re-reads the current theme's source file from disk and applies it live
- `.tmTheme` files auto-discovered next to theme JSON (mytheme.tmTheme → mytheme.json) override syntax highlighting
- Password popup has dimmed backdrop, themed border_type, surface1 bg, styled title
- `border_muted` for inline/separator borders (dimmer), `border_accent` for overlay borders (brighter)
- `border_base` retained for structural panel borders (sidebar, plan panel)

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git revert HEAD
```

---

**Task:** Fix 4 critical hardcoded styles, wire 6 unused theme tokens, add background fills, and add overlay backdrop dimming.

**Files modified:**
- `crates/cade-tui/src/app/render.rs` (fix hardcoded border_type, unstyled "Terminal too small", hardcoded cursor style, add plan title style)
- `crates/cade-tui/src/session_tree.rs` (fix unstyled icon)
- `crates/cade-tui/src/colors.rs` (add ThemeColorsExt methods: bg_card_style, selected_bg_style, tool_success/error/pending_bg_style)
- `crates/cade-tui/src/app/timeline/mod.rs` (wire bg_card_style into user/assistant cards)
- `crates/cade-tui/src/app/timeline/render_item.rs` (wire tool_success_bg/tool_error_bg/tool_pending_bg into status badges)
- `crates/cade-tui/src/app/layout/pickers.rs` (wire selected_bg_style into selected rows + add backdrop)
- `crates/cade-tui/src/app/layout/command_palette.rs` (wire selected_bg into selected items + add backdrop)
- `crates/cade-tui/src/app/layout/question.rs` (add surface1 background fill)
- `crates/cade-tui/src/app/layout/summary.rs` (add backdrop)
- `crates/cade-tui/src/app/layout/helpers.rs` (add render_backdrop helper)
- `crates/cade-tui/src/mcp_picker.rs` (wire selected_bg_style)
- `crates/cade-core/src/resources/themes.rs` (add selected_bg, tool_pending_bg, tool_success_bg, tool_error_bg fields; set defaults for all themes; wire from_theme resolution)

**Previous behavior:**
- Subagent overlay used hardcoded `BorderType::Rounded`
- "Terminal too small" had no styling
- Cursor was hardcoded REVERSED
- Session tree icons had no fg color
- User/assistant cards had no background fill
- Tool result badges had no background
- Picker selected rows used hardcoded bg_surface1
- Overlays had no backdrop dimming
- 25 theme tokens defined but never used

**New behavior:**
- All borders use `colors.border_style.to_ratatui()`
- "Terminal too small" styled with error color
- Cursor uses `primary` bg + `bg_base` fg
- Session tree icons use `text_muted`
- User/assistant cards have subtle `bg_card` background
- Tool result OK/ERR/LIVE badges have status-colored backgrounds
- Picker selections use `selected_bg` token
- Command palette, theme picker, summary overlays have dimmed backdrop
- Question panel has `surface1` background fill
- Plan title styled with `primary_bold`

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git revert HEAD
```

---

## 2026-05-03T22:15:00Z — refactor(its): replace hardcoded tool name lists with tag-based discovery

**Task:** Remove all hardcoded tool name lists from ITS. Use DB registration tags (`"cade"`, `"meta"`, `"mcp"`) to classify tools dynamically. Use meta-tool registry for sequential-tool classification.

**Files modified:**
- `crates/cade-server/src/server/api/messages/context.rs` (carry tags alongside schemas; use `"mcp"` tag for prune/compress decisions instead of name heuristics)
- `crates/cade-server/src/server/api/messages/mod.rs` (removed `ALWAYS_INCLUDE_TOOL_NAMES` constant — no longer needed)
- `crates/cade-server/src/server/api/messages/tests.rs` (replaced hardcoded-name tests with tag-contract tests)
- `crates/cade-cli/src/cli/headless.rs` (replaced hardcoded `is_sequential_tool` match with `LazyLock` set built from `all_meta_schemas()` registry)
- `docs/intelligent-tool-selection.md` (updated to reflect tag-based discovery)

**Previous behavior:**
- `ALWAYS_INCLUDE_TOOL_NAMES` hardcoded 9 tool names for prune bypass.
- `name.contains("__")` heuristic used to identify MCP tools for compression.
- `is_sequential_tool` hardcoded 6 tool names for parallel-execution safety.
- Adding a new meta tool required manually updating 3 separate lists.

**New behavior:**
- ITS reads `tags` from the DB `ToolRow` and checks for `"mcp"` tag to identify third-party tools.
- CADE-owned tools (no `"mcp"` tag) are never pruned or compressed.
- `is_sequential_tool` discovers meta tools from `all_meta_schemas()` at first call via `LazyLock`.
- Adding a new meta tool only requires adding its schema to `meta.rs` — all downstream logic auto-discovers it.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git revert HEAD
```

---

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
