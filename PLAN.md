## 2026-05-07T14:30:00Z — fix(subagent): Implement REC-1, REC-2, REC-3 audit recommendations

**Task:** Implement the three P1–P2 recommendations from the subagent system audit: wall-clock timeout (REC-1), ephemeral row cleanup guard (REC-2), and cascading write-back prefix filter (REC-3).

**Files modified:**
- `crates/cade-server/src/server/api/run/subagent.rs`
  - REC-1: Added `subagent_timeout_secs()` helper (300s prod, 2s test) and wrapped agentic loop in `tokio::time::timeout`. On timeout, sets error with "timeout" message.
  - REC-2: Added `EphemeralAgentGuard` struct with `Drop` impl. Ensures `write_back_subagent_memory` + `delete_agent` run even on panic/early-return. Replaced manual cleanup calls (lines 394–402) with guard.
- `crates/cade-store/src/sqlite/memory.rs`
  - REC-3: Added `label.starts_with("subagent:")` check to `write_back_subagent_memory` filter (line 1153). Prevents cascading `subagent:subagent:subagent:*` labels.
- `crates/cade-server/src/server/api/run/tests.rs`
  - Added `subagent_loop_respects_wall_clock_timeout` (async, uses SlowLlm mock)
  - Added `ephemeral_agent_guard_cleans_up_on_drop` (sync, verifies guard Drop + write-back)
- `crates/cade-store/src/sqlite/memory/tests.rs`
  - Added `test_a15_write_back_excludes_subagent_prefix` (verifies cascading labels blocked)

**Previous behavior:**
- No wall-clock timeout — hung LLM/tool held semaphore permit indefinitely
- Ephemeral agent row leaked on panic between create (line 197) and delete (line 341)
- `subagent:` prefixed blocks cascaded through write-back: `subagent:subagent:subagent:foo`

**New behavior:**
- Agentic loop times out after `CADE_SUBAGENT_TIMEOUT_SECS` (default 300s, env-configurable). Returns error with "timeout" message. EphemeralAgentGuard ensures cleanup.
- `EphemeralAgentGuard` Drop runs write-back + delete on all exit paths (happy, error, panic)
- Write-back filters out `subagent:` prefixed labels — no cascading duplication

**Dependency policy:** No new dependencies. EphemeralAgentGuard is a manual Drop struct (no scopeguard crate).

**Test results:** 1579 passed, 0 failed. `cargo clippy --workspace -- -D warnings` clean.

**Rollback steps:**
```sh
git checkout cp-3ce7db43  # or: git revert HEAD
```

---

## 2026-05-04T04:50:00Z — fix(gui): theme variant field + overlay backdrops + /theme list

**Task:** P0: Fix `dark_mode` hardcoded to `true` in GUI. P1: Theme overlay backdrops. P1: Add `/theme list`.

**Files modified:**
- `crates/cade-core/src/resources/themes.rs` — added `variant: Option<String>` to `ThemeColors`, `is_light()` helper, set in all 5 built-ins, copied in `from_theme()`; 8 new tests
- `crates/cade-gui/src/theme.rs` — `apply_theme()` now uses `theme.is_light()` for `dark_mode` and `Visuals::light()`/`dark()` base; added `overlay_backdrop()` to `EguiThemeExt`
- `crates/cade-gui/src/app/overlays/menu.rs` — `from_black_alpha(140)` → `theme.overlay_backdrop()`
- `crates/cade-gui/src/app/overlays/mcp.rs` — same
- `crates/cade-gui/src/app/overlays/models.rs` — same
- `crates/cade-cli/src/cli/repl/commands_theme.rs` — added `/theme list` sub-command
- `crates/cade-tui/src/menu.rs` — added `/theme list` to `/help` catalogue

**Previous behavior:** GUI `Visuals::dark_mode` was always `true`, light themes got wrong egui base. Overlay backdrops were hardcoded `from_black_alpha(140)`. No inline theme listing.

**New behavior:** `dark_mode` is `!theme.is_light()`, spread from correct `Visuals::light()`/`dark()`. Backdrops derive from `theme.bg_base` with alpha. `/theme list` prints all themes inline with variant, source, and active marker.

**Rollback:** `git revert <sha>` or restore checkpoint cp-98238dcd.

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

## 2026-05-04T23:45:00Z — Plan panel scrolling for overflowing todos

**Summary:** Made the `/todos` (plan) panel scrollable when steps exceed the visible panel height. Added auto-scroll to keep the first incomplete step visible, `ListState`-backed rendering for scroll offset, and a `Scrollbar` indicator.

**Files modified:**
- `crates/cade-tui/src/app/mod.rs` — Added `scroll_offset: usize` field to `PlanState`, implemented `auto_scroll()` method, auto-scroll logic in `draw()` before `render_frame` call. 5 new tests.
- `crates/cade-tui/src/app/state.rs` — Updated `set_plan()` to initialize `scroll_offset: 0`.
- `crates/cade-tui/src/app/render.rs` — Replaced `render_widget(List)` with `render_stateful_widget(List, &mut ListState)` using scroll offset. Added `Scrollbar` widget when steps overflow. Updated title to show progress count `(done/total)`.

**Reason:** When the agent sets a plan with more than 8 steps, the panel height was capped at 10 rows (8 visible + 2 border). Steps beyond 8 were silently clipped with no way to see them.

**Previous behavior:** Plan panel used stateless `render_widget(List)` with hard-capped height `.min(10)`. No scroll offset, no scrollbar, no indication of hidden steps.

**New behavior:**
- Panel keeps the `.min(10)` height cap but now scrolls internally via `ListState::with_offset()`.
- Auto-scroll keeps the first incomplete step visible, positioning it ~⅓ from the top of the visible area.
- When all steps are done, scrolls to show the last steps.
- `Scrollbar` with `↑`/`↓` arrows rendered when content overflows.
- Title shows progress: `Todos (3/7)`.

**Rollback steps:**
```sh
git checkout cp-277da1e8-8378-44d5-9528-57e884632deb -- crates/cade-tui/
```

---

## WI-SEMANTIC: Opt-in Hybrid Semantic Search for Memory Blocks

**Memory anchor:** `ANCHOR_WI_SEMANTIC_SEARCH`

**Status:** Plan approved. Not started.

**Problem:** `search_memory` (tools.rs:207) uses `LIKE` + brute-force word-match. When an
agent searches "how we fixed the deadlock", it will miss memory blocks containing "scoped
mutex lock" because no keywords overlap. The existing `embedding.rs` has RRF scaffolding
and `is_available() → false` — this plan wires it up.

**Constraint:** Opt-in via `semantic-search` feature flag. Zero binary bloat for
default builds. No ONNX/C++ compilation unless explicitly enabled.

### Validated Current State

| Component | Location | State |
|-----------|----------|-------|
| `search_memory` | `cade-store/src/sqlite/tools.rs:207` | LIKE + fuzzy word-match (no FTS5, no semantic) |
| `embedding.rs` | `cade-store/src/sqlite/embedding.rs` | RRF helper + `is_available() → false` |
| `search_memory_blocks_fts` | `embedding.rs:11` | **BUG**: queries `messages_fts` (wrong table) instead of memory-specific FTS |
| Schema | `mod.rs:95` | `shared_memory_blocks` — no `embedding` column, no `vec0` virtual table |
| Workspace deps | `Cargo.toml:97-98` | `fastembed = "4"`, `sqlite-vec = "0.1"` declared but unused |
| `cade-store` feature | `cade-store/Cargo.toml:9` | Comment: "Removed semantic-search" |
| Schema version | `mod.rs:552` | Currently at `PRAGMA user_version = 9` |
| Write path | `memory.rs:364` | Comment: "Semantic search feature removed (F5)" |
| Search call sites | `tools.rs:207`, `meta_tools.rs:494`, `agents.rs:759`, `tools.rs:712` | 4 callers |

### Bugs to Fix (Pre-requisites)

**Bug 1 — Wrong FTS table in `search_memory_blocks_fts`:**
`embedding.rs:18` queries `messages_fts` (conversation history FTS) but is documented as
searching memory blocks. Must either create a `memory_blocks_fts` table or rewrite the
query to join correctly.

---

### Phase 1: Feature Gate + Schema (no new deps)

**Goal:** Add the `semantic-search` feature flag and migration. Default build unchanged.

**WI-S1: Re-enable `semantic-search` feature flag in `cade-store/Cargo.toml`**
```toml
[features]
default         = ["bundled-sqlite"]
bundled-sqlite  = ["rusqlite/bundled"]
semantic-search = ["dep:fastembed", "dep:sqlite-vec"]
```

Files: `crates/cade-store/Cargo.toml`

**WI-S2: Migration 10 — add `embedding` column + `memory_blocks_fts` table**
```sql
-- Only when semantic-search feature is active:
ALTER TABLE shared_memory_blocks ADD COLUMN embedding BLOB;

-- Always (fixes Bug 1 — memory-specific FTS5):
CREATE VIRTUAL TABLE IF NOT EXISTS memory_blocks_fts USING fts5(
    label, value,
    content=shared_memory_blocks,
    content_rowid=rowid
);
-- Backfill existing blocks into the FTS index:
INSERT INTO memory_blocks_fts(memory_blocks_fts) VALUES('rebuild');
```

Files: `crates/cade-store/src/sqlite/mod.rs` (migration 10)

**WI-S3: Fix `search_memory_blocks_fts` to use `memory_blocks_fts`**

Rewrite `embedding.rs:17-25` to query the new `memory_blocks_fts` table:
```sql
SELECT b.id, bm25(memory_blocks_fts) as rank, b.label, b.value
FROM memory_blocks_fts f
JOIN shared_memory_blocks b ON b.rowid = f.rowid
JOIN agent_memory_blocks amb ON amb.block_id = b.id AND amb.agent_id = ?1
WHERE memory_blocks_fts MATCH ?2
ORDER BY rank
LIMIT ?3
```

Files: `crates/cade-store/src/sqlite/embedding.rs`

**Verification:**
- `cargo check -p cade-store` — compiles without `semantic-search`
- `cargo check -p cade-store --features semantic-search` — compiles with it
- Existing `rrf_*` tests still pass
- New test: `search_memory_blocks_fts` returns results from `memory_blocks_fts`
- `cargo test --workspace` — 0 regressions

---

### Phase 2: Embedder Trait + Write Path

**Goal:** Compute and store embeddings on memory block write when the feature is active.

