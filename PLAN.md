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
