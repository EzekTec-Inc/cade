# CADE Implementation Plan

> **Anti-amnesia design:** Every work item is self-contained. Each one lists the
> exact files, current state, what to do, how to verify, and a memory anchor tag.
> On session start, the agent reads this file + searches archival memory for the
> anchor tag to recover full context — no conversation history needed.
>
> **Memory protocol:** After completing each work item:
> 1. Update `active_goal` memory block with the completed item and next item.
> 2. Append a `## Completed` entry at the bottom of this file with the commit hash.
> 3. Run the full test suite (`cargo test --workspace`) before committing.
>
> **Session cold-start checklist:**
> 1. `cade-rag__index_workspace` + `start_watcher`
> 2. `read_file("IMPLEMENTATION_PLAN.md")` — find the next unchecked `[ ]` item
> 3. `archival_memory_search("<anchor tag>")` — pull in detailed context
> 4. `conversation_search("<last work item name>")` — recover any in-flight state
> 5. Proceed with the next `[ ]` item.

---

## Status Summary

| # | Work Item | Status | Est. Size |
|---|-----------|--------|-----------|
| WI-1 | TUI Refactor Phase 2: EditorComponent | ✅ Done (already shipped) | — |
| WI-2 | TUI Refactor Phase 3: Overlay Stack Migration | 🟡 Partial | Medium |
| WI-3 | TUI Refactor Phase 4: Slot Rendering + Input | 🟡 Partial | Small |
| WI-4 | Askpass Integration | ✅ Done (already shipped) | — |
| WI-5 | MCP Prefix Stripping | ✅ Done (already shipped) | — |
| WI-6 | Semantic Memory Search (P2) | ❌ Not started | Large |
| WI-7 | System Prompt Optimization | 🟡 Analysis done | Small |
| WI-8 | Unused Import Cleanup | ❌ Trivial | Trivial |

**Execution order:** WI-8 → WI-2 → WI-3 → WI-7 → WI-6

Rationale: WI-8 is a 10-second fix. WI-2 must precede WI-3 (slots depend on
clean overlay dispatch). WI-7 is a small prompt edit. WI-6 is a standalone
feature that can be deferred.

---

## WI-8: Unused Import Cleanup

**Memory anchor:** `ANCHOR_WI8_UNUSED_IMPORT`

**Status:** Not started

**Problem:** Build warning: `unused import: unicode_width::UnicodeWidthStr` in
`crates/cade-tui/src/app/mod.rs` line 41.

**Files to modify:**
- `crates/cade-tui/src/app/mod.rs` — remove line 41

**Steps:**
1. [ ] Remove `use unicode_width::UnicodeWidthStr;` from line 41
2. [ ] `cargo check -p cade-tui` — confirm warning gone, no errors
3. [ ] Commit: `chore(tui): remove unused unicode_width import`

**Verification:** `cargo check -p cade-tui 2>&1 | grep warning` returns nothing.

**Rollback:** `git revert HEAD`

---

## WI-2: TUI Overlay Stack Migration

**Memory anchor:** `ANCHOR_WI2_OVERLAY_MIGRATION`

**Status:** Trait exists, Vec field exists, 3 overlays implement trait, but all 5
overlays still dispatched via legacy `Option<...>` fields.

### Current State (committed in tree)

**Trait:** `crates/cade-tui/src/overlay_component.rs`
- `OverlayComponent` trait with: `render_overlay`, `handle_input`, `is_dismissed`, `take_result`
- `OverlayInputResult` enum: `Consumed`, `Dismissed`, `Unhandled`

**TuiApp fields (crates/cade-tui/src/app/mod.rs):**
- `overlays: Vec<Box<dyn OverlayComponent>>` — line 737 (NEW, unused for legacy)
- `active_question: Option<ActiveQuestionState>` — line 673 (LEGACY)
- `theme_picker: Option<ThemePickerState>` — line 726 (LEGACY, impls OverlayComponent)
- `command_palette: Option<CommandPaletteState>` — line 728 (LEGACY)
- `summary_overlay: Option<SummaryState>` — line 730 (LEGACY, impls OverlayComponent)
- `active_password: Option<PasswordPromptState>` — line 674 (LEGACY)

**Already impl OverlayComponent:** `PickerState`, `ThemePickerState`, `SummaryState`
**Not yet impl OverlayComponent:** `ActiveQuestionState`, `CommandPaletteState`, `PasswordPromptState`