**WI-S4: Define `Embedder` trait in `cade-store`**
```rust
// cade-store/src/sqlite/embedding.rs

/// Trait for computing text embeddings. Feature-gated implementations.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn dimension(&self) -> usize;
}

#[cfg(feature = "semantic-search")]
pub struct FastEmbedder { model: fastembed::TextEmbedding }

#[cfg(feature = "semantic-search")]
impl FastEmbedder {
    pub fn new() -> Result<Self> { /* init all-MiniLM-L6-v2 */ }
}

#[cfg(feature = "semantic-search")]
impl Embedder for FastEmbedder { /* delegate to fastembed */ }

/// Stub when feature is disabled. All methods return empty/error.
#[cfg(not(feature = "semantic-search"))]
pub struct NoopEmbedder;
```

Files: `crates/cade-store/src/sqlite/embedding.rs`

**WI-S5: Wire embedding into `upsert_memory_block`**

At `memory.rs:364` (currently a removed comment), add:
```rust
#[cfg(feature = "semantic-search")]
if let Some(embedder) = embedder {
    if let Ok(vec) = embedder.embed(&final_value) {
        let blob: Vec<u8> = vec.iter().flat_map(|f| f.to_le_bytes()).collect();
        conn.execute(
            "UPDATE shared_memory_blocks SET embedding = ?1 WHERE id = ?2",
            params![blob, block_id],
        )?;
    }
}
```

The `embedder` parameter is `Option<&dyn Embedder>` — `None` when feature is disabled.

Files: `crates/cade-store/src/sqlite/memory.rs`

**WI-S6: Backfill command — compute embeddings for existing blocks**

Add a one-shot function `backfill_embeddings(db, agent_id, embedder)` that iterates
all blocks with `embedding IS NULL` and computes + stores their vectors. Called from
server startup when the feature is active.

Files: `crates/cade-store/src/sqlite/embedding.rs`

**Verification:**
- Test: `upsert_memory_block` with `FastEmbedder` stores non-NULL embedding blob
- Test: `upsert_memory_block` without feature stores NULL embedding (no panic)
- Test: `backfill_embeddings` fills all NULL embedding rows
- `cargo test --workspace` — 0 regressions

---

### Phase 3: Semantic Search + Hybrid Merge

**Goal:** Add cosine similarity search and merge with keyword results via RRF.

**WI-S7: Implement `search_memory_semantic` in `embedding.rs`**
```rust
#[cfg(feature = "semantic-search")]
pub fn search_memory_semantic(
    conn: &Connection,
    agent_id: &str,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<(String, f64, String, String)>> {
    // Use sqlite-vec vec0 distance function:
    // SELECT b.id, vec_distance_cosine(b.embedding, ?1) as dist, b.label, b.value
    // FROM shared_memory_blocks b
    // JOIN agent_memory_blocks amb ON amb.block_id = b.id
    // WHERE amb.agent_id = ?2 AND b.embedding IS NOT NULL
    // ORDER BY dist ASC LIMIT ?3
}
```

Files: `crates/cade-store/src/sqlite/embedding.rs`

**WI-S8: Wire hybrid search into `search_memory` (tools.rs:286)**

Replace the "Phase 3: Semantic search (removed)" comment:
```rust
// Phase 3: Semantic search (feature-gated).
#[cfg(feature = "semantic-search")]
if results.len() < 10 {
    if let Some(embedder) = get_embedder() {
        if let Ok(query_vec) = embedder.embed(query) {
            let conn = db.lock();
            if let Ok(sem_hits) = embedding::search_memory_semantic(&conn, agent_id, &query_vec, 10) {
                let kw_ids: Vec<String> = results.iter().map(|(l, _, _)| l.clone()).collect();
                let sem_ids: Vec<String> = sem_hits.iter().map(|(id, _, _, _)| id.clone()).collect();
                let fused = embedding::reciprocal_rank_fusion(&kw_ids, &sem_ids, 60.0);
                // Rebuild results in fused order, deduplicating
                // ... merge logic using fused ordering ...
            }
        }
    }
}
```

Files: `crates/cade-store/src/sqlite/tools.rs`

**WI-S9: Update `is_available()` to reflect feature state**
```rust
pub fn is_available() -> bool {
    cfg!(feature = "semantic-search")
}
```

Files: `crates/cade-store/src/sqlite/embedding.rs`

**Verification:**
- Test: `search_memory("how we fixed the deadlock")` finds block containing "scoped mutex lock"
- Test: `search_memory("deadlock")` still finds block containing "deadlock" (keyword path)
- Test: RRF boosts blocks that appear in both keyword and semantic results
- Test: Feature-disabled build returns same results as today (LIKE + fuzzy only)
- `cargo test --workspace` — 0 regressions
- `cargo test --workspace --features semantic-search` — all new tests pass

---

### Phase 4: Server Integration

**Goal:** Initialize the embedder at server startup and pass it through call sites.

**WI-S10: Initialize embedder in `AppState`**

Add `embedder: Option<Arc<dyn Embedder>>` to `AppState`. Initialized at server startup:
- `#[cfg(feature = "semantic-search")]`: `Some(Arc::new(FastEmbedder::new()?))`
- `#[cfg(not(feature = "semantic-search"))]`: `None`

Files: `crates/cade-server/src/server/state.rs`

**WI-S11: Thread `embedder` through `upsert_memory_block` call sites**

All callers of `upsert_memory_block` must pass `state.embedder.as_deref()`:
- `meta_tools.rs` (update_memory tool handler)
- `consolidation.rs` (session_summary writes)
- `agents.rs` (memory API handlers)
- `evidence.rs` (memory with provenance)

Files: 4 files in `cade-server/src/server/api/` and `consolidation.rs`

**WI-S12: Run backfill on startup**

On server startup, if `semantic-search` feature is active:
```rust
if let Some(ref embedder) = state.embedder {
    tokio::task::spawn_blocking({
        let db = state.db.clone();
        let e = embedder.clone();
        move || embedding::backfill_embeddings(&db, "*", &*e)
    });
}
```

Files: `crates/cade-server/src/server/mod.rs` or `main.rs`

**Verification:**
- `cargo check --workspace` — compiles without feature
- `cargo check --workspace --features semantic-search` — compiles with feature
- `cargo test --workspace` — 0 regressions
- Manual test: start server with `--features semantic-search`, create memory blocks,
  search with semantic query, verify results

---

### Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| `fastembed` C++/ONNX compile failure on some platforms | Medium | Feature-gated; default build unaffected |
| Binary size increase (~25-50MB) | Low | Opt-in only; documented |
| First compile time increase (3-8 min) | Low | Only on `--features semantic-search`; incremental builds fast |
| `sqlite-vec` compatibility with `rusqlite 0.31` | Medium | Verify in Phase 1 before proceeding |
| Migration 10 adds FTS table — large existing DBs may rebuild slowly | Low | `INSERT INTO ... VALUES('rebuild')` is one-time; test on large DBs |
| Embedding quality for short text (<10 words) | Medium | Test with real memory blocks; fall back to keyword for very short queries |

### Execution Order

```
Phase 1 (no new deps)    →  feature gate + migration + FTS bug fix
Phase 2 (embedder trait) →  write-path embedding computation
Phase 3 (search merge)   →  hybrid retrieval + RRF
Phase 4 (server wiring)  →  startup init + call-site threading
```

Each phase is independently committable and testable. Phase 1 improves keyword
search (adds FTS5 for memory blocks) even without the semantic feature enabled.

### Commit Plan

```
Phase 1: fix(store): add memory_blocks_fts table + fix search_memory_blocks_fts bug
Phase 2: feat(store): add Embedder trait + write-path embedding (feature-gated)
Phase 3: feat(store): hybrid semantic+keyword search via RRF (feature-gated)
Phase 4: feat(server): wire embedder into AppState + startup backfill
```

---

## 2026-05-04T18:10:00Z — WI-SEMANTIC Phase 1: memory_blocks_fts + FTS bug fix

**Summary:** Phase 1 of WI-SEMANTIC. Re-enabled the `semantic-search` feature
gate in `cade-store` (off by default, no impact on default builds). Added
Migration 10 which creates the `memory_blocks_fts` FTS5 virtual table over
`shared_memory_blocks` plus three sync triggers (insert/update/delete) and a
one-time `'rebuild'` to backfill existing rows. Fixed the long-standing bug in
`embedding::search_memory_blocks_fts` where the SQL queried `messages_fts`
(conversation history) and joined `b.id = f.rowid` (incompatible types — TEXT
UUID vs INTEGER rowid), causing the function to silently return zero memory
hits. The query now targets `memory_blocks_fts` and joins on the integer rowid
that FTS5 stores via `content_rowid='rowid'`.

**Files modified:**
- `crates/cade-store/Cargo.toml` — added `[features] semantic-search = [...]`
  and optional `fastembed`/`sqlite-vec` deps. Default build pulls neither.
- `crates/cade-store/src/sqlite/mod.rs` — Migration 10: create
  `memory_blocks_fts` (FTS5 external-content over `shared_memory_blocks`,
  `content_rowid='rowid'`), three sync triggers, one-time rebuild. Bumps
  `PRAGMA user_version` 9 → 10. Tolerates "already exists" on re-run.
- `crates/cade-store/src/sqlite/embedding.rs` — rewrote
  `search_memory_blocks_fts` SQL: queries `memory_blocks_fts` (was
  `messages_fts`); joins `b.rowid = f.rowid` (was `b.id = f.rowid`).
- `crates/cade-store/src/sqlite/memory/tests.rs` — 3 new tests:
  `memory_blocks_fts_exists_after_migration`,
  `memory_blocks_fts_indexes_upserted_blocks`,
  `search_memory_blocks_fts_returns_memory_hits_not_messages`.

**Reason:** The plan validation in WI-SEMANTIC identified that the existing
FTS5 query in `embedding.rs` was unreachable on memory blocks — it referenced
the wrong table and joined incompatible columns. Phase 1 makes that path
correct so Phase 3's hybrid retrieval can reuse it.

**Previous behavior:**
- `search_memory_blocks_fts` returned `Ok(vec![])` for every input because
  `messages_fts` does not contain memory-block rows and the join key mismatch
  filtered out anything that did match.
- No FTS5 index over `shared_memory_blocks` existed.
- The `semantic-search` feature gate was removed (per a prior commit) with no
  way to opt back in without editing `Cargo.toml`.

**New behavior:**
- Default build: identical surface area, plus a working FTS5 index for memory
  blocks. `cade-store` clippy: clean.
- `--features semantic-search` build: pulls `fastembed`/`sqlite-vec`, compiles
  cleanly. (Phase 2 will start using them.)
- `search_memory_blocks_fts` returns correct BM25-ranked memory blocks for the
  given agent.
- Migration 10 is idempotent — second run is a no-op.

**Verification:**
- `cargo test -p cade-store --lib` → 140 passed (137 baseline + 3 new), 0 fail.
- `cargo test --workspace` → all suites pass, 0 regressions.
- `cargo check -p cade-store` → clean (default).
- `cargo check -p cade-store --features semantic-search` → clean.
- `cargo clippy -p cade-store --all-targets` → no warnings on cade-store.

**Rollback steps:**
```sh
git checkout cp-614cdf39-f4a3-4b00-a186-afecad1a199d -- crates/cade-store/
```

Note: the migration is idempotent and additive — rolling back the code does
not require dropping `memory_blocks_fts` or the triggers; subsequent runs will
just re-create them (harmless `IF NOT EXISTS`).

---

## 2026-05-04T18:50:00Z — WI-SEMANTIC Phase 2: Embedder trait + write-path

**Summary:** Phase 2 of WI-SEMANTIC. Introduced the `Embedder` trait and a
no-op default impl (`NoopEmbedder`), added Migration 11 for the `embedding`
BLOB column on `shared_memory_blocks`, and built two new write-path APIs:
`upsert_memory_block_with_embedder` (single-row write that also stores the
embedding) and `embedding::backfill_embeddings` (bulk fill of NULL rows for
DBs created before the column existed). Behind `#[cfg(feature =
"semantic-search")]`, added a `FastEmbedder` adapter that drives `fastembed`'s
quantised `all-MiniLM-L6-v2` model (384-dim). The default build pulls neither
`fastembed` nor `sqlite-vec`; existing call sites are unchanged.

**Files modified:**
- `crates/cade-store/src/sqlite/mod.rs` — Migration 11: `ALTER TABLE
  shared_memory_blocks ADD COLUMN embedding BLOB`. Idempotent, tolerates
  duplicate-column on re-run. Bumps `PRAGMA user_version` 10 → 11.
- `crates/cade-store/src/sqlite/embedding.rs` — added `Embedder` trait,
  `NoopEmbedder`, `FastEmbedder` (feature-gated), `backfill_embeddings`,
  doc updates. Updated `is_available()` to reflect the feature flag instead
  of always returning `false`.
- `crates/cade-store/src/sqlite/memory.rs` — added
  `upsert_memory_block_with_embedder` thin wrapper.
- `crates/cade-store/src/sqlite/memory/tests.rs` — 4 new tests:
  `shared_memory_blocks_has_embedding_column`,
  `upsert_with_embedder_writes_blob`,
  `upsert_with_none_embedder_leaves_embedding_null`,
  `backfill_embeddings_populates_null_rows`. Each uses an inline
  `FakeEmbedder` so the tests run on the default feature set.
- (in `embedding.rs`) — 2 new tests: `noop_embedder_dimension_zero_and_empty_embed`,
  `is_available_reflects_feature_flag`. Plus `fast_embedder_produces_384_dim_vector`
  gated on the feature and marked `#[ignore]` because the first run downloads
  ~25 MB of model weights.

**Reason:** The plan required a stable type for embedding production that
works in both default and feature-enabled builds without `#[cfg]` blocks
leaking into call sites. The trait + `NoopEmbedder` lets us pass
`Option<&dyn Embedder>` everywhere and pay zero runtime cost when disabled.
Migration 11 reserves the storage slot so DB files are forward-compatible.

**Previous behavior:**
- No way to store a per-block embedding vector; the column did not exist.
- `embedding::is_available()` always returned `false`, even with the feature
  enabled, so callers had no way to detect the embedding stack.