**Dispatch paths (crates/cade-tui/src/app/input.rs):**
- Lines 57+: hardcoded `if self.active_question.is_some()` block
- Lines 159+: hardcoded `if self.summary_overlay.is_some()` block
- Lines 191+: hardcoded `if self.command_palette.is_some()` block
- Lines 235+: hardcoded `if self.theme_picker.is_some()` block
- Line 124: `if let Some(overlay) = self.overlays.last_mut()` — the NEW path (works but nothing pushed to it)

**Render path (crates/cade-tui/src/app/render.rs):**
- Individual overlay refs passed to `render_frame` as separate parameters
- No iteration over `self.overlays`

### Plan

Phase A: Make remaining overlays implement `OverlayComponent`:
1. [ ] Impl `OverlayComponent` for `CommandPaletteState`
2. [ ] Impl `OverlayComponent` for `ActiveQuestionState`
3. [ ] Impl `OverlayComponent` for `PasswordPromptState`
4. [ ] Tests: each overlay's `handle_input` returns correct `OverlayInputResult`

Phase B: Migrate dispatch from legacy fields to the Vec stack:
5. [ ] In `input.rs`: replace the 5 hardcoded `if self.X.is_some()` blocks with
       the single `overlays.last_mut()` dispatch at line 124
6. [ ] In render paths: iterate `self.overlays` bottom-to-top instead of
       passing individual overlay refs
7. [ ] Update all call sites that set overlays (e.g. `self.theme_picker = Some(...)`)
       to instead `self.overlays.push(Box::new(...))`
8. [ ] Remove the 5 legacy `Option<...>` fields from `TuiApp`

Phase C: Verify:
9. [ ] `cargo test -p cade-tui` — all tests pass
10. [ ] Manual test: file picker, theme picker, command palette, question modal, password modal
11. [ ] Commit: `refactor(tui): migrate overlays from legacy Option fields to dynamic stack`

**Verification:**
- `TuiApp` has zero `Option<...Picker/Question/Palette/Password...>` fields
- `grep -c "active_question\|theme_picker\|command_palette\|summary_overlay\|active_password" crates/cade-tui/src/app/mod.rs` returns 0
- All tests pass

**Rollback:** `git revert HEAD`

**Dependencies:** None (EditorComponent already done)

**Risk:** Medium — this touches the input dispatch hot path. Incorrect migration
could break overlay interactions. Checkpoint before starting.

---

## WI-3: Slot Rendering + Input Dispatch

**Memory anchor:** `ANCHOR_WI3_SLOT_WIRING`

**Status:** `SlotManager`, `UiSlot`, `SlotComponent` defined + tested in
`crates/cade-tui/src/slots.rs`. `TuiApp.slots` field exists. But `render.rs`
and `input.rs` never reference slots.

### Current State

**Defined (crates/cade-tui/src/slots.rs):**
- `UiSlot` enum: `Header`, `Footer`, `Sidebar` (hashable, map key)
- `SlotComponent` trait: `render`, `handle_input` (default: `InputResult::Ignored`)
- `SlotManager`: `HashMap<UiSlot, Box<dyn SlotComponent>>` with `set`, `take`, `get_mut`
- 7 tests (set/get/take/independent/displaces/defaults)

**TuiApp (crates/cade-tui/src/app/mod.rs):**
- `pub slots: SlotManager` — line 755
- Only reference: line 1193 uses `UiSlot` (likely a plan mode integration)
- `render.rs`: 0 references to `UiSlot` or `slots`
- `input.rs`: 0 references to `UiSlot` or `slots`

### Plan

1. [ ] In `render.rs` `render_frame()`: check `app.slots.get_mut(UiSlot::Header)` etc.
       and render into the layout if a widget is installed
2. [ ] Adjust layout constraints dynamically: if `Header` slot is occupied,
       allocate N rows for it; same for `Footer` and `Sidebar`
3. [ ] In `input.rs`: before editor dispatch, check if active slot wants the key
4. [ ] Add a test: install a mock `SlotComponent`, verify `render` is called
5. [ ] `cargo test -p cade-tui` — all pass
6. [ ] Commit: `feat(tui): wire SlotManager into render + input dispatch`

**Verification:**
- `grep -c "UiSlot\|slots\." crates/cade-tui/src/app/render.rs` > 0
- `grep -c "UiSlot\|slots\." crates/cade-tui/src/app/input.rs` > 0
- Tests pass

**Dependencies:** WI-2 (overlay migration) should be done first so the input
dispatch is clean before adding slot routing.

**Risk:** Low — additive changes only.

---

## WI-7: System Prompt Optimization

**Memory anchor:** `ANCHOR_WI7_PROMPT_OPT`

**Status:** Session 5 analyzed the system prompt and proposed 5 changes. No code
edits made.

### Current State

**File:** `src/bootstrap/prompt.rs` — 7830 bytes, `BASE_SYSTEM_PROMPT` constant
**Context assembly:** `crates/cade-server/src/server/api/messages/context.rs` →
`assemble_system_prompt_memory()`

### Proposed Changes (from session 5 analysis)

1. [ ] Add note that tool availability is dynamic (ITS prunes to ~15 tools/request)
       and agent should fall back to core tools if a specific one isn't available
2. [ ] Strengthen RAG-first language with token-cost framing ("semantic_search
       costs ~50 tokens, blind grep costs ~2000 tokens")
3. [ ] Clarify memory overflow strategy (what happens when blocks are archived,
       how to retrieve them)
4. [ ] Strengthen plan mode guidance (always set_plan for 2+ step tasks)
5. [ ] Add checkpoint guidance (create_checkpoint before risky operations)

### Plan

1. [ ] Read current `BASE_SYSTEM_PROMPT` in full
2. [ ] Edit `src/bootstrap/prompt.rs` with all 5 changes
3. [ ] `cargo check --workspace` — no errors
4. [ ] `cargo test --workspace` — all pass
5. [ ] Commit: `docs(prompt): optimize system prompt for tool awareness and memory guidance`

**Verification:** `cargo test --workspace` passes. Manual review of prompt text.

**Dependencies:** None

**Risk:** Low — prompt text only, no logic changes. But subtle wording changes
can shift agent behavior, so review carefully.

---

## WI-6: Semantic Memory Search (P2)

**Memory anchor:** `ANCHOR_WI6_SEMANTIC_SEARCH`

**Status:** Not started. Largest remaining item. Requires design decisions.

### Problem

`search_memory` currently uses SQLite FTS5 (keyword matching). When the user
searches for "how did we fix the deadlock", FTS5 won't match memory blocks that
say "scoped the parking_lot::Mutex lock" because no keywords overlap. Semantic
search would find it via embedding similarity.

### Design Decisions Needed

1. **Embedding model:** Local (e.g. `fastembed-rs`, `ort` ONNX runtime) vs.
   API call (OpenAI embeddings, Gemini embeddings)?
   - Local: no network dependency, ~100ms/query, adds ~50MB binary size
   - API: requires provider key, adds latency, but no binary bloat

2. **Storage:** SQLite `vec0` virtual table (sqlite-vec) vs. separate vector DB
   vs. brute-force in-memory cosine similarity over all blocks?
   - CADE already depends on SQLite heavily; `sqlite-vec` is the natural fit
   - Memory blocks are small (typically <100 per agent), so brute-force is viable

3. **Scope:** Memory blocks only? Or also conversation messages + archival memory?

### Proposed Architecture

```
cade-store/src/sqlite/memory.rs:
  pub fn search_memory_semantic(db, agent_id, query, limit) -> Vec<MemoryBlock>
    1. Embed the query string → Vec<f32>
    2. Load all memory block embeddings for agent_id
    3. Cosine similarity sort → top N
    4. Return blocks above threshold

cade-store/src/sqlite/tools.rs:
  Update search_memory() to merge FTS5 + semantic results (RRF or interleave)
```

### Plan (high-level — will be refined when started)

1. [ ] Design decision: choose embedding approach (local vs. API)
2. [ ] Add embedding dependency to `cade-store/Cargo.toml`
3. [ ] Add `embedding: Option<Vec<u8>>` column to memory_blocks table (migration 8)
4. [ ] Implement `embed_text(text) -> Vec<f32>` function
5. [ ] On `upsert_memory_block`, compute and store embedding
6. [ ] Implement `search_memory_semantic(db, agent_id, query, limit)`
7. [ ] Merge semantic results into existing `search_memory()` (hybrid ranking)
8. [ ] Tests: keyword search still works, semantic search finds conceptual matches
9. [ ] `cargo test --workspace` — all pass
10. [ ] Commit: `feat(memory): add semantic search via embeddings (P2)`

**Verification:**
- Test: searching "how we fixed the deadlock" finds a block mentioning "scoped mutex lock"
- All existing `search_memory` tests still pass
- No regression in `cargo test --workspace`

**Dependencies:** None (standalone feature)

**Risk:** High — new dependency, binary size impact, migration complexity. Should
be the last item implemented.

---

## Long-Term (not planned for immediate execution)

These are documented in `docs/roadmap.md` but not actionable yet:

- **Team features** — shared agents, memory, skills across a team
- **Voice mode** — speech-to-text input + audio output
- **Mobile/responsive dashboard** — `cade-gui` adapts to smaller viewports

---

## Completed Log

_Append entries here as work items are finished._

| Date | WI | Commit | Notes |
|------|----|--------|-------|
| — | — | — | — |