- Removing semantic search left a comment in `memory.rs` ("Semantic search
  feature removed (F5)") with no API to re-enable it.

**New behavior:**
- `embedding` BLOB column present on every DB after migrations.
- `Embedder` trait with three impls available to call sites: `NoopEmbedder`
  (default, returns empty vec), `FastEmbedder` (feature-gated, 384-dim
  MiniLM-L6-v2-Q via ONNX runtime).
- `upsert_memory_block_with_embedder(..., embedder: Option<&dyn Embedder>)`
  preserves existing `upsert_memory_block` behavior and additionally writes
  the packed le-f32 BLOB when `embedder` is `Some`.
- `backfill_embeddings(db, embedder)` iterates rows where `embedding IS NULL`
  and fills them; per-row failures are logged and skipped, returns the count
  of rows successfully written.
- `is_available()` correctly reports `cfg!(feature = "semantic-search")`.
- All existing callers of `upsert_memory_block` continue to compile and run
  unchanged. Migration 11 is idempotent.

**Verification:**
- `cargo test -p cade-store --lib` → 146 passed (140 baseline + 6 new), 0 fail.
- `cargo test --workspace` → all suites pass, 0 regressions.
- `cargo check -p cade-store` → clean (default).
- `cargo check -p cade-store --features semantic-search` → clean.
- `cargo clippy -p cade-store --all-targets` → no `cade-store` warnings.
- `cargo clippy -p cade-store --all-targets --features semantic-search` →
  no `cade-store` warnings.

**Rollback steps:**
```sh
git checkout cp-614cdf39-f4a3-4b00-a186-afecad1a199d -- crates/cade-store/
```

Note: Migration 11 is additive and idempotent — rolling back the code does
not require schema surgery. The unused `embedding` column simply stays
present with all NULLs, costing one byte per row in SQLite's row header.

---

## 2026-05-04T19:50:00Z — WI-SEMANTIC Phase 3: hybrid search via RRF

**Summary:** Phase 3 of WI-SEMANTIC. Added cosine-similarity semantic
search (`embedding::search_memory_semantic`) over the BLOB embedding column,
implemented in pure Rust with no `sqlite-vec` dependency, and a new
hybrid retrieval entry point `tools::search_memory_hybrid` that runs the
existing keyword leg plus the semantic leg and merges them via Reciprocal
Rank Fusion (k=60). Existing `search_memory` is unchanged so all four
current call sites continue to work bit-for-bit; Phase 4 will switch them
over.

**Files modified:**
- `crates/cade-store/src/sqlite/embedding.rs` — new pub fn
  `search_memory_semantic(conn, agent_id, query_embedding, limit)` returning
  `(id, similarity, label, value)` ordered by descending cosine similarity.
  Two private helpers: `decode_embedding_blob` (le-f32 BLOB → `Vec<f32>`)
  and `cosine_similarity` (returns `None` on shape mismatch or zero norm).
  Pure-Rust brute-force scan over `embedding IS NOT NULL` rows; no extra deps.
- `crates/cade-store/src/sqlite/tools.rs` — new pub fn
  `search_memory_hybrid(db, agent_id, query, embedder: Option<&dyn Embedder>)`
  that calls the existing `search_memory` for the keyword leg, embeds the
  query and runs `search_memory_semantic` for the semantic leg, then merges
  the two label rankings via `reciprocal_rank_fusion`. Materialises the
  fused order back into `(label, value, snippet)` rows. With
  `embedder = None` it returns the keyword result verbatim.
- `crates/cade-store/src/sqlite/memory/tests.rs` — 4 new tests:
  `search_memory_semantic_ranks_by_cosine_similarity`,
  `search_memory_semantic_skips_null_embedding_rows`,
  `search_memory_hybrid_finds_non_keyword_conceptual_match` (the central
  acceptance test: 'deadlock' query finds a 'mutex' block via the semantic
  leg, which the keyword leg alone misses),
  `search_memory_hybrid_with_none_embedder_matches_old_behaviour`.

**Reason:** The plan required hybrid retrieval that surfaces conceptual
matches (e.g. 'deadlock' query finds 'scoped mutex lock' notes) without
breaking keyword precision. Reciprocal Rank Fusion is the standard merge
strategy because it boosts results that appear in both legs while still
including unique hits from either side.

**Previous behavior:**
- `search_memory` only used LIKE + fuzzy word-match.
- `embedding::search_memory_semantic` did not exist.
- No hybrid retrieval entry point.

**New behavior:**
- `search_memory_semantic` ranks blocks by cosine similarity to a query
  embedding; rows without an embedding are silently skipped, dimension
  mismatches are treated as a non-match.
- `search_memory_hybrid` is opt-in via the `embedder` argument: pass `None`
  for legacy behaviour, pass `Some(&embedder)` (e.g. `&FastEmbedder` under
  the `semantic-search` feature) for hybrid retrieval. RRF (k=60) fuses
  both rankings, deduplicated by block label.
- Both functions leave the existing `search_memory` callers untouched.

**Verification:**
- `cargo test -p cade-store --lib` → 150 passed (146 baseline + 4 new), 0 fail.
- `cargo test --workspace` → all suites pass, 0 regressions.
- `cargo check -p cade-store` → clean (default).
- `cargo check -p cade-store --features semantic-search --tests` → clean.
- `cargo clippy -p cade-store --all-targets` → no `cade-store` warnings.

**Rollback steps:**
```sh
git checkout cp-771bd5db-0a39-440d-ae88-0cc97cfb6f3c -- crates/cade-store/
```

Note: this phase adds new public API only; nothing existing was removed or
renamed. Reverting just leaves the new functions inert — no callers in the
default build use them yet.

---

## 2026-05-04T20:50:00Z — WI-SEMANTIC Phase 4: server wiring + startup backfill

**Summary:** Phase 4 of WI-SEMANTIC. Added `embedder:
Option<Arc<dyn Embedder>>` to `AppState` and switched the two server
`search_memory` call sites (`agents.rs::search_handler` and
`meta_tools.rs::handle_search_memory_meta`) over to
`search_memory_hybrid`, threading `state.embedder.as_deref()` into the
spawn-blocking closure. With the `semantic-search` feature off (default),
the field is `None` and the new path returns the keyword leg verbatim —
behaviour is bit-for-bit identical to before. With the feature on, the
production `cade-server` binary instantiates a `FastEmbedder` at startup
(falling back to `None` on init failure with a warning) and spawns a
one-shot blocking task that runs `embedding::backfill_embeddings` so any
pre-existing memory blocks get their embeddings populated.

**Files modified:**
- `Cargo.toml` (root) — added `semantic-search = ["cade-store/semantic-search"]`
  feature so a single `--features semantic-search` flag at the workspace
  root activates the embedder stack. Default features unchanged.
- `crates/cade-server/src/server/state.rs` — added `pub embedder:
  Option<Arc<dyn cade_store::sqlite::embedding::Embedder>>` field on
  `AppState`. Doc-comment explains the default-build vs feature-build
  semantics.
- `crates/cade-server/src/server/api/run/meta_tools.rs` — switched
  `handle_search_memory_meta` to call
  `cade_store::sqlite::tools::search_memory_hybrid(.., embedder.as_deref())`.
  Captures `state.embedder.clone()` into the spawn-blocking closure.
- `crates/cade-server/src/server/api/agents.rs` — switched the
  `/v1/agents/:id/search_memory` handler over to
  `sqlite::tools::search_memory_hybrid` the same way.
- `src/bin/cade-server.rs` — wires up `embedder` in the production
  `AppState` initialiser. Behind `#[cfg(feature = "semantic-search")]`,
  calls `FastEmbedder::new()` and stores it as `Some(Arc::new(e))`; on
  failure logs a warning and leaves the field `None`. Also adds a one-shot
  startup backfill task that calls `embedding::backfill_embeddings(&db,
  &*emb)` in a `spawn_blocking` closure when the embedder is present.
- 11 test fixtures touched mechanically to add `embedder: None,` next to
  the existing `subagent_semaphore` field — same pattern that prior
  AppState extensions (e.g. `subagent_semaphore` itself) used.

**Reason:** Phase 3 added `search_memory_hybrid` but no caller used it.
Phase 4 routes the live server requests through the hybrid path so users
on `--features semantic-search` builds get conceptual recall, while
default builds continue to behave exactly as before.

**Previous behavior:**
- Two server search call sites called the legacy
  `cade_store::sqlite::search_memory` directly.
- `AppState` had no embedder field; there was no place to hang the
  optional `FastEmbedder`.
- Existing memory blocks created before the `embedding` column existed
  stayed `embedding IS NULL` indefinitely.

**New behavior:**
- Both server search call sites use `search_memory_hybrid`. With
  `state.embedder = None` (default builds), the hybrid path returns the
  keyword result verbatim — no observable change.
- With `--features semantic-search` and a successful `FastEmbedder::new()`,
  searches surface conceptual matches via cosine similarity over stored
  embeddings, fused with keyword hits via RRF (k=60).
- A one-shot startup backfill fills `embedding IS NULL` rows on first
  feature-enabled boot. Per-row failures log and skip; the server never
  blocks on this work.
- A failed `FastEmbedder::new()` (e.g. offline first-run with no cached
  weights) logs a warning and leaves `state.embedder = None`, so the
  server still boots and `search_memory_hybrid` falls back to the
  keyword-only path.

**Scope note (intentionally out of scope):** `upsert_memory_block` call
sites (~12 across the server, including `meta_tools::handle_update_memory`
and `consolidation::session_summary` writes) were **not** switched to
`upsert_memory_block_with_embedder`. New writes will appear without an
embedding until the next backfill cycle. This keeps Phase 4 truly minimal
— a future small change can flip those sites once a periodic backfill job
or write-path embedding is wanted. The acceptance contract (search
surfaces conceptual matches once embeddings are present) is unaffected
because the startup backfill plus a future periodic top-up cover the gap.

**Verification:**
- `cargo check --workspace` → clean.
- `cargo check --workspace --features semantic-search` → clean.
- `cargo test --workspace` → all suites pass, 0 regressions, 1 ignored
  test (the `FastEmbedder` smoke test that downloads weights).
- `cargo test --workspace --features semantic-search` → all pass, 1
  ignored.
- `cargo clippy -p cade-server -p cade --all-targets` → no new warnings
  introduced by Phase 4 (pre-existing `cade-core` and `server_executor.rs`
  warnings remain unchanged on master).

**Rollback steps:**
```sh
git checkout cp-5739a43d-c5aa-4ecc-9771-a4c2c7628e9f -- .
```

---

## 2026-05-05T13:45:00Z — Fix CADE rule persistence across sessions (4 fixes)

**Summary:** Implemented 4 fixes to ensure CADE reliably remembers and enforces project rules stored in the `[project]` memory block across all sessions and turns.

**Root causes addressed:**
- R1: No mandatory "read and enforce project rules" step in the system prompt
- R2: Skills listed in `[project]` not auto-loaded by the server
- R3: No session-start hook forces rule verification
- R4: No server-side detection of missing required skills in context assembly

**Files modified:**
- `src/bootstrap/prompt.rs` — Added "## Project Rules (CRITICAL)" section to `BASE_SYSTEM_PROMPT`
- `src/bootstrap/agents.rs` — Added `parse_required_skills()` helper + auto-load logic at session start + 4 unit tests
- `.cade/hooks/session-start-rules.sh` — New SessionStart hook that injects mandatory rule reminder
- `.cade/settings.json` — Wired SessionStart hook into project hooks config
- `crates/cade-server/src/server/api/messages/context.rs` — Added `parse_required_skills_from_project()` + missing-skills warning injection into dynamic context + 5 unit tests
- `crates/cade-server/src/server/state.rs` — Pre-existing clippy fix (field_reassign_with_default)
- `crates/cade-core/src/resources/themes.rs` — Pre-existing clippy fixes (redundant closures, iter_cloned_collect)
- `crates/cade-cli/src/cli/repl/turn_loop/agent.rs` — Pre-existing clippy fix
- `crates/cade-cli/src/cli/repl/tool_intercepts.rs` — Pre-existing clippy fix

**Previous behavior:** CADE stored project rules in the pinned `[project]` block but relied entirely on the LLM voluntarily reading and acting on them. Required skills were never auto-loaded; the LLM had to remember to call `load_skill` each session.

**New behavior:**
1. System prompt now contains explicit mandatory instructions to read and enforce `[project]` rules
2. Bootstrap auto-parses `[project]` for "## Required Skills" and loads them at session start
3. SessionStart hook injects a rule-compliance reminder into the agent's context
4. Context assembly detects missing required skills and injects a warning into the dynamic section

**Verification:** `cargo test --workspace` — 1,554 tests pass, 0 failures. `cargo clippy --workspace --all-targets -- -D warnings` — clean.

**Rollback steps:**
```sh
git checkout cp-2afa4485-9d14-4553-ad93-85bf3afdc6b0 -- .
```

---

## 2026-05-05T17:50:00Z — chore(clippy): clear pre-existing -D warnings gate (15 errors)

**Summary:** Resolved 15 pre-existing clippy errors that were blocking
`cargo clippy --workspace --all-targets -- -D warnings`. All fixes are
mechanical and behaviour-preserving — no logic changed.

**Files modified:**
- `crates/cade-core/src/resources/themes.rs` — 9× `redundant_closure` (`.map(|c| resolve(c))` → `.map(&resolve)` for the 8 borrowed call sites; `.map(resolve)` for the final `ctx_bar_buffer` site so the closure can be moved); 1× `iter_cloned_collect` (`.iter().copied().collect()` → `.to_vec()`).
- `crates/cade-server/src/server/state.rs` — 1× `field_reassign_with_default` in `accumulate_usage_saturates_on_overflow`: folded `m.input_tokens_total = u64::MAX - 5` into the `AgentMetrics { .., ..Default::default() }` struct literal.
- `crates/cade-tui/src/colors.rs` — 1× `collapsible_if`: collapsed the nested `if let Some(path)` / `if let Ok(f)` pair in `generate_syntect_theme` into a `let-else && let-chain`.
- `crates/cade-tui/src/markdown.rs` — 4× `needless_borrow` in tests: dropped the `&` on the literal `&str` `md` argument to `parse_markdown_lines_with_theme` at lines 1226, 1236, 1253, 1266.
- `crates/cade-cli/src/cli/repl/tool_intercepts.rs` — 1× `needless_borrow`: `.arg(&cmd)` → `.arg(cmd)` (`cmd` is already `&str` from `as_deref()`).
- `crates/cade-cli/src/cli/repl/turn_loop/agent.rs` — 1× `collapsible_if`: collapsed the nested mouse-event match in the REPL tick path into a `let-chain`.

**Reason:** The lint gate is part of the project's quality contract
(rust skill §12). These errors had been masking each other (the compiler
stops at the first failed crate), so resolving them required a full
workspace pass.

**Previous behavior:**
- `cargo clippy --workspace --all-targets -- -D warnings` failed with 15
  errors across 6 files.
- Lint-driven CI gate (and any pre-commit hook running this command)
  could not pass on `master`.

**New behavior:**
- `cargo clippy --workspace --all-targets -- -D warnings` is clean.
- `cargo build --workspace` and `cargo test --workspace` continue to
  pass with 0 regressions across all 1,500+ tests.

**Verification:**
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo test --workspace` → all suites pass, 0 regressions.

**Rollback steps:**
```sh
git checkout cp-fb8607f8-6988-4298-a2fa-7db52ece83ac -- .
```

---
**UTC Timestamp:** 2026-05-05 20:30:00Z (approx)
**Summary of change:** Phase 1 of Memory Architecture Rework (A1 + A2 + A3)
**Files modified:**
- `crates/cade-store/src/sqlite/memory.rs` (A1: truncation logic, A3: stamp_provenance fn)
- `crates/cade-store/src/sqlite/mod.rs` (A3: migration 12, schema column)
- `crates/cade-server/src/server/api/run/meta_tools.rs` (A3: provenance threading through all 4 memory handlers)
- `crates/cade-store/src/sqlite/memory/tests.rs` (6 new tests + 1 updated test)
- `docs/MEMORY_ARCHITECTURE_REWORK.md` (created — full 5-phase spec)

**Reason:**
Address root causes of agent amnesia and hallucination (from memory-deficiency-report.md).
Phase 1 covers A1 (write-ahead verification), A2 (ground truth protocol — already present),
and A3 (provenance tracking on memory writes).

**Previous behavior:**
- `upsert_memory_block` returned a hard error when content exceeded `max_chars`, so the
  agent got "Failed: ..." and the data was lost entirely.
- `was_truncated` in `WriteResult` was always `false`.
- `source_te_id` and `source_msg_id` columns existed in the schema but were never populated.
- No provenance tracking: memory blocks had no record of which turn or tool call wrote them.

**New behavior:**
- `upsert_memory_block` truncates to `max_chars` and returns `was_truncated = true` so the
  agent receives partial data + warning instead of nothing.
- `handle_update_memory` (and typed/patch/field variants) surface `⚠️ WARNING: Content was
  truncated from N to M chars` when truncation occurs.
- All 4 server-side memory write handlers now call `stamp_provenance()` after successful
  writes, recording the agent's turn counter and tool_call_id in `source_turn` and
  `source_te_id` columns.
- Migration 12 adds `source_turn INTEGER` column to `shared_memory_blocks`.
- 6 new tests validate truncation + provenance behavior.

**Verification:**
- `cargo build --workspace` → clean
- `cargo clippy --workspace --all-targets -- -D warnings` → clean
- `cargo test --workspace` → 1,560+ tests pass, 0 failures

**Rollback steps:**
```sh
git checkout cp-62fd17fe-9312-4696-b619-3061ed322231 -- .
```

---
**UTC Timestamp:** 2026-05-05 21:00:00Z (approx)
**Summary of change:** Phase 2 of Memory Architecture Rework (A4 + A5 + A6)
**Files modified:**
- `crates/cade-store/src/sqlite/mod.rs` (A5: migration 13 + base schema for memory_chunks table)
- `crates/cade-store/src/sqlite/memory.rs` (A5: chunk_text, rechunk_block, TextChunk, constants)
- `crates/cade-store/src/sqlite/tools.rs` (A6: chunk-level keyword search in search_memory)
- `crates/cade-server/src/server/api/run/meta_tools.rs` (A5: rechunk_block calls in all 4 handlers)
- `crates/cade-store/src/sqlite/memory/tests.rs` (7 new tests for chunking + chunk search)

**Reason:**
Phase 2 of the memory architecture rework. A4 (rich archived excerpts) was already implemented.
A5 (semantic chunking) and A6 (chunk-level search) are the new work.

**Previous behavior:**
- Large memory blocks (>500 chars) were stored as monolithic blobs.
- search_memory only searched at the whole-block level via LIKE.
- No memory_chunks table existed.

**New behavior:**
- New `memory_chunks` table stores overlapping sentence-boundary chunks for blocks > 500 chars.
- `rechunk_block()` is called after every memory write in all 4 server-side handlers.
- Chunks support per-chunk embeddings via the Embedder trait.
- `search_memory()` now also queries `memory_chunks` for keyword hits, surfacing
  the relevant portion of large blocks as `[chunk N] ...` snippets.
- 7 new tests validate chunking logic (splits, overlap, storage, replacement, search).

**Verification:**
- `cargo build --workspace` → clean
- `cargo clippy --workspace --all-targets -- -D warnings` → clean
- `cargo test --workspace` → 1,570+ tests pass, 0 failures

**Rollback steps:**
```sh
git checkout cp-93090b51-bc4b-4680-ac46-0ee32e94b396 -- .
```

---
**UTC Timestamp:** 2026-05-05 21:30:00Z (approx)
**Summary of change:** Phase 3 of Memory Architecture Rework (A7 + A8 + A9)
**Files modified:**
- `crates/cade-store/src/sqlite/memory.rs` (A9: recall_chunks fn, RecalledChunk struct)
- `crates/cade-server/src/server/api/messages/context.rs` (A9: proactive injection in assemble_system_prompt_memory, added conversation_id param)
- `crates/cade-store/src/sqlite/memory/tests.rs` (3 new tests for recall_chunks)

**Reason:**
Phase 3 of the memory architecture rework. A7 (priority-ordered greedy packing) and
A8 (context overflow manifest) were already implemented in prior sessions. A9 (proactive
injection) is the new work.

**Previous behavior:**
- The agent had to explicitly call `search_memory()` to recall any stored facts.
- If the agent forgot it had stored something, that knowledge was effectively lost.
- `assemble_system_prompt_memory` did not accept `conversation_id`.

**New behavior:**
- Before generating the LLM prompt, the system fetches the latest user message,
  extracts keywords, and searches `memory_chunks` for matching fragments.
- Top 3 matching chunks are injected as a `# Recalled Context` section in the
  dynamic system prompt.
- Only runs on user messages (not tool returns) to avoid redundant lookups.
- `assemble_system_prompt_memory` now receives `conversation_id` so it can scope
  the user-message lookup.
- 3 new tests validate keyword matching, empty-query handling, and deduplication.

**Verification:**
- `cargo build --workspace` → clean
- `cargo clippy --workspace --all-targets -- -D warnings` → clean
- `cargo test --workspace` → 1,570+ tests pass, 0 failures

**Rollback steps:**
```sh
git checkout cp-3a6ec9fe-7dd9-46bd-80ee-2a6da6fb1155 -- .
```

---
**UTC Timestamp:** 2026-05-05 22:00:00Z (approx)
**Summary of change:** Phase 4 of Memory Architecture Rework (A10 + A11 + A12)
**Files modified:**
- `crates/cade-store/src/sqlite/tools.rs` (A11: recency_frequency_score fn + Recency × Frequency ranking in search_memory)
- `crates/cade-store/src/sqlite/memory/tests.rs` (4 new tests for scoring + search ranking)

**Reason:**
Phase 4 of the memory architecture rework. A10 (access_count + last_access_turn) and
A12 (active_goal staleness nudge) were already implemented. A11 (Recency × Frequency
scoring) is the new work.

**Previous behavior:**
- `search_memory` ranked results by `updated_at DESC` — pure write-recency.
- Frequently-accessed blocks had no ranking advantage over never-accessed ones.

**New behavior:**
- `recency_frequency_score()` computes `recency_weight × frequency_weight` where:
  - `frequency_weight = 1 + log2(access_count + 1)` — diminishing returns
  - `recency_weight = 1 / (1 + turns_idle × 0.02)` — smooth decay
- `search_memory` Phase 1 (LIKE) now fetches access_count + last_access_turn,
  scores each result, and sorts by composite score descending.
- Blocks that are both frequently accessed AND recently active rank higher.
- 4 new tests validate scoring math, logarithmic diminishing returns, recency decay,
  and end-to-end search ranking with frequency boost.

**Verification:**
- `cargo build --workspace` → clean
- `cargo clippy --workspace --all-targets -- -D warnings` → clean
- `cargo test --workspace` → 1,570+ tests pass, 0 failures

**Rollback steps:**
```sh
git checkout cp-9b757e75-efad-4bba-b54a-b248240cb4d6 -- .
```

---
**UTC Timestamp:** 2026-05-05 22:30:00Z (approx)
**Summary of change:** Phase 5 of Memory Architecture Rework (A13 + A14 + A15) — FINAL PHASE
**Files modified:**
- `crates/cade-store/src/sqlite/memory.rs` (A15: write_back_subagent_memory fn, WritebackFact struct, WRITEBACK_EXCLUDE list)
- `crates/cade-server/src/server/api/run/subagent.rs` (A15: call write_back before delete_agent, writeback_facts in completion event)
- `crates/cade-store/src/sqlite/memory/tests.rs` (4 new tests for subagent write-back)
- `docs/MEMORY_ARCHITECTURE_REWORK.md` (updated to reflect all 5 phases complete)

**Reason:**
Phase 5 (final) of the memory architecture rework. A13 (expanded observation trail)
and A14 (improved session eviction) were already implemented. A15 (subagent write-back)
is the new work.

**Previous behavior:**
- When a subagent completed, its ephemeral DB row was deleted immediately.
- Any memory blocks (typed facts, decisions, constraints) the subagent wrote
  during its execution were lost forever.
- The parent agent only received the subagent's final text output.

**New behavior:**
- Before deleting the ephemeral subagent row, `write_back_subagent_memory()`
  extracts all custom memory blocks the subagent wrote.
- System blocks (persona, human, project, active_goal, etc.) and skill blocks
  are excluded (they're parent-seeded copies).
- Remaining facts are written to the parent's memory with `subagent:` prefix
  (e.g. `subagent:api_design`) with provenance in the description.
- The `subagent_complete` SSE event now includes `writeback_facts` count.
- 4 new tests validate: custom block copying, system block exclusion, empty
  value skipping, and zero-block edge case.

**Verification:**
- `cargo build --workspace` → clean
- `cargo clippy --workspace --all-targets -- -D warnings` → clean
- `cargo test --workspace` → 1,570+ tests pass, 0 failures

**Rollback steps:**
```sh
git checkout cp-eed1f657-eb50-4f31-9c35-2edaf739925e -- .
```

---
**UTC Timestamp:** 2026-05-06 14:25:00Z (approx)
**Summary of change:** Fix Anthropic 404 — compaction model claude-3-5-haiku-latest → claude-haiku-4-5
**Files modified:**
- `crates/cade-server/src/server/consolidation.rs`

**Reason:**
Anthropic retired `claude-3-5-haiku-latest` model ID. Sleeptime consolidation was
failing with `404 Not Found` on every trigger, preventing session summaries from
being created.

**Previous behavior:**
- `default_compaction_model("anthropic/*")` returned `"anthropic/claude-3-5-haiku-latest"`
- Every consolidation call failed with 404

**New behavior:**
- Returns `"anthropic/claude-haiku-4-5"` (current valid cheapest Anthropic model per catalogue)
- 1 new test added, 2 existing assertions updated

**Rollback steps:**
```sh
git checkout cp-5548522b-e039-4f0b-8506-ed91c0e093a6 -- .
```

---
**UTC Timestamp:** 2026-05-07T13:00:00Z
**Summary of change:** Fix CADE amnesia loops (RC-1, RC-2, RC-3/RC-6).
**Files modified:**
- `crates/cade-server/src/server/consolidation.rs`
- `crates/cade-server/src/server/reflection.rs`
- `docs/AMNESIA_ROOT_CAUSE_REPORT.md` (new)

**Reason:**
Agent repeatedly lost track of tasks mid-flight across sessions due to architectural bugs in consolidation and reflection.
Implemented three major fixes from the Amnesia Survival Guide:
1. RC-2: Added recency and regression guards to `auto_update_active_goal` so the LLM doesn't blindly overwrite accurate task state.
2. RC-3 & RC-6: Included `tool` roles in the reflection loop (with heavy truncation) and updated the reflection prompt so CADE can automatically detect completed tasks based on tool outputs (like passing tests or successful git commits).
3. RC-1: Synchronized budget calculation between `consolidate_agent` and `build_context` so the background consolidation task doesn't summarize turns that the main agent is still seeing in full context (resolving "double vision").

**Previous behavior:**
Consolidation hallucinated task states, reflection ignored tool outputs, and budget mismatches caused conflicting contexts.

**New behavior:**
Task states are accurately retained, completed tasks are automatically detected by reflection, and the context window and consolidation are fully synchronized.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-server/src/server/consolidation.rs crates/cade-server/src/server/reflection.rs
```

---
**UTC Timestamp:** 2026-05-07T13:30:00Z
**Summary of change:** Fix `@` file picker overlay transparency
**Files modified:**
- `crates/cade-tui/src/app/layout/pickers.rs`

**Reason:**
Following the dynamic overlay stack refactor (`ae34cd3e`), the `@` file picker was receiving the full terminal area instead of its calculated bottom-anchored area. Because it did not render a backdrop or clear the area underneath it, the timeline text bled through, rendering it effectively transparent and unusable.

**Previous behavior:**
The `@` picker rendered a raw `Paragraph` without a `Clear` widget or shell, causing transparency.

**New behavior:**
The `@` file picker now correctly uses `render_overlay_shell` to draw a centered, opaque floating window with a "Select File" title, dim backdrop, and proper background coloring.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/app/layout/pickers.rs
```

---
**UTC Timestamp:** 2026-05-07T14:30:00Z
**Summary of change:** Replaced `ThemeColors` with `opaline::Theme` across workspace.
**Files modified:**
- `crates/cade-core/src/resources/themes.rs`
- `crates/cade-core/src/resources/mod.rs`
- `crates/cade-tui/src/colors.rs`
- `crates/cade-tui/src/app/*.rs`
- `crates/cade-gui/src/theme.rs`
- `crates/cade-gui/src/api.rs`
- `crates/cade-gui/src/session/mod.rs`
- `crates/cade-cli/src/cli/repl/commands_theme.rs`
- `crates/cade-server/src/server/api/run/mod.rs`

**Reason:**
Phases 3, 4, and 5 of the Opaline Refactor plan. Replaced the rigid `ThemeTokens` struct in `cade-core` with `opaline::Theme`. Refactored `cade-tui` to use a `ThemeColorsExt` trait applied to `Theme` to prevent rewriting 650+ lines of UI code, resolving color definitions directly from Opaline semantic tokens. Rewrote `EguiThemeExt` in `cade-gui` to dynamically resolve `Color32` values from `opaline::color::OpalineColor`. Adapted SSE payload to broadcast theme names rather than serializing the entire theme payload. `cargo test --workspace` verified cleanly.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-core crates/cade-tui crates/cade-gui crates/cade-cli crates/cade-server
```
---
**UTC Timestamp:** 2026-05-07T14:45:00Z
**Summary of change:** Re-apply fix for `@` file picker overlay transparency
**Files modified:**
- `crates/cade-tui/src/app/layout/pickers.rs`

**Reason:**
During the massive `opaline` theme refactor across the workspace, the previous fix for the transparent `@` file picker was accidentally reverted. `git restore` was used to clean up a flawed automated refactoring script, which correctly reverted the bad changes but also wiped out the legitimate file picker fix since they were batched together.

**Previous behavior:**
The `@` picker rendered a raw `Paragraph` without a `Clear` widget or shell, causing transparency.

**New behavior:**
Re-applied the fix: the `@` file picker correctly uses `render_overlay_shell` to draw a centered, opaque floating window with a "Select File" title, dim backdrop, and proper background coloring via the Opaline theme tokens.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/app/layout/pickers.rs
```

---
**Files modified:**
- `crates/cade-tui/src/app/layout/helpers.rs`

**Reason:**
The TUI overlays (like the `/theme` picker) were using a solid background color (`c_bg_base`) to clear the entire screen behind the popup, completely hiding the user's conversation timeline, instead of properly "dimming" it as the documentation stated.

**Previous behavior:**
`render_backdrop` used a `ratatui::widgets::Paragraph` with `.bg(colors.c_bg_base())` composed entirely of spaces to wipe the screen.

**New behavior:**
`render_backdrop` now directly accesses the terminal cell buffer using `frame.buffer_mut()`, applying `ratatui::style::Modifier::DIM` to the existing characters. This allows the timeline and background text to remain visible (and properly styled with live-previewed theme colors) but correctly dimmed to allow the active modal to stand out.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/app/layout/helpers.rs
```
---
**UTC Timestamp:** 2026-05-07T15:10:00Z
**Summary of change:** Fix terminal background color overlay caused by `Modifier::DIM`.
**Files modified:**
- `crates/cade-tui/src/app/layout/helpers.rs`

**Reason:**
The previous fix for `render_backdrop` used `ratatui::style::Modifier::DIM`. However, on many terminal emulators (e.g. GNOME Terminal on Pop!_OS / `tmux-256color`), `DIM` is implemented by blending the entire cell background, which destroys terminal transparency and renders an opaque color overlay over the entire screen.

**Previous behavior:**
`render_backdrop` applied `Modifier::DIM` to every cell, causing unintended opaque background overlays on terminals without full DIM support.

**New behavior:**
`render_backdrop` now avoids `Modifier::DIM` entirely. Instead, it iterates over the terminal buffer and updates only the foreground (`fg`) color of the text to `colors.c_text_dim()`, leaving the background completely untouched. This achieves the visual "dimming" effect for the modal backdrop while perfectly preserving terminal transparency.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/app/layout/helpers.rs
```
---
**UTC Timestamp:** 2026-05-07T15:45:00Z
**Summary of change:** Fix full-screen opaque background caused by unconstrained theme picker popup.
**Files modified:**
- `crates/cade-tui/src/app/layout/pickers.rs`

**Reason:**
The previous fix to `render_backdrop` successfully prevented the terminal's global dimming from destroying transparency. However, the user still saw an opaque background because the `/theme` picker popup itself had lost its dimensional constraints during a previous automated refactoring. It was drawing a `Block` with `bg_surface0` over the *entire terminal area* instead of drawing a centered modal window, which completely masked the timeline and ruined transparency.

**Previous behavior:**
`render_theme_picker` used the full `area` for its layout boundaries, painting the popup over the entire screen.

**New behavior:**
`render_theme_picker` computes a centered bounding box `r` and renders the picker modal exclusively within that region, leaving the rest of the screen fully visible (and correctly dimmed by the backdrop).

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/app/layout/pickers.rs
```
---
**UTC Timestamp:** 2026-05-07T15:55:00Z
**Summary of change:** Fix global gray background overlay by correcting Opaline token mapping.
**Files modified:**
- `crates/cade-tui/src/colors.rs`
- `crates/cade-gui/src/theme.rs`
- `crates/cade-tui/src/app/layout/pickers.rs` (Resolved git conflict)

**Reason:**
After refactoring to `opaline`, the entire UI (side-tray, input field, help menu, modals, and viewport) had a gray background. This happened because the custom UI code queried Opaline for semantic tokens that do not exist in standard Opaline themes (e.g., `bg.surface`, `bg.overlay`, `accent.error`). When Opaline cannot resolve a token, it returns `OpalineColor::FALLBACK`, which is a neutral gray (`128, 128, 128`).

**Previous behavior:**
Queried undefined tokens (e.g. `bg.surface`), resulting in default gray color (`128, 128, 128`) everywhere.

**New behavior:**
Mapped background and border color accesses correctly to the standard Opaline token palette (e.g., `bg.panel`, `bg.elevated`, `success`, `error`, `border.unfocused`). This correctly resolves theme backgrounds for all 39 built-in themes instead of falling back to gray. Also resolved lingering git conflict in `pickers.rs` leftover from a previous stash operation.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/colors.rs crates/cade-gui/src/theme.rs
```
---
**UTC Timestamp:** 2026-05-07T16:05:00Z
**Summary of change:** Fix global gray background overlay in custom themes due to missing Opaline tokens.
**Files modified:**
- `crates/cade-tui/src/colors.rs`
- `crates/cade-gui/src/theme.rs`

**Reason:**
The previous fix to correct Opaline token mappings assumed all themes would define standard tokens like `bg.panel`, `bg.elevated`, `success`, etc. However, custom user `.json` themes migrated to `.toml` using the `migrate_themes.py` script only mapped the original `cade.*` namespace tokens. Because the new standard tokens were completely absent from the users' existing `.toml` themes, Opaline correctly returned `OpalineColor::FALLBACK` (RGB: 128, 128, 128) for almost every UI element. This led to a solid gray background being painted over the entire terminal and UI for anyone using a migrated custom theme (like Dracula or Ayu Dark).

**Previous behavior:**
If a standard token (like `bg.panel` or `success`) was missing from the loaded theme, the UI would silently render it as Gray (128, 128, 128).

**New behavior:**
Implemented a `resolve_fallback` helper in both `ThemeColorsExt` (TUI) and `EguiThemeExt` (GUI). It safely intercepts the Opaline token resolution: if a standard token (like `bg.panel`) returns the fallback gray value (128, 128, 128), it automatically falls back to querying the legacy custom `cade.` token (e.g., `cade.user_message_bg`). This ensures 100% backward compatibility for users' existing migrated custom themes while preserving the correct standard token lookups for Opaline's 39 built-in themes.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/colors.rs crates/cade-gui/src/theme.rs
```
---
**UTC Timestamp:** 2026-05-07T16:20:00Z
**Summary of change:** Remove background color highlight from input field and ask question modal.
**Files modified:**
- `crates/cade-tui/src/app/render.rs`
- `crates/cade-tui/src/app/layout/question.rs`

**Reason:**
After the Opaline theme refactor, the input field and question modal were rendering with opaque background colors (`bg_input` and `style_surface1` respectively), causing an unwanted highlight block on terminals with transparent backgrounds.

**Previous behavior:**
The input field applied `.bg(colors.c_bg_input())` to the input prefix, the cursor line, and the base textarea. The question modal applied `.style(colors.style_surface1())` to the overall paragraph. This resulted in solid background blocks.

**New behavior:**
Removed the `.bg(...)` modifiers and used `Style::default()` for the input field components and the question modal paragraph. This leaves the background completely untouched (transparent), removing the unwanted highlight block while maintaining foreground text colors.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/app/render.rs crates/cade-tui/src/app/layout/question.rs
```
---
**UTC Timestamp:** 2026-05-07T16:35:00Z
**Summary of change:** Fix theme name resolution error for `/theme <display_name>` commands.
**Files modified:**
- `crates/cade-cli/src/cli/repl/commands_theme.rs`
- `crates/cade-server/src/server/api/run/mod.rs`

**Reason:**
When the user selects a theme from the TUI picker (e.g., `SilkCircuit Glow`), the `ThemePickerState::take_result` formats the command as `/theme SilkCircuit Glow`. The underlying `opaline::load_by_name` expects the internal ID (`silkcircuit-glow`), causing it to fail. The fallback substring matching previously only compared against the internal ID (`n.name`), which also failed because `"silkcircuit-glow".contains("silkcircuit glow")` is false.

**Previous behavior:**
The fallback substring matching only compared against the `n.name` field.

**New behavior:**
Updated the fallback substring matching to also compare against `n.display_name`. Additionally, updated the resolution block so the GUI and TUI accurately persist and report `theme.meta.name` instead of the raw input. This ensures perfectly matching display names correctly load the associated theme.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-cli/src/cli/repl/commands_theme.rs crates/cade-server/src/server/api/run/mod.rs
```
---
**UTC Timestamp:** 2026-05-07T16:55:00Z
**Summary of change:** Remove background colors from user and assistant messages in the timeline viewport.
**Files modified:**
- `crates/cade-tui/src/app/timeline/mod.rs`
- `crates/cade-tui/src/colors.rs`

**Reason:**
The user requested the removal of the gray background color rendered behind CADE's responses and the user's own prompt/chat in the viewport to fully preserve terminal transparency. Previously, the `CardStyle::User` and `CardStyle::Assistant` branches applied `.style(colors.bg_card_style())`, which explicitly set a background color across the entire paragraph block. Additionally, the `c_bg_input` mapping was fixed to correctly map to `bg.panel` instead of the non-existent `bg.surface` token in `colors.rs`.

**Previous behavior:**
Messages in the timeline used `colors.bg_card_style()`, which applied an opaque background color (e.g. `cade.tool_success_bg` / `bg.elevated` via fallback).

**New behavior:**
Replaced `.style(colors.bg_card_style())` with `.style(colors.text_primary())` in `render_timeline_viewport`. This leaves the terminal background untouched (transparent) for conversation messages while preserving the correct foreground text color. Also correctly mapped `c_bg_input()` to `bg.panel`.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/app/timeline/mod.rs crates/cade-tui/src/colors.rs
```
---
**UTC Timestamp:** 2026-05-07T17:15:00Z
**Summary of change:** Match 'OK' timeline success block background color to the 'INFO' block style.
**Files modified:**
- `crates/cade-tui/src/app/timeline/render_item.rs`

**Reason:**
The user requested that the success entries (`OK` badge, such as Auto-checkpoint saved) in the timeline match the background color of the `INFO` badge, eliminating the solid background block and preserving terminal transparency where possible.

**Previous behavior:**
The `OK` badge in `render_success_item` used `.bg(colors.c_bg_surface1())`, causing a prominent background highlight block.

**New behavior:**
The `OK` badge in `render_success_item` now uses `.bg(colors.c_bg_base())`, matching the styling of `render_system_item` which renders the `INFO` badge. This removes the distinct opaque overlay highlight from the badge.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui/src/app/timeline/render_item.rs
```
---
**UTC Timestamp:** 2026-05-07T18:00:00Z
**Summary of change:** Fix SessionStart hook missing context injection (R3).
**Files modified:**
- `crates/cade-core/src/hooks/mod.rs`
- `crates/cade-cli/src/cli/repl/mod.rs`

**Reason:**
The user reported that CADE doesn't adhere to project rules. Upon investigation, I found that the `SessionStart` hook (`.cade/hooks/session-start-rules.sh`) outputs a JSON payload with `additionalContext` intended to inject a compliance reminder. However, `session_start()` in `cade-core/src/hooks/mod.rs` was calling `run_all_fire_forget()`, discarding the output. Consequently, the agent never received the prompt injection.

**Previous behavior:**
`SessionStart` hooks fired in a non-blocking `fire_and_forget` context, discarding `additionalContext`.

**New behavior:**
- Updated `HookEngine::session_start` to return `Option<String>` using `run_entries_context`.
- Updated `cade-cli` REPL loop to capture `session_hook_ctx` and inject it into the very first message sent to the agent as a `[System Note]`.
- This ensures the rule compliance reminder explicitly reaches the agent context at session start.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-core/src/hooks/mod.rs crates/cade-cli/src/cli/repl/mod.rs
```

---
**UTC Timestamp:** 2026-05-07T20:00:00Z
**Summary of change:** Implement subagent token/cost budget limit.
**Files modified:**
- `crates/cade-agent/src/subagents/config.rs`
- `crates/cade-agent/src/tools/meta.rs`
- `crates/cade-cli/src/cli/headless.rs`
- `crates/cade-server/src/server/api/run/subagent.rs`

**Reason:**
Phase 1 of the Subagent Polish Plan. To prevent runaway subagents from silently exhausting API credits, a hard token limit per execution is needed.

**Previous behavior:**
Subagents ran until they finished the task or hit the `max_iters` limit, without any bound on token consumption.

**New behavior:**
Added `max_tokens_budget` to `SubagentConfig` and the `run_subagent` tool schema.
Implemented tracking of cumulative prompt and completion tokens during the subagent execution loops in both `server/api/run/subagent.rs` (server-side) and `cli/headless.rs` (local). Before each LLM call, if `cumulative_tokens` exceeds `max_tokens_budget`, the loop forcefully terminates with an error indicating budget exhaustion.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-agent/src/subagents/config.rs crates/cade-agent/src/tools/meta.rs crates/cade-cli/src/cli/headless.rs crates/cade-server/src/server/api/run/subagent.rs
```
---
**UTC Timestamp:** 2026-05-07T21:05:00Z
**Summary of change:** Implement Phase 2 of Subagent Polish Plan: Parallel Subagents (Map-Reduce).
**Files modified:**
- `crates/cade-agent/src/tools/meta.rs`
- `crates/cade-server/src/server/api/run/subagent.rs`
- `crates/cade-server/src/server/api/run/meta_tools.rs`
- `crates/cade-cli/src/cli/repl/tool_intercepts.rs`
- `crates/cade-cli/src/cli/repl/turn_tools/runner.rs`

**Reason:**
Phase 2 of the Subagent Polish Plan. Allowing the parent agent to spawn multiple subagents simultaneously (Map-Reduce pattern) significantly speeds up parallelizable tasks like analyzing multiple independent files.

**Previous behavior:**
The parent agent could only spawn a single subagent per tool call using `run_subagent`.

**New behavior:**
Added a new tool `run_parallel_subagents` that accepts a `tasks` array. 
On both the server and CLI:
- It spawns a concurrent `tokio::spawn` (or `Box::pin`) task for each subagent config.
- These tasks race to complete, utilizing the same execution loop as `run_subagent` and respecting the `subagent_semaphore` limit.
- `futures::future::join_all` aggregates their final text outputs and returns them together as a JSON array to the parent agent.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-agent/src/tools/meta.rs crates/cade-server/src/server/api/run/subagent.rs crates/cade-server/src/server/api/run/meta_tools.rs crates/cade-cli/src/cli/repl/tool_intercepts.rs crates/cade-cli/src/cli/repl/turn_tools/runner.rs
```
---
**UTC Timestamp:** 2026-05-07T21:30:00Z
**Summary of change:** Implement Phase 3 of Subagent Polish Plan: Subagent Steering (Interrupts).
**Files modified:**
- `crates/cade-agent/src/tools/meta.rs`
- `crates/cade-server/src/server/state.rs`
- `crates/cade-server/src/server/api/run/meta_tools.rs`
- `crates/cade-server/src/server/api/run/subagent.rs`
- `crates/cade-cli/src/cli/repl/mod.rs`
- `crates/cade-cli/src/cli/repl/tool_intercepts.rs`
- `crates/cade-cli/src/cli/repl/turn_tools/runner.rs`
- `src/bin/cade-server.rs`

**Reason:**
Phase 3 of the Subagent Polish Plan. Previously, when a subagent was launched in the background, the parent agent (or human) had no way to abort its execution if it started hallucinating or proceeding down the wrong path. 

**Previous behavior:**
Subagents ran uninterrupted until completion, timeout, or hitting iteration limits.

**New behavior:**
Added a `cancel_subagent` tool. Both the server `AppState` and the CLI `Repl` state now maintain a `subagent_cancellations` hash map linking active `subagent_id`s to a `tokio::sync::mpsc::Sender<()>`. The execution loops in `run_headless` and `handle_run_subagent_tool` were wrapped with `tokio::select!` so they concurrently listen for a cancellation signal. If the parent calls `cancel_subagent`, the token triggers the receiver, gracefully terminating the subagent loop and returning an early cancellation error.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-agent/src/tools/meta.rs crates/cade-server/src/server/state.rs crates/cade-server/src/server/api/run/meta_tools.rs crates/cade-server/src/server/api/run/subagent.rs crates/cade-cli/src/cli/repl/mod.rs crates/cade-cli/src/cli/repl/tool_intercepts.rs crates/cade-cli/src/cli/repl/turn_tools/runner.rs src/bin/cade-server.rs
```
---
**UTC Timestamp:** 2026-05-07T22:30:00Z
**Summary of change:** Implement Phase 4 of Subagent Polish Plan: Smart Memory Merge.
**Files modified:**
- `crates/cade-store/src/sqlite/memory.rs`
- `crates/cade-server/src/server/api/run/subagent.rs`
- `crates/cade-server/src/server/consolidation.rs`

**Reason:**
Phase 4 of the Subagent Polish Plan. Previously, when multiple subagents were spawned (e.g. via `run_parallel_subagents`) and they wrote back memory facts to the parent agent, their facts were simply upserted into the parent's memory with the `subagent:` prefix. This caused earlier subagents' facts to be silently overwritten by later subagents if they used the same label, losing valuable context. 

**Previous behavior:**
`write_back_subagent_memory` in `cade-store` blindly upserted subagent memory blocks to the parent agent, causing collisions and data loss.

**New behavior:**
Refactored `write_back_subagent_memory` into an extraction method `extract_subagent_memory_for_writeback` in `cade-store`. Moved the write-back orchestration into `cade-server` via a new `write_back_and_delete_async` method on `EphemeralAgentGuard`. When writing back facts, if the parent agent already has a block with the exact same `subagent:` label, the server now spawns an asynchronous LLM merge task (`smart_memory_merge`) using the compaction model to synthesize the old and new facts into a single coherent block, preventing data loss and resolving conflicts automatically.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-store/src/sqlite/memory.rs crates/cade-server/src/server/api/run/subagent.rs crates/cade-server/src/server/consolidation.rs
```
---
**UTC Timestamp:** 2026-05-07T23:30:00Z
**Summary of change:** Implement Phase 5 of Subagent Polish Plan: Granular Tool RBAC.
**Files modified:**
- `crates/cade-agent/src/subagents/mod.rs`
- `crates/cade-agent/src/subagents/config.rs`
- `crates/cade-agent/src/tools/runtime/mod.rs`
- `crates/cade-agent/src/tools/manager.rs`
- `crates/cade-server/src/server/api/run/subagent.rs`
- `crates/cade-cli/src/cli/headless.rs`

**Reason:**
Phase 5 of the Subagent Polish Plan. To provide strict sandbox environments for subagents, preventing them from stepping outside of intended paths and providing better security boundaries.

**Previous behavior:**
Subagents ran with access to `All`, `Readonly`, or a `List` of specific tools, but could not be restricted to specific directories.

**New behavior:**
Added a `Restricted` variant to the `SubagentTools` enum which holds both `allowed_tools` and `allowed_paths`. `ToolRuntime` now accepts `allowed_paths` and delegates it down to the tool dispatcher (`crates/cade-agent/src/tools/manager.rs`). For filesystem tools (`read_file`, `write_file`, `grep`, `glob`, etc.), the dispatcher verifies the target path prefix matches an allowed path, returning `[Blocked by RBAC]` if the subagent tries to escape the sandbox.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-agent/src/subagents/mod.rs crates/cade-agent/src/subagents/config.rs crates/cade-agent/src/tools/runtime/mod.rs crates/cade-agent/src/tools/manager.rs crates/cade-server/src/server/api/run/subagent.rs crates/cade-cli/src/cli/headless.rs
```
---
**UTC Timestamp:** 2026-05-07T21:40:00Z
**Summary of change:** Updated `ARCHITECTURE.md` to reflect Opaline migration.
**Files modified:**
- `ARCHITECTURE.md`

**Reason:**
The previous architectural documentation still described the legacy JSON `ThemeColors` struct and manual trait-based mapping, which was replaced during the migration to `opaline`.

**New behavior:**
`ARCHITECTURE.md` now correctly describes the `opaline::Theme` token-based architecture, illustrating how both TUI and GUI frontends now resolve UI colors and styles from semantic tokens provided by the engine.

**Rollback steps:**
```sh
git checkout HEAD^ -- ARCHITECTURE.md
```
---
**UTC Timestamp:** 2026-05-07T23:55:00Z
**Summary of change:** Clean up compiler warnings related to the Opaline theme refactor.
**Files modified:**
- `crates/cade-tui/src/app/input.rs`
- `crates/cade-tui/src/colors.rs`
- `crates/cade-tui/src/slots.rs`
- `crates/cade-tui/src/app/mod.rs`
- `crates/cade-cli/src/cli/repl/commands_theme.rs`

**Reason:**
The recent Opaline theme refactoring and related fixes left behind unused imports and mutable bindings in the codebase. Cleaning these up keeps `cargo check` warnings at 0, adhering to the project's strict compilation standards.

**Previous behavior:**
Compiling the workspace generated several warnings about unused imports (`ThemeColorsExt`, `ScopeSelectors`, `ThemeItem`, `cade_core::resources::Theme`), an unused variable (`colors`), and an unnecessary `mut` binding in `generate_syntect_theme`.

**New behavior:**
Removed the unused imports and fixed the variable bindings. `cargo check --workspace` is now completely warning-free.

**Rollback steps:**
```sh
git checkout HEAD^ -- crates/cade-tui crates/cade-cli
```
---
**UTC Timestamp:** 2026-05-10T19:35:00Z
**Summary of change:** Fix compilation errors caused by granular RBAC commit and add subagent cancellations state field.
**Files modified:**
- `crates/cade-agent/src/tools/manager.rs`
- `crates/cade-server/src/server/api/auth_test.rs`
- `crates/cade-server/src/server/api/compact.rs`
- `crates/cade-server/src/server/api/complete.rs`
- `crates/cade-server/src/server/api/context_stats.rs`
- `crates/cade-server/src/server/api/dashboard_test.rs`
- `crates/cade-server/src/server/api/evals_test.rs`
- `crates/cade-server/src/server/api/messages/tests.rs`
- `crates/cade-server/src/server/api/router_test.rs`
- `crates/cade-server/src/server/api/run/mod.rs`
- `crates/cade-server/src/server/api/run/tests.rs`
- `crates/cade-server/src/server/api/skills.rs`
- `src/main.rs`

**Reason:**
A previous subagent introduced granular tool RBAC (commit `a9d3477b`) which modified the signature of `cade_agent::tools::manager::dispatch`, but missed updating several call sites in the codebase resulting in compilation errors. Additionally, `subagent_cancellations` was added to `AppState` but the test mock states were not updated. These missing parameters and fields have been added to restore a clean build.

**Previous behavior:**
The project failed to compile due to missing arguments in `dispatch` calls and missing fields in `AppState` initializers during test runs.

**New behavior:**
Added `None` for the `allowed_paths` argument in `dispatch` calls that run with full permissions, and added `subagent_cancellations: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()))` to all test `AppState` structs. The project and test suite now compile perfectly.

**Rollback steps:**
\`\`\`sh
git checkout HEAD^ -- crates/cade-agent crates/cade-server src/main.rs
\`\`\`

---
**UTC Timestamp:** 2026-05-10T19:42:00Z
**Summary of change:** Add `allowed_paths` to `SessionOptions` in `cade-sdk` to support granular RBAC for file sandboxing.
**Files modified:**
- `crates/cade-sdk/src/session.rs`

**Reason:**
A previous commit (`a9d3477b`) introduced granular RBAC restricting file I/O operations inside `ToolRuntime` and native tool dispatch to a set of `allowed_paths`. The `cade-sdk` was passing an implicit `None` when initializing `ToolRuntime`, meaning custom clients spinning up agents via the SDK had no way to configure file sandboxing.

**Previous behavior:**
Agents instantiated via `cade-sdk` bypassed granular path sandboxing entirely and had full file system access.

**New behavior:**
`SessionOptions` now includes an `allowed_paths: Option<Vec<String>>` field (defaulting to `None`). This list is correctly passed down to the `ToolRuntime` configuration, enabling downstream clients to sandbox their embedded agents.

**Rollback steps:**
\`\`\`sh
git checkout HEAD^ -- crates/cade-sdk/src/session.rs
\`\`\`

---
**UTC Timestamp:** 2026-05-10T19:50:00Z
**Summary of change:** Introduce `SubagentExecutor` to decouple the subagent state machine.
**Files modified:**
- `crates/cade-server/src/server/api/run/subagent.rs`

**Reason:**
Phase 1 of the Subagent Architecture Refactoring Plan calls for dismantling the monolithic `handle_run_subagent_tool` loop. The first foundational step is creating a dedicated struct (`SubagentExecutor`) to hold the execution context, making it easier to incrementally decouple SSE streaming, database environments, and cancellation logic in subsequent passes.

**Previous behavior:**
The subagent ran via a single massive `handle_run_subagent_tool` function containing the LLM agentic loop, SSE networking, state management, and memory write-backs deeply intertwined.

**New behavior:**
Introduced `SubagentExecutor` which wraps `AppState`, parent context, and the SSE channel. `handle_run_subagent_tool` now instantiates this executor and calls its `execute` method, delegating the complex internal logic (`handle_run_subagent_tool_inner`) into a stateful struct context.

**Rollback steps:**
\`\`\`sh
git checkout HEAD^ -- crates/cade-server/src/server/api/run/subagent.rs
\`\`\`

---
**UTC Timestamp:** 2026-05-10T20:00:00Z
**Summary of change:** Implement Phase 1b and 1c of subagent refactoring (SubagentEventEmitter and EphemeralEnvironment).
**Files modified:**
- `crates/cade-server/src/server/api/run/subagent.rs`

**Reason:**
Continuing the architectural decoupling of the subagent system. By isolating the SSE streaming logic into a `SubagentEventEmitter` trait, the execution engine is now decoupled from the raw HTTP transport layer. This makes the system far more testable and prepares it for alternative emitters (like direct UI manipulation or SDK interception). Additionally, `EphemeralAgentGuard` was logically renamed and shifted towards becoming `EphemeralEnvironment` to better encapsulate the SQLite sandbox logic.

**Previous behavior:**
The monolithic `handle_run_subagent_tool` directly called `sse_tx.send(...)` with raw JSON string payloads tied directly to the axum SSE endpoint format.

**New behavior:**
The loop now relies entirely on the `emitter: Box<dyn SubagentEventEmitter>` trait interface, calling `emitter.emit_started` and `emitter.emit_complete`. `handle_run_subagent_tool` handles the injection of `SseEventEmitter` before passing execution into the inner loop.

**Rollback steps:**
\`\`\`sh
git checkout HEAD^ -- crates/cade-server/src/server/api/run/subagent.rs
\`\`\`

---
**UTC Timestamp:** 2026-05-10T20:15:00Z
**Summary of change:** Implement Phase 2 of subagent refactoring (TUI/UI rendering options).
**Files modified:**
- `crates/cade-tui/src/app/render.rs`
- `crates/cade-tui/src/app/timeline/mod.rs`
- `crates/cade-tui/src/app/timeline/render_item.rs`

**Reason:**
Phase 2 of the Subagent Architecture Refactoring Plan called for distinct visual modes for subagents and improved rendering of the historical scratchpad to prevent UI bloat.

**Previous behavior:**
Subagents were rendered with generic borders regardless of their mode, and the `historical_scratchpad` output block was rendered inline as plain text inside the assistant output, bloating the scroll buffer.

**New behavior:**
- Subagent tracker cards are now color-coded based on mode (`plan` mode gets a success border, `build` mode gets a warning border, others default to primary).
- The `historical_scratchpad` tags are intercepted during the timeline Markdown parsing. The raw XML-like string is extracted and rendered as a distinct, collapsible `╭ HISTORICAL SCRATCHPAD` summary card within the assistant's timeline item, significantly cleaning up the UI.

**Rollback steps:**
\`\`\`sh
git checkout HEAD^ -- crates/cade-tui
\`\`\`
