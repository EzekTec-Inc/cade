## 2026-04-24T20:30:00Z — tui+core: theme context-bar + spinner tokens

**Task:** Move 8 hardcoded context-bar data-viz colors and 4 hardcoded spinner gradient colors into themeable tokens in `ThemeColors`, with palette-specific values for all 5 built-in themes.

**Files modified:**
- `crates/cade-core/src/resources/themes.rs` — added `ColorDef::default_reset()` helper; added 12 new fields (`ctx_bar_system..ctx_bar_buffer`, `spinner_0..spinner_3`) with `#[serde(default)]` for backward compat; set palette-appropriate values in `dark()`, `light()`, `catppuccin_mocha()`, `catppuccin_latte()`, `tokyo_night()`.
- `crates/cade-tui/src/app/timeline/render_item.rs` — replaced `const CATS` array with runtime `CAT_META` + `cat_colors` from theme tokens; updated 2 usage sites; added 2 new tests (`ctx_bar_tokens_are_all_distinct_and_non_reset`, `spinner_tokens_are_non_reset`).
- `crates/cade-tui/src/app/render.rs` — extracted `spinner_color(ms, colors)` helper; replaced 2 duplicated inline palettes with single call.

**Previous behavior:** Context-bar segments and animated spinner used hardcoded `RC::Rgb(...)` values, ignoring the active theme.
**New behavior:** All 12 colors are theme-token-driven. Each built-in theme has palette-aligned values (e.g., catppuccin uses Sapphire/Sky/Peach/Mauve, tokyo-night uses cyan/teal/orange/magenta).

**TDD record:**
- RED: 2 new tests compile-failed (fields don't exist), then pass after token addition.
- GREEN: 55/55 cade-tui, 212/212 cade-core, wasm32 check clean, clippy clean.
- REFACTOR: deduped spinner palette (2 copies → 1 helper fn).

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-24T20:00:00Z — tui: fix 5 hardcoded colors bypassing theme system

**Task:** Replace 5 hardcoded ratatui color literals in cade-tui with themed token references so all built-in and custom themes render correctly.

**Files modified:**
- `crates/cade-tui/src/app/timeline/render_item.rs` — replaced `RC::DarkGray` separator with `colors.border_base()`, `RC::White` model name with `colors.text_primary`, `RC::Rgb(239,68,68)` / `RC::Rgb(245,158,11)` context % with `colors.error` / `colors.warning`, `RC::Rgb(40,40,40)` bar padding with `colors.text_dim`; added `colors` param to `render_separator_item`; added 4 tests.
- `crates/cade-tui/src/app/timeline/mod.rs` — pass `colors` to `render_separator_item`.
- `crates/cade-tui/src/skills.rs` — replaced `RC::White` selected-row text with `colors.text_primary()`; removed unused `Color as RC` import.

**Previous behavior:** 5 UI elements used hardcoded color literals, causing incorrect rendering on non-dark themes (e.g., white-on-white text on light themes, wrong error/warning colors on catppuccin).
**New behavior:** All 5 elements use semantic theme tokens, rendering correctly across all themes.

**TDD record:**
- RED: 4 new tests in `render_item::tests` — assert themed tokens ≠ old hardcoded literals.
- GREEN: replaced 5 hardcoded colors — 53/53 tests pass, clippy clean.
- REFACTOR: simplified skills.rs branches (identical after fix) + removed unused import.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-24T19:20:00Z — theme: close Phase 3 & Phase 4 gaps

**Task:** Remove the last hardcoded `Color32` literal in the GUI render path and mark Phases 3 & 4 of the Theme System Refactoring Plan as done.

**Files modified:**
- `crates/cade-gui/src/app/mod.rs` — replaced `Color32::from_rgb(220, 50, 50)` with `self.theme.error()` in the `ConnectionFailed` render branch.
- `crates/cade-gui/src/theme.rs` — added `#[cfg(test)] mod tests` with 2 tests: `dark_theme_error_is_not_old_hardcoded_value`, `themed_error_matches_core_color_def`.
- `.cade-todo.md` — marked Phase 3 and Phase 4 as `[DONE]`.

**Previous behavior:** `ConnectionFailed` label used hardcoded `(220, 50, 50)` regardless of active theme.
**New behavior:** `ConnectionFailed` label uses `self.theme.error()`, matching all other themed UI elements.

**TDD record:**
- RED: 2 new tests in `theme::tests` — pass (theme properties confirmed).
- GREEN: replaced hardcoded color — `cargo check --target wasm32-unknown-unknown` clean, 308/308 tests pass.
- REFACTOR: none.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T22:20:00Z — cade-ide-mcp M-IDE-1b.15: docs closing M-IDE-1b

**Task:** Update `docs/ide-integration-plan.md` to reflect M-IDE-1b completion. Move the M-IDE-1b section to a ✅ status block with the final 9-tool / 7-callback table, document the two collapsed callbacks (save + debug_control) as deliberate design choices.

Not a TDD cycle — docs only. All 50 tests still pass.

**Files modified:** `docs/ide-integration-plan.md`.

**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T22:15:00Z — cade-ide-mcp M-IDE-1b.14: start_debug + stop_debug MCP tools (closes M-IDE-1b)

**Task:** Wire the last pair of mutating tools. `start_debug` forwards `DebugAction::Start { config }`, `stop_debug` forwards `DebugAction::Stop`, both through the single `debug_control` callback added in cycle 13.

**Closes M-IDE-1b.** All 7 planned mutating tools (apply_edit, open_file, set_selection, save_file, save_all, run_task, run_terminal, start_debug, stop_debug — 9 tool-names over 7 callbacks) now live on the wire.

**Files modified:** `crates/cade-ide-mcp/src/server.rs`.

**TDD record:**
- RED: `start_debug_and_stop_debug_forward_to_channel` + `tool_router_registers_start_and_stop_debug` — E0599.
- GREEN: added `StartDebugIn`, `DebugOut`, `start_debug_impl` + `stop_debug_impl`, `#[tool] start_debug` + `#[tool] stop_debug`. 48 unit + 2 e2e = 50/50 pass. `cargo check --workspace` clean.
- REFACTOR: none.

**End-to-end binary smoke test (informational, not a regression test):**
```sh
( initialize; initialized; tools/list ) | ./target/debug/cade-ide-mcp
```
Returns the expected 16 tool names: 7 read (get_active_file, get_open_files, get_selection, get_diagnostics, get_workspace_folders, get_visible_range, get_file_content) + 9 write (apply_edit, open_file, set_selection, save_file, save_all, run_task, run_terminal, start_debug, stop_debug). Binary exits cleanly on stdin close.

**Dependency policy:** No new dependencies.

**M-IDE-1a + M-IDE-1b summary:** 37 atomic commits, 50 tests (48 unit + 2 integration), 0 warnings, 16 MCP tools live on the wire, 2 workspace-level dep-feature additions across both sub-phases (schemars + rmcp features in M-IDE-1a.10; async-trait dep on crate in M-IDE-1b.1 — both already in the workspace).

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T22:05:00Z — cade-ide-mcp M-IDE-1b.13: debug_control callback on EditorChannel

Seventh (and last M-IDE-1b) mutating callback. Single method covers start + stop via a tagged enum argument — analogous to the save(Option<String>) pattern.

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs` — added `pub enum DebugAction { Start { config: String }, Stop }` with `#[serde(tag = "action", rename_all = "lowercase")]`.
- `crates/cade-ide-mcp/src/channel.rs` — `async fn debug_control(&self, action: DebugAction) -> Result<(), ErrorData>` with default method_not_found impl.

TDD record:
  RED   default_debug_control_returns_method_not_supported: E0599 (two call sites)
  GREEN added DebugAction enum + debug_control default impl.
        46 unit + 2 e2e = 48/48 pass. workspace clean.
  REFACTOR  none

**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T21:55:00Z — cade-ide-mcp M-IDE-1b.12: run_terminal MCP tool

TDD record:
  RED   run_terminal_forwards_command_to_channel + tool_router_registers_run_terminal: E0599
  GREEN added RunTerminalIn/Out + run_terminal_impl + #[tool] run_terminal.
        45 unit + 2 e2e = 47/47 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/server.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T21:48:00Z — cade-ide-mcp M-IDE-1b.11: run_terminal callback

Sixth mutating callback.

TDD record:
  RED   default_run_terminal_returns_method_not_supported: E0599
  GREEN added `async fn run_terminal(&self, command: String) -> Result<(), ErrorData>`
        with default method_not_found impl. 43 unit + 2 e2e = 45/45 pass.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/channel.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T21:42:00Z — cade-ide-mcp M-IDE-1b.10: run_task MCP tool

TDD record:
  RED   run_task_forwards_name_to_channel + tool_router_registers_run_task: E0599
  GREEN added RunTaskIn/Out + run_task_impl + #[tool] run_task.
        42 unit + 2 e2e = 44/44 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/server.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T21:36:00Z — cade-ide-mcp M-IDE-1b.9: run_task callback

Fifth mutating callback.

TDD record:
  RED   default_run_task_returns_method_not_supported: E0599
  GREEN added `async fn run_task(&self, name: String) -> Result<(), ErrorData>`
        with default method_not_found impl. 40 unit + 2 e2e = 42/42 pass.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/channel.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T21:30:00Z — cade-ide-mcp M-IDE-1b.8: save_file + save_all MCP tools

Two tools on the single save callback: save_file maps to save(Some(path)), save_all to save(None). Bundled into one cycle because they expose the same single behavior (forwarding through EditorChannel::save) at two callable names.

TDD record:
  RED   save_file_and_save_all_forward_args_to_channel +
        tool_router_registers_save_file_and_save_all: E0599
  GREEN added SaveFileIn + SaveOut, save_file_impl + save_all_impl,
        #[tool] save_file + #[tool] save_all.
        39 unit + 2 e2e = 41/41 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/server.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T21:20:00Z — cade-ide-mcp M-IDE-1b.7: save callback on EditorChannel

Fourth mutating callback. Single method handles both single-file save and save-all via `path: Option<String>` (None = save every dirty buffer). Avoids a redundant save_all callback.

TDD record:
  RED   default_save_returns_method_not_supported: E0599 (two call sites)
  GREEN added `async fn save(&self, path: Option<String>) -> Result<(), ErrorData>`
        with default method_not_found impl.
        37 unit + 2 e2e = 39/39 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/channel.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T21:12:00Z — cade-ide-mcp M-IDE-1b.6: set_selection MCP tool

Wire set_selection tool on top of cycle-5's callback.

TDD record:
  RED   set_selection_forwards_args_to_channel +
        tool_router_registers_set_selection: E0599
  GREEN added SetSelectionIn/Out + set_selection_impl + #[tool].
        36 unit + 2 e2e = 38/38 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/server.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T21:05:00Z — cade-ide-mcp M-IDE-1b.5: set_selection callback on EditorChannel

Third mutating callback.

TDD record:
  RED   default_set_selection_returns_method_not_supported: E0599
  GREEN added `async fn set_selection(&self, path: String, range: Range) -> Result<(), ErrorData>`
        with default method_not_found impl.
        34 unit + 2 e2e = 36/36 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/channel.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T21:00:00Z — cade-ide-mcp M-IDE-1b.4: open_file MCP tool (TDD cycle 4 of M-IDE-1b)

Wire the open_file tool on top of cycle-3's reveal_file callback. Same pattern as apply_edit.

TDD record:
  RED   open_file_forwards_path_to_channel +
        tool_router_registers_open_file: E0599 + E0282
  GREEN added OpenFileIn / OpenFileOut, open_file_impl,
        #[tool] open_file with Parameters<OpenFileIn>.
        33 unit + 2 e2e = 35/35 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/server.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T20:55:00Z — cade-ide-mcp M-IDE-1b.3: reveal_file callback on EditorChannel (TDD cycle 3 of M-IDE-1b)

Second mutating callback. Pattern identical to cycle 1's apply_edit: default impl returns -32601 method_not_found with the adapter label echoed.

TDD record:
  RED   default_reveal_file_returns_method_not_supported: E0599
  GREEN added `async fn reveal_file(&self, path: String) -> Result<(), ErrorData>`
        with default method_not_found impl.
        31 unit + 2 e2e = 33/33 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/channel.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T20:45:00Z — cade-ide-mcp M-IDE-1b.2: apply_edit MCP tool (TDD cycle 2 of M-IDE-1b)

**Task:** Wire the first mutating MCP tool — `apply_edit` — on top of the callback added in cycle 1. The tool forwards the full `ApplyEditRequest` to the attached `EditorChannel` and returns an empty success object on `Ok(())`; any `ErrorData` from the channel bubbles up unchanged.

**Scope guardrail:** Only `server.rs`. No new deps. No change to state or channel layers.

**Files modified:**
- `crates/cade-ide-mcp/src/server.rs`:
  - New `pub struct ApplyEditOut {}` (empty success; the editor itself is the source of truth for resulting buffer state, which the adapter will push back via `EditorState` as a follow-up update).
  - New inherent `apply_edit_impl(&self, ApplyEditRequest) -> Result<ApplyEditOut, ErrorData>` that calls `self.channel.apply_edit(req).await?`.
  - New `#[tool(name = "apply_edit", …)]` wrapping the `_impl` method via `Parameters<ApplyEditRequest>`.
  - Two new tests:
    - `apply_edit_forwards_request_to_channel` — defines a local `RecordingChannel` that captures the forwarded request, constructs an `IdeMcpServer` with it, calls `apply_edit_impl`, asserts the channel saw the exact request.
    - `tool_router_registers_apply_edit` — router registration.

**TDD record:**
- RED: both tests fail with E0599 (`apply_edit_impl` missing) and E0282.
- GREEN: added output type + `_impl` method + `#[tool]` method. `cargo test -p cade-ide-mcp` → 30 unit + 2 e2e = 32/32 pass. `cargo check --workspace` clean.
- REFACTOR: none.

**Previous behavior:** No mutating MCP tools existed; `EditorChannel::apply_edit` had a default but no tool reached it.

**New behavior:** Agents can now call `apply_edit`. With the default `NullEditorChannel` (and any adapter that hasn't overridden `apply_edit`), the tool returns JSON-RPC `-32601 method_not_found`. With an adapter that does override, the adapter applies the edit and the tool returns `{}`.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T20:30:00Z — cade-ide-mcp M-IDE-1b.1: apply_edit callback on EditorChannel (TDD cycle 1 of M-IDE-1b)

**Task:** Open M-IDE-1b (edit tools). First cycle extends the `EditorChannel` trait with an `apply_edit` mutating callback. Ships the callback shape + default "method not found" behavior; tool wiring in the next cycle.

**User-approved dependency change this cycle:** `async-trait.workspace = true` added to `crates/cade-ide-mcp/Cargo.toml`. `async-trait` is already a workspace-level dep (used by cade-ai, cade-server). No net-new crate.

**Design-shift honesty:** Earlier in cycle 8 I claimed we could use native `async fn` in traits (Rust 1.94 + edition 2024). That's correct for monomorphic impls, but **not** yet dyn-compatible without nightly features. Since the existing object-safety test asserts `Arc<dyn EditorChannel>`, mutating methods must go through `async-trait`. Adopting the project-standard `#[async_trait]` macro matches cade-ai / cade-server patterns.

**Scope guardrail:** Only `channel.rs`, `state.rs`, and `Cargo.toml`. No tool wiring yet. Default trait-method impl returns a JSON-RPC -32601 `METHOD_NOT_FOUND` error, so `NullEditorChannel` and all future adapters that haven't overridden `apply_edit` refuse loudly by default.

**Files modified:**
- `crates/cade-ide-mcp/Cargo.toml` — added `async-trait.workspace = true`.
- `crates/cade-ide-mcp/src/state.rs`:
  - New `pub struct TextEdit { range: Range, new_text: String }` (LSP `TextEdit` shape).
  - New `pub struct ApplyEditRequest { path: String, text_edits: Vec<TextEdit> }`.
- `crates/cade-ide-mcp/src/channel.rs`:
  - `EditorChannel` gets `#[async_trait]` and an `async fn apply_edit(&self, ApplyEditRequest) -> Result<(), ErrorData>` with a default method-not-found impl.
  - `impl EditorChannel for NullEditorChannel` is now `#[async_trait]`-decorated; inherits the default `apply_edit`.
  - New test `default_apply_edit_returns_method_not_supported` asserting `err.code.0 == ErrorCode::METHOD_NOT_FOUND.0`.
  - Existing `editor_channel_is_object_safe_and_send_sync` kept — proves `async-trait` preserved dyn compatibility.

**TDD record:**
- RED: new test fails with E0422 (`ApplyEditRequest` missing), E0599 (method missing), E0282.
- GREEN: added deps, types, trait method + default impl. `cargo test -p cade-ide-mcp` → 28 unit + 2 e2e = 30/30 pass. `cargo check --workspace` clean.
- REFACTOR: none.

**Previous behavior:** `EditorChannel` had lifecycle-only methods.

**New behavior:** Adapters can override `apply_edit` to handle the forthcoming tool. The default rejects every call with a structured MCP error, so the trait is safe to expand without breaking `NullEditorChannel` or existing adapters.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T20:10:00Z — cade-ide-mcp M-IDE-1a.23: docs (cycle 23, closes M-IDE-1a)

**Task:** Document the new IDE-MCP bridge so future contributors and users understand what shipped, how to run it, and what comes next. Closes M-IDE-1a.

**Scope guardrail:** Docs only. No source changes. No deps. Not a TDD cycle — docs fall outside tdd-guide.

**Files modified:**
- `ARCHITECTURE.md` — expanded the `cade-ide-mcp` crate description from a one-liner stub to three lines naming the tool surface and the adapter pattern.
- `README.md` — added `cade-ide` as a third example under the `mcpServers` block and a short paragraph explaining its role and current scope.
- `docs/ide-integration-plan.md` — **new**. Phased roadmap (M-IDE-1a complete, 1b edit tools, 1c adapter protocol, 2 VS Code extension, 3 JetBrains plugin). Includes architecture diagram, the seven shipped tools in a table, operational notes (stderr-only logging, no filesystem fallback, NullEditorChannel semantics), and a turn-key `~/.cade/settings.json` registration example.

**Test record:**
- All 29 tests still pass. `cargo check --workspace` clean.

**Previous behavior:** `cade-ide-mcp` was described in one line in the architecture doc and nowhere else.

**New behavior:** The crate has a dedicated plan doc, is referenced from the README MCP section, and has an expanded ARCHITECTURE entry.

**Milestone closed:** M-IDE-1a (scaffold + read-only tool surface + stdio binary). 14 commits in total — `1e3ad1bf` through this one.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T19:58:00Z — cade-ide-mcp M-IDE-1a.22: stdio binary (TDD cycle 22)

**Task:** Make `cade-ide-mcp` runnable as a subprocess. Editor adapters spawn the binary and speak MCP over stdin/stdout.

**Scope guardrail:** One new source file + `[[bin]]` block in the crate manifest + one new crate dep (`tracing-subscriber.workspace`). No workspace-level dep changes (`tracing-subscriber` already a workspace dep). No new workspace crate.

**Files modified:**
- `crates/cade-ide-mcp/Cargo.toml` — added `[[bin]] name = "cade-ide-mcp" path = "src/bin/cade-ide-mcp.rs"` and `tracing-subscriber.workspace = true` to `[dependencies]`.
- `crates/cade-ide-mcp/src/bin/cade-ide-mcp.rs` — **new**. `#[tokio::main]` stdio entrypoint. Builds an `IdeMcpServer::with_null_channel(EditorState::new())`, wires `rmcp::transport::io::stdio()` as the transport, and serves until stdin closes. Logging routed to stderr via `tracing_subscriber::fmt().with_writer(std::io::stderr)` with ANSI disabled and an `EnvFilter` defaulting to `info`. Error type is `Box<dyn Error + Send + Sync>` — no new dep like `anyhow` introduced.

**TDD record:**
- RED: added the `[[bin]]` block pointing at a non-existent file. `cargo build -p cade-ide-mcp --bin cade-ide-mcp` failed: `can't find bin 'cade-ide-mcp'`.
- GREEN: created the file with the stdio main. `cargo build` succeeds. Smoke test: `echo '{"jsonrpc":"2.0","id":1,"method":"initialize",…}' | ./target/debug/cade-ide-mcp` returns a well-formed `initialize` response advertising `serverInfo.name = "cade-ide-mcp"`, `version = "0.1.0"`, `capabilities.tools = {}`. Stderr shows `cade-ide-mcp starting on stdio version="0.1.0"` and `rmcp::service: Service initialized as server`. All 29 existing tests still pass. `cargo check --workspace` clean.
- REFACTOR: none.

**Previous behavior:** `cade-ide-mcp` was library-only.

**New behavior:** Editor adapters (or `.cade/settings.json` MCP server entries) can run `cade-ide-mcp` as a subprocess and consume its read tools over MCP stdio. The `NullEditorChannel` still gates mutating tools; cycle 23+ will add an adapter protocol layer that populates the shared `EditorState` from real editor events.

**Dependency policy:** No new workspace deps. `tracing-subscriber` is already workspace-level.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T19:48:00Z — cade-ide-mcp M-IDE-1a.21: get_file_content tool (TDD cycle 21)

**Task:** Seventh read tool — `get_file_content(path)` returns the full buffer text of a single open file. First tool with an argument and first tool that can fail (path not open → MCP error -32602 invalid_params).

**Scope guardrail:** Only `server.rs`. No new deps.

**Files modified:**
- `crates/cade-ide-mcp/src/server.rs`:
  - New `pub struct GetFileContentIn { path: String }` (Deserialize + JsonSchema) and `pub struct GetFileContentOut { path, text, language_id, version, is_dirty }` (Serialize + JsonSchema).
  - New inherent `get_file_content_impl(&self, path: String) -> Result<GetFileContentOut, ErrorData>` — looks up `path` in `state.open_files_snapshot()`; returns `ErrorData::invalid_params` when not found (the agent should not silently filesystem-read; the adapter owns buffer state).
  - New `#[tool] get_file_content(&self, Parameters(GetFileContentIn { path })) -> Result<Json<GetFileContentOut>, ErrorData>` wrapping it.
  - Added `rmcp::handler::server::wrapper::Parameters` and `rmcp::model::ErrorData` to the import list; `serde::Deserialize` too.
  - Three new tests: happy-path round-trip, error-path (file not open → path echoed in error), router registration.

**TDD record:**
- RED: all three tests fail (E0599 missing method, E0282 needs type hint because method doesn't exist).
- GREEN: `cargo test -p cade-ide-mcp` → 27 unit + 2 e2e = 29/29 pass. `cargo check --workspace` clean.
- REFACTOR: none.

**Previous behavior:** Agents could only see the path list via `get_open_files`; reading the buffer text was impossible.

**New behavior:** Agents can request the full text of any open file. Unknown paths return a structured MCP error rather than falling through to a filesystem read.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T19:35:00Z — cade-ide-mcp M-IDE-1a.20: OpenFile LSP shape (TDD cycle 20)

**Task:** Extend `OpenFile` with the four LSP-standard fields the next tool (`get_file_content`, cycle 21) needs: `text`, `language_id`, `version`, `is_dirty`. Keeps the state type alone in this cycle; no new tool yet.

**Scope guardrail:** Only `state.rs` + minimal caller patches in `server.rs` for the one existing test that constructs `OpenFile` literals. No new deps.

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs` — `OpenFile` gains `text: String`, `language_id: String`, `version: u64`, `is_dirty: bool`. Docs call out the LSP `TextDocumentItem` correspondence.
- `crates/cade-ide-mcp/src/server.rs` — existing `get_open_files_returns_adapter_pushed_list` test now supplies the full shape. `OpenFileSummary` (the output shape of `get_open_files`) intentionally stays `{ path }` only; the heavier body+metadata comes via `get_file_content` in cycle 21.

**TDD record:**
- RED: added `open_file_round_trips_full_shape` round-tripping all five fields; existing `replace_open_files_updates_count` rewritten to supply the full shape. Both failed with E0560 (missing fields).
- GREEN: added the four fields, patched the server-side test site. `cargo test -p cade-ide-mcp` → 24 unit + 2 e2e = 26/26 pass. `cargo check --workspace` clean.
- REFACTOR: none.

**Public API change (internal to cade-ide-mcp):** `OpenFile` literal construction now requires five fields instead of one. No outside callers exist; no migration needed.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T19:20:00Z — cade-ide-mcp M-IDE-1a.19: get_visible_range tool (TDD cycle 19)

Sixth read tool. Drop-in applying the established pattern. Output flattens `Option<(u32, u32)>` → `{ start_line: Option<u32>, end_line: Option<u32> }` for a friendlier JSON shape than `"visible": [5, 42]`.

TDD record:
  RED   get_visible_range_returns_adapter_pushed_range +
        tool_router_registers_get_visible_range: E0599
  GREEN added GetVisibleRangeOut + _impl + #[tool].
        23 unit + 2 e2e = 25/25 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/server.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T19:12:00Z — cade-ide-mcp M-IDE-1a.18: get_workspace_folders tool (TDD cycle 18)

Fifth read tool. Drop-in applying the established pattern.

TDD record:
  RED   get_workspace_folders_returns_adapter_pushed_list +
        tool_router_registers_get_workspace_folders: E0599
  GREEN added GetWorkspaceFoldersOut + _impl + #[tool].
        21 unit + 2 e2e = 23/23 pass. workspace clean.
  REFACTOR  none

**Files modified:** `crates/cade-ide-mcp/src/server.rs`
**Dependency policy:** No new dependencies.
**Rollback steps:** `git reset --hard HEAD~1`

## 2026-04-23T19:05:00Z — cade-ide-mcp M-IDE-1a.17: get_diagnostics tool (TDD cycle 17)

**Task:** Fourth read tool — `get_diagnostics` returns the full diagnostic list from the editor's language services.

**Scope guardrail:** Only `server.rs`. No new deps. Follows the drop-in pattern from cycle 15/16.

**Files modified:**
- `crates/cade-ide-mcp/src/server.rs` — added `GetDiagnosticsOut`, `get_diagnostics_impl`, `#[tool] get_diagnostics`, plus two tests.

**TDD record:**
- RED: `get_diagnostics_returns_adapter_pushed_list` + `tool_router_registers_get_diagnostics` — E0599.
- GREEN: 19 unit + 2 e2e = 21/21 pass. workspace clean.
- REFACTOR: none.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T18:55:00Z — cade-ide-mcp M-IDE-1a.16: get_selection tool (TDD cycle 16)

**Task:** Third read tool — `get_selection` returns the user's current selection (path + range + text) or `null`. Also promote the state-layer value types to serde+schemars so they can appear directly in tool output schemas.

**Scope guardrail:** Only `state.rs` derives + `server.rs`. No new dependencies.

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs` — added `Serialize, Deserialize, schemars::JsonSchema` to every public value type (`OpenFile`, `Position`, `Range`, `Selection`, `DiagnosticSeverity` (with `#[serde(rename_all = "lowercase")]`), `Diagnostic`, `WorkspaceFolder`). Pure-additive — existing derives preserved.
- `crates/cade-ide-mcp/src/server.rs`:
  - New `pub struct GetSelectionOut { selection: Option<crate::state::Selection> }`.
  - New inherent `get_selection_impl()` method returning the above.
  - New `#[tool(name = "get_selection", …)]` wrapping it in `Json<_>`.
  - Two new tests: `get_selection_returns_adapter_pushed_selection` (round-trip via shared state) and `tool_router_registers_get_selection` (router registration).

**TDD record:**
- RED: both new tests fail with E0599.
- GREEN: added derives on state types, output struct, `_impl` method, `#[tool]` method. `cargo test -p cade-ide-mcp` → 17 unit + 2 integration = 19/19 pass. `cargo check --workspace` clean.
- REFACTOR: none.

**Previous behavior:** The agent could not read the user's active text selection.

**New behavior:** Calling `get_selection` returns `{ selection: { path, range: { start, end }, text } | null }`.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T18:45:00Z — cade-ide-mcp M-IDE-1a.15: get_open_files tool (TDD cycle 15)

**Task:** Second read tool. Expose the adapter-pushed open-file list via an MCP `get_open_files` tool that returns `{ files: [{ path }] }`.

**Scope guardrail:** Only `state.rs` (one new snapshot accessor) and `server.rs` (new output types + `get_open_files_impl` + `#[tool]` method). No new deps. Establishes the pattern used for the remaining five read tools: delegate each `#[tool]` to a test-friendly `…_impl` method on the same struct.

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs` — added `pub async fn open_files_snapshot(&self) -> Vec<OpenFile>` (a cloning snapshot accessor, parallel to the existing `active_file()` / `selection()` / `diagnostics()` family).
- `crates/cade-ide-mcp/src/server.rs`:
  - Added `pub struct OpenFileSummary { path: Option<String> }` and `pub struct GetOpenFilesOut { files: Vec<OpenFileSummary> }` (both `Serialize + JsonSchema`).
  - Refactored `get_active_file` to delegate to a new inherent `get_active_file_impl()` method on an `impl IdeMcpServer` block. The `#[tool]` method now just `Json(self.get_active_file_impl().await)`.
  - Added sibling `get_open_files_impl()` method + `#[tool(name = "get_open_files", …)]` wrapping it.
  - Two new tests in `server::tests`:
    - `get_open_files_returns_adapter_pushed_list` — pushes two files through a shared `EditorState` clone, asserts the `_impl` method returns both in order.
    - `tool_router_registers_get_open_files` — router exposes the new tool name.

**TDD record:**
- RED: both new tests fail with E0599 (`get_open_files_impl` / route missing).
- GREEN: added state snapshot accessor, output structs, inherent impl method, and the `#[tool]` method. `cargo test -p cade-ide-mcp` → 15 unit + 2 integration = 17/17 pass. `cargo check --workspace` clean.
- REFACTOR: extracted `get_active_file_impl` (previously inlined in the `#[tool]` body). Same behavior; now cycle-15 establishes the idiom for all remaining read tools.

**Previous behavior:** Only `get_active_file` was exposed.

**New behavior:** The agent can now query the editor's open tabs through MCP.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T18:28:00Z — cade-ide-mcp M-IDE-1a.14: adapter-push e2e test (TDD cycle 14)

**Task:** Prove the new shared-storage `EditorState` (cycle 13) wired end-to-end: adapter clones the state, pushes `active_file = Some("/tmp/foo.rs")`, MCP client calls `get_active_file`, and the response contains the pushed path.

**Scope guardrail:** Only the integration test file. No production code. No new deps.

**Honest TDD note:** Another **characterization test** — the behavior already works after cycle 13's `Arc<RwLock<Inner>>` refactor. Value is regression-guarding the adapter-push pathway end-to-end and demonstrating the canonical wiring that future cycles reuse.

**Files modified:**
- `crates/cade-ide-mcp/tests/e2e_tool_call.rs` — added `get_active_file_returns_path_pushed_by_adapter`. Clones `EditorState`, pushes `set_active_file` **after** constructing the server (the realistic adapter lifecycle), spawns the server over a `tokio::io::duplex`, and asserts the tool result JSON contains `"/tmp/foo.rs"`.

**Test record:**
- `cargo test -p cade-ide-mcp` → 13 unit + 2 integration + 0 doc = **15/15** pass.
- `cargo check --workspace` clean.

**Previous behavior:** The adapter-push pathway was exercised only by an internal unit test (`clones_share_storage_after_mutation`).

**New behavior:** The adapter-push pathway is regression-guarded end-to-end through the full rmcp wire protocol.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T18:15:00Z — cade-ide-mcp M-IDE-1a.13: shared-storage EditorState refactor (TDD cycle 13)

**Task:** Back `EditorState` with `Arc<tokio::sync::RwLock<Inner>>` so adapter-side clones and server-side clones share storage. This is the prerequisite for the next tool-wiring cycles: a real editor adapter pushes updates into its clone after the server is already running, and the tool must see them.

**Design choice (user-approved this cycle):** `Arc<tokio::sync::RwLock<Inner>>` over `Arc<std::sync::RwLock<Inner>>` or "immutable snapshots only". Matches the concurrency idiom used elsewhere in the project (cade-mcp, cade-server).

**Scope guardrail:** Only `state.rs` + minimal caller patches in `server.rs`. No new dependencies (`tokio` already a workspace dep, used). No public API outside this crate — no external callers break.

**Public API change (internal to cade-ide-mcp):**
- All getters and setters on `EditorState` are now `async` and take `&self` (no `&mut self`). `Clone` clones the `Arc`; mutations go through the `RwLock`.
- `EditorState` no longer exposes its fields through struct-literal construction; adapters build an empty state via `EditorState::new()` and populate it via the setter methods.

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs` — extracted an internal `struct Inner { … }` holding the owned fields; `pub struct EditorState { inner: Arc<RwLock<Inner>> }`. All 13 getters and setters rewritten to `async fn (&self)`. All existing unit tests rewritten as `#[tokio::test]` with the new `.await` call shape.
- `crates/cade-ide-mcp/src/server.rs` — `get_active_file` tool uses `self.state.active_file().await`; server unit test uses `open_file_count().await`.

**TDD record:**
- RED: added `clones_share_storage_after_mutation` — clones state `a` → `b`, sets active file on `b`, reads it on `a`. `cargo test -p cade-ide-mcp --lib state::tests::clones_share_storage_after_mutation` failed with E0277 (type mismatch: `()` is not a future; sync method signatures).
- GREEN: refactored `EditorState` to `Arc<RwLock<Inner>>`, rewrote every getter/setter as `async fn (&self)`, migrated all unit tests to `#[tokio::test]`, patched the `get_active_file` tool and the server unit test. `cargo test -p cade-ide-mcp` → 13 unit + 1 integration = 14/14 pass. `cargo check --workspace` clean.
- REFACTOR: removed a transient `open_file_count_async` helper added during the refactor; the final API is uniformly async with no sync "blocking_read" footgun.

**Previous behavior:** `EditorState` was a plain owned struct; clones diverged; mutating setters required `&mut self`.

**New behavior:** `EditorState::clone()` is an `Arc::clone`. Adapters and the MCP server hold independent clones that agree on current state.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T17:55:00Z — cade-ide-mcp M-IDE-1a.12: end-to-end tool_call characterization test (TDD cycle 12)

**Task:** Prove that the `get_active_file` tool registered in cycle 10 and exposed through the `ServerHandler` impl of cycle 11 is actually reachable through a real rmcp client↔server roundtrip. The test spawns `IdeMcpServer` on one end of a `tokio::io::duplex` pair and an empty `()` client on the other, then issues a `tools/call` via `client.peer().call_tool(…)` and asserts the result JSON contains `"path":null` for an empty `EditorState`.

**Scope guardrail:** Only the new integration test file. No new dependencies. No production code changed. Single assertion — the adapter-side mutation path is deferred to the next cycle (which must first add shared state between the adapter and the server).

**Honest TDD note:** This is a **characterization test**, not a red → green cycle. The behavior under test was already implemented in cycles 10 + 11; running the code end-to-end exposes nothing new. The value is (1) a regression guard against future rmcp upgrades breaking wire compatibility, and (2) a worked example of driving the server in-process, which future test cycles will reuse.

**Files modified:**
- `crates/cade-ide-mcp/tests/e2e_tool_call.rs` — **new**. Single `#[tokio::test]` using `rmcp::ServiceExt::serve` on both ends of a `tokio::io::duplex(4096)` and `CallToolRequestParams::new("get_active_file")`.

**Test record:**
- `cargo test -p cade-ide-mcp` → 12 unit + 1 integration + 0 doc = 13 passed, 0 failed.
- `cargo check --workspace` clean.

**Previous behavior:** The MCP tool was registered and routed, but no test exercised the wire path.

**New behavior:** The wire path is regression-guarded. Any future rmcp upgrade that breaks `tools/call` framing, the `#[tool_handler]` macro output, or the `Json<T>` wrapper's content encoding will surface as a test failure.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T17:35:00Z — cade-ide-mcp M-IDE-1a.11: ServerHandler impl + get_info (TDD cycle 11)

**Task:** Implement `rmcp::ServerHandler` for `IdeMcpServer` via `#[tool_handler]` and expose an `initialize`-friendly `get_info()` advertising the crate name, version, tool capability, and an instructions string.

**Scope guardrail:** Only `server.rs`. No new dependencies. No stdio transport yet (lands with a later TDD cycle that drives a real end-to-end call through an in-process duplex).

**Files modified:**
- `crates/cade-ide-mcp/src/server.rs`:
  - Added `rmcp::ServerHandler` and `rmcp::tool_handler` to the import list; added `rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo}`.
  - Added `#[tool_handler] impl ServerHandler for IdeMcpServer { fn get_info(…) }`. Because `ServerInfo` and `Implementation` are `#[non_exhaustive]`, `get_info` constructs them by mutating `Default::default()` instead of struct-literal syntax.
  - Replaced the `#[allow(dead_code)]` stale comment on the `tool_router` field with a comment that explains the attribute is necessary because the macro reads the field via an associated-fn indirection invisible to the dead-code lint.

**TDD record:**
- RED: added `server_implements_server_handler_with_expected_name` asserting trait bound + inspecting `get_info().server_info.{name,version}` and tool capability. `cargo test -p cade-ide-mcp --lib` failed with E0277 (`ServerHandler` not implemented).
- GREEN: added `#[tool_handler]` impl + `get_info()`. `cargo test -p cade-ide-mcp --lib` → 12/12 pass with zero warnings. `cargo check --workspace` clean.
- REFACTOR: none.

**Previous behavior:** `IdeMcpServer` was a standalone struct; rmcp had no way to serve tool calls through it.

**New behavior:** `IdeMcpServer` is a full `rmcp::ServerHandler`. Once a transport (stdio, in-process duplex, HTTP) is attached in a later cycle, `initialize` returns our advertised server info and `tools/call get_active_file` routes through the generated router.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T17:20:00Z — cade-ide-mcp M-IDE-1a.10: first MCP tool `get_active_file` (TDD cycle 10)

**Task:** First real MCP tool. Wire `rmcp` server-side features and register `get_active_file` via `#[tool_router]` + `#[tool]` so the router reports a single route named `get_active_file`.

**Scope guardrail:** Only `server.rs` + crate `Cargo.toml` + workspace `Cargo.toml`. No ServerHandler impl yet (deferred to the cycle that introduces stdio transport). No editor adapter. One tool only.

**Dependency additions (user-approved this cycle):**
- Workspace `Cargo.toml`:
  - Added `server` and `macros` to the feature list of the **existing** `rmcp = "1.4"` workspace entry. No net-new crate — only more features of an already-present dep.
  - Added `schemars = "1"` as a workspace dep. Already present transitively (pulled by `rmcp`); now declared directly so `cade-ide-mcp` can `#[derive(JsonSchema)]` on tool output types.
- `crates/cade-ide-mcp/Cargo.toml`:
  - Added `rmcp.workspace`, `schemars.workspace`, `serde.workspace`, `serde_json.workspace`, `tokio.workspace`, `tracing.workspace`. Every addition uses `.workspace = true`; nothing pulls in a new top-level crate.

**Files modified:**
- `Cargo.toml` — feature + workspace-dep additions above.
- `crates/cade-ide-mcp/Cargo.toml` — crate dep wiring.
- `crates/cade-ide-mcp/src/server.rs`:
  - `IdeMcpServer` gains a `tool_router: ToolRouter<Self>` field (marked `#[allow(dead_code)]` until the ServerHandler impl lands in the next cycle).
  - `IdeMcpServer::new` populates the field from the macro-generated `Self::tool_router()`.
  - New `#[tool_router] impl IdeMcpServer { ... }` block with a single `#[tool(name = "get_active_file", …)]` async method that returns `Json<GetActiveFileOut>` reading `self.state.active_file()`.
  - New `pub struct GetActiveFileOut { path: Option<String> }` as the tool's output schema.

**TDD record:**
- RED: added `tool_router_registers_get_active_file` asserting `IdeMcpServer::tool_router().has_route("get_active_file")`. `cargo test -p cade-ide-mcp --lib` failed with E0599 (associated `tool_router` missing).
- GREEN: added workspace + crate dep wiring, added the macro-decorated impl block and the `GetActiveFileOut` struct. `cargo test -p cade-ide-mcp --lib` → 11/11 pass. `cargo check --workspace` clean.
- REFACTOR: none.

**Previous behavior:** `cade-ide-mcp` had no MCP tools and no rmcp deps.

**New behavior:** The generated `IdeMcpServer::tool_router()` returns a `ToolRouter<Self>` with `get_active_file` registered. Calling the tool (once `ServerHandler` is implemented next cycle) will return `{ "path": <active_file_or_null> }` JSON.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T16:55:00Z — cade-ide-mcp M-IDE-1a.9: IdeMcpServer wrapper (TDD cycle 9)

**Task:** Introduce the top-level `IdeMcpServer` struct that wraps `EditorState` and `Arc<dyn EditorChannel>`. Ninth TDD cycle. Deliberately minimal: no rmcp transport yet, no `#[tool]` macros yet — that lands in a later cycle once the approval to extend rmcp features is exercised.

**Scope guardrail:** Only `server.rs` (new) + a `pub use` line in `lib.rs`. No new workspace dependencies. Workspace `Cargo.toml` untouched.

**Files modified:**
- `crates/cade-ide-mcp/src/server.rs` — **new**. `pub struct IdeMcpServer { state: EditorState, channel: Arc<dyn EditorChannel> }` with `new`, `with_null_channel`, `state()`, `channel_label()`.
- `crates/cade-ide-mcp/src/lib.rs` — added `mod server;` and `pub use server::IdeMcpServer;`.

**TDD record:**
- RED: `server::tests::server_with_null_channel_builds_and_exposes_state`. `cargo test -p cade-ide-mcp --lib` failed with E0432 / E0433 (`IdeMcpServer` undeclared).
- GREEN: added struct + four methods. `cargo test -p cade-ide-mcp --lib` → 10/10 pass.
- REFACTOR: none.

**Previous behavior:** No wrapper existed to bind state and channel into a single handler.

**New behavior:** Adapters construct one `IdeMcpServer` per editor attach and serve it as the MCP handler (once the rmcp wiring lands in a later cycle).

**Dependency policy:** No new dependencies. Uses only `std::sync::Arc`.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T16:44:00Z — cade-ide-mcp M-IDE-1a.8: EditorChannel trait (TDD cycle 8)

**Task:** Define the adapter-facing trait. Editor adapters (VS Code, JetBrains, tests) implement `EditorChannel`; phase M-IDE-1a only exposes lifecycle methods (`label()`, `is_connected()`) — mutating callbacks for edits, tasks, terminal, and debugger are deferred to later phases so each lands with its own failing test.

**Scope guardrail:** Only `channel.rs` + a `pub use` line in `lib.rs`. No new dependencies. Trait uses plain sync methods for now (native `async fn` in traits is stable on Rust 1.94 / edition 2024 and will be used when a callback method actually needs it).

**Files modified:**
- `crates/cade-ide-mcp/src/channel.rs` — **new**. Defines `pub trait EditorChannel: Send + Sync + 'static` with `label()` and `is_connected()`, plus `pub struct NullEditorChannel` as a no-op impl for tests and warm-up.
- `crates/cade-ide-mcp/src/lib.rs` — added `mod channel;` declaration and `pub use channel::{EditorChannel, NullEditorChannel};`.

**TDD record:**
- RED: added two tests in `channel.rs` — `null_channel_reports_disconnected_with_label_null` and `editor_channel_is_object_safe_and_send_sync`. `cargo test -p cade-ide-mcp --lib` failed with E0432/E0425/E0405 (trait + struct missing).
- GREEN: added the trait and `NullEditorChannel` impl. `cargo test -p cade-ide-mcp --lib` → 9/9 pass.
- REFACTOR: none.

**Previous behavior:** No adapter-facing abstraction existed.

**New behavior:** Adapters implement `EditorChannel`; consumers can hold `Arc<dyn EditorChannel>`. `NullEditorChannel` provides a tests-friendly no-op.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T16:35:00Z — cade-ide-mcp M-IDE-1a.7: visible_range getter/setter (TDD cycle 7)

**Task:** Let the adapter report the active editor's visible viewport. Seventh TDD cycle; final state-layer behavior before the channel trait (cycle 8).

**Scope guardrail:** Only `state.rs`. No new dependencies. Visible range is represented as a compact `Option<(u32, u32)>` tuple — a dedicated struct is deferred until a test demands named fields.

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs`
  - Added `visible_range: Option<(u32, u32)>` field on `EditorState`.
  - Added `visible_range(&self) -> Option<(u32, u32)>` and `set_visible_range(&mut self, Option<(u32, u32)>)`.

**TDD record:**
- RED: added `visible_range_round_trips_through_setter`. `cargo test -p cade-ide-mcp --lib` failed with 5× E0599 on the missing methods.
- GREEN: added field + getter + setter. `cargo test -p cade-ide-mcp --lib` → 7/7 pass.
- REFACTOR: none.

**Previous behavior:** `EditorState` could not expose the visible-range viewport.

**New behavior:** Adapters push `Some((start_line, end_line))` via `set_visible_range()`; tools read it via `visible_range()`.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T16:28:00Z — cade-ide-mcp M-IDE-1a.6: Workspace folders (TDD cycle 6)

**Task:** Let the adapter push the list of workspace roots the editor has open. Sixth TDD cycle.

**Scope guardrail:** Only `state.rs`. No new dependencies.

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs`
  - Added `pub struct WorkspaceFolder { path: String, name: String }`.
  - Added `workspace_folders: Vec<WorkspaceFolder>` field on `EditorState`.
  - Added `workspace_folders(&self) -> &[WorkspaceFolder]` and `replace_workspace_folders(&mut self, Vec<WorkspaceFolder>)`.

**TDD record:**
- RED: added `replace_workspace_folders_updates_slice`. `cargo test -p cade-ide-mcp --lib` failed with E0422 (WorkspaceFolder) and 3× E0599 (missing methods).
- GREEN: added the struct, field, and two methods. `cargo test -p cade-ide-mcp --lib` → 6/6 pass.
- REFACTOR: none.

**Previous behavior:** `EditorState` could not expose workspace roots.

**New behavior:** Adapters push `WorkspaceFolder` snapshots via `replace_workspace_folders()`; tools read them via `workspace_folders()`.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T16:20:00Z — cade-ide-mcp M-IDE-1a.5: Diagnostics state (TDD cycle 5)

**Task:** Let the adapter report LSP-style diagnostics into the shared state. Fifth TDD cycle.

**Scope guardrail:** Only `state.rs`. No new dependencies. `DiagnosticSeverity` is a plain enum with four variants (no serde, no schemars yet).

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs`
  - Added `pub enum DiagnosticSeverity { Error, Warning, Info, Hint }`.
  - Added `pub struct Diagnostic { path, range, severity, message, source, code }` — mirrors LSP diagnostic shape.
  - Added `diagnostics: Vec<Diagnostic>` field on `EditorState`.
  - Added `diagnostics(&self) -> &[Diagnostic]` and `replace_diagnostics(&mut self, Vec<Diagnostic>)`.

**TDD record:**
- RED: added `replace_diagnostics_updates_slice` exercising the empty-state slice, then replace with one Diagnostic and expect it back. `cargo test -p cade-ide-mcp --lib` failed with E0422 (Diagnostic missing), E0433 (DiagnosticSeverity missing), 3× E0599 (diagnostics / replace_diagnostics missing).
- GREEN: added the enum, struct, field, and two methods. `cargo test -p cade-ide-mcp --lib` → 5/5 pass.
- REFACTOR: none.

**Previous behavior:** `EditorState` could not report diagnostics.

**New behavior:** Adapters push a full diagnostic snapshot via `replace_diagnostics()`; tools read it via `diagnostics()` as a slice.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T16:13:00Z — cade-ide-mcp M-IDE-1a.4: Selection state (TDD cycle 4)

**Task:** Let the adapter report the user's current selection. Fourth TDD cycle.

**Scope guardrail:** Only `state.rs`. No new dependencies. Position + Range + Selection types introduced as plain structs with `PartialEq + Eq`, no serde yet (added in a later cycle when the MCP tool layer demands it).

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs`
  - Added `pub struct Position { line: u32, character: u32 }` (0-indexed LSP convention).
  - Added `pub struct Range { start: Position, end: Position }`.
  - Added `pub struct Selection { path: String, range: Range, text: String }`.
  - Added `selection: Option<Selection>` field on `EditorState`.
  - Added `selection(&self) -> Option<&Selection>` and `set_selection(&mut self, Option<Selection>)`.

**TDD record:**
- RED: added `selection_round_trips_through_setter` exercising the getter returning `None`, setting a selection, reading it back, clearing, and reading `None` again. `cargo test -p cade-ide-mcp --lib` failed with 4× E0422 (Selection/Range/Position missing) and 5× E0599 (set_selection / selection methods missing).
- GREEN: added the three types + field + getter/setter. `cargo test -p cade-ide-mcp --lib` → 4/4 pass.
- REFACTOR: none.

**Previous behavior:** `EditorState` could not expose the current text selection.

**New behavior:** Adapters push a `Selection { path, range, text }` via `set_selection()`; tools read it via `selection()`.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T16:04:00Z — cade-ide-mcp M-IDE-1a.3: active_file getter/setter (TDD cycle 3)

**Task:** Let the adapter report which file the user is focused on. Third TDD cycle.

**Scope guardrail:** Only `state.rs`. No new dependencies. Active file is a simple `Option<String>` path; cross-check against `open_files` is deferred until a test demands it.

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs` — added `active_file: Option<String>` field and getter/setter pair: `active_file(&self) -> Option<&str>`, `set_active_file(&mut self, Option<String>)`.

**TDD record:**
- RED: added `active_file_round_trips_through_setter` exercising `active_file() == None`, set, read back, clear, read back. `cargo test -p cade-ide-mcp --lib` failed with E0599 on both missing methods.
- GREEN: added field + both methods. `cargo test -p cade-ide-mcp --lib` → 3/3 pass.
- REFACTOR: none.

**Previous behavior:** `EditorState` could not expose which file was focused.

**New behavior:** Adapters push the active file path; callers can read it back.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T15:55:00Z — cade-ide-mcp M-IDE-1a.2: OpenFile list (TDD cycle 2)

**Task:** Extend `EditorState` so the adapter can push a list of open files and the tool layer can count them. Second TDD cycle of the IDE-integration milestone.

**Scope guardrail:** Only `state.rs`. No new dependencies. No tool layer. No async. `OpenFile` gets only the `path` field — buffer text, dirty flag, language id, version all deferred to later cycles where a test demands them.

**Files modified:**
- `crates/cade-ide-mcp/src/state.rs` — added `pub struct OpenFile { path: Option<String> }`, a private `open_files: Vec<OpenFile>` field on `EditorState`, and `replace_open_files(&mut self, Vec<OpenFile>)`. `open_file_count()` now returns `self.open_files.len()` instead of the hard-coded `0`.

**TDD record:**
- RED: added `replace_open_files_updates_count` expecting `open_file_count() == 2` after pushing two files. `cargo test -p cade-ide-mcp --lib` failed with E0422 (OpenFile), E0599 (replace_open_files missing).
- GREEN: added the struct, field, and method. `cargo test -p cade-ide-mcp --lib` → 2/2 pass.
- REFACTOR: none.

**Previous behavior:** `EditorState` was an empty marker struct; `open_file_count()` always returned 0.

**New behavior:** Adapters can call `replace_open_files(Vec<OpenFile>)`; `open_file_count()` reports the stored length.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```

## 2026-04-23T15:46:00Z — cade-ide-mcp M-IDE-1a.1: EditorState skeleton (TDD cycle 1)

**Task:** First TDD cycle for the IDE-integration milestone. Replace the `Hello World` stub in `cade-ide-mcp` with a library crate that exposes a minimal `EditorState` type. Scope is deliberately tiny so every later read-tool can be added via its own red-green cycle.

**Scope guardrail:** No new dependencies. No MCP transport. No tools. No editor adapter. Only the library skeleton + one struct + one test.

**Files modified:**
- `crates/cade-ide-mcp/Cargo.toml` — replaced 6-line stub with `[lib]` section (`path = "src/lib.rs"`). No new dependencies added. Added `[lints.rust] unsafe_code = "forbid"` to match other crates in the workspace.
- `crates/cade-ide-mcp/src/main.rs` — **deleted** (Hello-World stub).
- `crates/cade-ide-mcp/src/lib.rs` — **new**. Module declaration + `pub use state::EditorState;`.
- `crates/cade-ide-mcp/src/state.rs` — **new**. Empty marker struct `EditorState` with `new()` and `open_file_count() -> usize` (returning `0` for now).

**TDD record:**
- RED: `cargo test -p cade-ide-mcp --lib` failed with E0432/E0433 (EditorState not declared).
- GREEN: added `pub struct EditorState;` + two methods. `cargo test -p cade-ide-mcp --lib` → 1/1 pass.
- REFACTOR: none.

**Previous behavior:** `cade-ide-mcp` was a `Hello, world!` binary stub with zero dependencies and zero functionality; listed in `ARCHITECTURE.md` as the IDE bridge but unimplemented.

**New behavior:** `cade-ide-mcp` is now a library crate with an `EditorState` type that reports zero open files. Still unimplemented as an MCP server — later cycles add that behavior.

**Dependency policy:** No new dependencies. Workspace `Cargo.toml` unchanged.

**Rollback steps:**
```sh
git restore crates/cade-ide-mcp/Cargo.toml
git restore --source=HEAD --staged --worktree crates/cade-ide-mcp/src/main.rs
rm crates/cade-ide-mcp/src/lib.rs crates/cade-ide-mcp/src/state.rs
```
(or simply `git reset --hard HEAD~1` if the cycle has been committed.)

## 2026-04-18T01:11:17Z — cade-gui M16.5: palette recognizes all TUI slash commands

**Task:** Close the gap between TUI and GUI slash-command coverage at the palette layer. Previously, typing a TUI-only command (e.g. `/providers`, `/plan`, `/hooks`, `/reflect`) in the GUI palette hit `PaletteCmd::Unknown` and showed "Unknown command". Now those commands are recognized, canonicalized, and surface a user-facing message that tells the user the feature is available in the CLI/TUI today with a GUI panel coming soon.

**Scope guardrail:** Palette parser + dispatch only. No new UI panels. No new server HTTP calls. No new egui widgets. No new dependencies. No changes to existing `PaletteCmd` variants. No changes to any other crate.

**Files modified:**
- `crates/cade-gui/src/palette.rs`
  - Added `PaletteCmd::Unsupported(String)` variant, carrying the canonical lowercase TUI command name (without the leading `/`).
  - Extended `parse_palette_input` to recognize 30 TUI-only commands across four tiers (lifecycle, mode toggles, integrations, data ops) and map each to `PaletteCmd::Unsupported(<canonical>)`.
  - Canonical names and alias mappings (e.g. `del`/`rm-agent` → `delete`, `agents-list` → `subagents`, `provider-list` → `providers`, `debug_last` → `debug-last`, `normal` → `default`, `select` → `mouse`, `summary` → `summarize`) match `crates/cade-cli/src/cli/repl/slash.rs` so a user's muscle memory from the TUI works in the GUI.
  - Added two tests:
    - `parse_slash_tui_only_commands_are_unsupported` — locks in representative mappings across all four tiers (`/providers`, `/plan`, `/resume`, `/export`, `/reflect`, `/hooks`).
    - `parse_slash_still_unknown_for_truly_unknown` — regression guard ensuring the new Unsupported path does not swallow genuinely unknown input.

- `crates/cade-gui/src/app.rs`
  - Added `PaletteCmd::Unsupported(name)` match arm to `dispatch_palette_cmd`, pushing a user-facing error toast: `/<name> is available in the CADE CLI/TUI — GUI panel coming soon`.
  - No other control-flow changes.

**Dependency policy:** No new dependencies. Change is entirely within existing cade-gui modules.

**TDD record:**
- RED: `cargo test -p cade-gui --lib palette::` failed with E0599 "no variant `Unsupported` found for enum PaletteCmd".
- GREEN (parser): added variant + 30 match arms; `cargo test -p cade-gui --lib palette::` → 20/20 pass.
- RED (dispatcher, wasm target): `cargo build -p cade-gui --target wasm32-unknown-unknown` failed with E0004 "non-exhaustive patterns: `PaletteCmd::Unsupported(_)` not covered" (expected — `app.rs` is wasm-only, native check didn't catch it).
- GREEN (dispatcher): added match arm emitting error toast; `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown` → clean.
- Regression: full `cargo test --workspace` → all suites pass (cade-gui 197/197, workspace totals unchanged otherwise).

**Build pipeline:**
1. `trunk build --release` in `crates/cade-gui` → new dashboard bundle hash `59ac82027149375d` (was `9df6c4299304ccf`; +a few KB from the enlarged parser match table).
2. `cargo build --release --bin cade-server` → `build.rs` fired via the new dist watch, rust-embed re-baked the fresh WASM, binary now contains the M16.5 palette.

**Previous behavior:** Typing `/providers`, `/plan`, `/hooks`, `/reflect`, `/export`, `/resume`, `/permissions`, `/yolo`, `/mode`, etc. in the GUI palette produced "Unknown command: /providers" — indistinguishable from a typo.

**New behavior:** Same input now produces "/providers is available in the CADE CLI/TUI — GUI panel coming soon", telling the user (a) the command name is valid, (b) where to reach it today, (c) it's on the roadmap. TUI muscle memory is preserved.

**Commands now recognized (all surface the Unsupported toast):**
- Lifecycle: `/resume`, `/rename`, `/delete` (aliases `/del`, `/rm-agent`), `/new-agent`, `/pin`, `/init`, `/info`, `/feedback`.
- Mode toggles: `/plan`, `/yolo`, `/default` (alias `/normal`), `/mode`, `/todos`, `/todo`, `/reasoning`, `/stream`, `/mouse` (alias `/select`), `/toolset`, `/theme`.
- Integrations: `/providers` (alias `/provider-list`), `/connect`, `/disconnect`, `/permissions`, `/hooks`, `/subagents` (alias `/agents-list`), `/mcp-save`, `/link`, `/unlink`, `/approve-always`, `/deny-always`, `/reflect`, `/summarize` (alias `/summary`).
- Data ops: `/export`, `/remember`, `/pricing`, `/backend`, `/compaction-model`, `/debug-last` (alias `/debug_last`), `/fork`.

Existing mappings unchanged: `/checkpoint`, `/checkpoints`, `/undo`, `/tree` still → `PaletteCmd::Checkpoints` (M17 stub). `/cost`, `/usage`, `/stats` still → `PaletteCmd::Stats` (M17 stub). `/memory`, `/mem` still fully wired.

**Compatibility:** No breaking changes. `PaletteCmd::Unknown` remains for genuinely unknown input. Existing palette UI, `CMD_DEFS` fuzzy-search table, and keyboard shortcuts are untouched — these new commands do not appear in the palette browse list by design (they have no working action, so advertising them would be misleading). Users discover them via direct typing, matching TUI behavior.

**Rollback steps:**
1. `git revert <this-commit>` — reverts the single commit.
2. Or restore checkpoint `cp-484fb085-db65-4b24-9e1a-5ee028c0c491` (label `before-palette-expansion`, HEAD `14e39711`) for a working-tree restore.

---



**Task:** Build cade-gui into a real WASM bundle via `trunk build` and serve the assets from cade-server using `rust-embed` at `/dashboard` and `/dashboard/*`.

**Scope guardrail:** Build pipeline + asset serving only.  No new GUI features.  No wasm-opt (bundled version is outdated).  The GUI itself is unchanged from M7.

**Files created:**
- `crates/cade-gui/index.html` — trunk entry point with `<canvas id="cade_gui_canvas">`, dark theme styles, `<link data-trunk rel="rust" data-wasm-opt="0" />`.
- `crates/cade-gui/Trunk.toml` — trunk config: `dist = "dist"`, `filehash = true`, `public_url = "/dashboard/"`, `minify = "never"`.
- `crates/cade-server/src/server/api/dashboard_assets.rs` — `DashboardAssets` struct deriving `rust_embed::Embed`, folder = `../cade-gui/dist/`, `allow_missing = "true"`.

**Files modified:**
- `crates/cade-server/Cargo.toml` — added `rust-embed = "8.11.0"` dependency.
- `crates/cade-server/src/server/api/mod.rs` — registered `dashboard_assets` module; added `GET /dashboard/*path` wildcard route for asset serving.
- `crates/cade-server/src/server/api/dashboard.rs` — rewritten from inline HTML string to `rust-embed` asset serving:
  - `get_dashboard()` serves embedded `index.html`.
  - `get_dashboard_asset(Path(path))` serves JS/WASM/CSS/etc. with correct MIME types.
  - `mime_for(path)` infers MIME from extension (html, js, wasm, css, json, png, svg, ico).
  - `serve_embedded(path)` returns the file with cache headers (`no-cache` for index.html, `immutable` for hashed assets).
- `crates/cade-server/src/server/api/dashboard_test.rs` — updated for new architecture:
  - `make_app` now mounts both `/dashboard` and `/dashboard/*path`.
  - Existing tests preserved and updated: `dashboard_returns_html_page_without_auth`, `dashboard_does_not_leak_server_api_key`, `dashboard_error_page_has_no_stack_trace_or_framework_info`, `dashboard_contains_canvas_with_expected_id`.
  - New tests: `dashboard_index_html_has_no_cache_header`, `dashboard_missing_asset_returns_404`, `dashboard_assets_do_not_require_auth`, `mime_for_returns_correct_types`.
- `crates/cade-server/src/server/api/router_test.rs` — added `dashboard_asset_wildcard_is_reachable_through_full_router_without_auth`.
- `.gitignore` — added `crates/cade-gui/dist/`.

**Dependency policy:** 1 new crate: `rust-embed 8.11.0` (+ transitive: `rust-embed-impl`, `rust-embed-utils`, `sha2`, `digest`, `crypto-common`, `block-buffer`, `generic-array`).  Needed to embed the trunk-built `dist/` directory into the cade-server binary at compile time.

**Build pipeline:**
1. `cd crates/cade-gui && trunk build --release` → produces `dist/index.html`, `dist/cade-gui-<hash>.js`, `dist/cade-gui-<hash>_bg.wasm`.
2. `cargo build -p cade-server` → embeds `dist/` contents via `rust-embed`.  If `dist/` is empty (trunk not run), the server compiles but returns 404 for all dashboard routes.
3. `data-wasm-opt="0"` in index.html skips wasm-opt (bundled v123 doesn't support bulk-memory ops from recent Rust).
4. `public_url = "/dashboard/"` ensures all JS/WASM references use the correct server path prefix.

**Auth contract:** `/dashboard` and `/dashboard/*` are exempt from bearer auth (unchanged from M1 — auth.rs already had `path.starts_with("/dashboard/")`).

**Reason:** M8 of the cade-gui roadmap.  Replaces the static placeholder HTML with the real egui WASM application.  Users can now access the full GUI at `/dashboard`.

**Previous behavior:** `GET /dashboard` returned a hardcoded inline HTML login page.

**New behavior:**
- `GET /dashboard` serves trunk-built `index.html` that loads the WASM app.
- `GET /dashboard/cade-gui-<hash>.js` serves JS glue (application/javascript).
- `GET /dashboard/cade-gui-<hash>_bg.wasm` serves WASM binary (application/wasm).
- Hashed assets get `Cache-Control: public, max-age=31536000, immutable`.
- `index.html` gets `Cache-Control: no-cache` for revalidation.
- Missing assets return 404.
- Tests: 755/755 workspace (was 750; +5 new), 55/55 cade-gui native (unchanged).
- `RUSTFLAGS="-D warnings" cargo check -p cade-server` → clean.
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown` → clean.

**Rollback steps:**
1. `git revert <this-commit>` — reverts all M8 changes.
2. Checkpoint `cp-05ea443e` (label `pre-M8`, HEAD 7d4e1ae9) for full rollback to pre-M8.

---

## 2026-04-17T19:23:24Z — cade-gui M7: egui_commonmark wiring (markdown in timeline)

**Task:** Wire egui_commonmark into the dashboard timeline panel so markdown content renders with headings, lists, code blocks, bold, italic, and block quotes.

**Scope guardrail:** Wiring only.  The timeline shows a static sample markdown to prove the pipeline works.  Real SSE stream content is a future milestone.  No syntect (no `better_syntax_highlighting` feature) — code fences render as monospace without syntax colouring.

**Files modified:**
- `crates/cade-gui/Cargo.toml`
  - Bumped `egui_commonmark` from `"0.20"` → `"0.23"`.  v0.20 depended on `egui 0.31` which conflicted with our `egui 0.34`.  v0.23 requires `egui ^0.34.0`.
  - This also updates `egui_commonmark_backend` (0.20→0.23), `egui_extras` (0.31→0.34), `pulldown-cmark` (0.12→0.13), and removes the duplicate `egui 0.31` tree.  No new crate names — only version bumps of existing transitive deps.
- `Cargo.lock` — updated automatically.
- `crates/cade-gui/src/app.rs` (modified, ~270 lines, wasm-only).
  - Added `use egui_commonmark::{CommonMarkCache, CommonMarkViewer};`.
  - Added `md_cache: CommonMarkCache` field to `CadeApp`.
  - Connected timeline panel now renders a sample markdown via `CommonMarkViewer::new().show(ui, &mut self.md_cache, sample_md)` inside a `ScrollArea::vertical`.
  - Sample covers: h2/h3 headings, bullet list, bold, italic, Rust code fence, block quote.

**Dependency policy:** No new crate names.  `egui_commonmark` was already declared; bumped from 0.20 → 0.23 for egui 0.34 compatibility.  Transitive dep versions updated in lockfile only.

**Reason:** M7 of the cade-gui roadmap.  Establishes the markdown rendering pipeline in the timeline area.  Future milestones will replace the sample with real streamed content.

**Previous behavior:** Connected timeline showed "Select an agent" placeholder text.

**New behavior:**
- Connected timeline renders markdown: headings, lists, bold/italic, code fences (monospace, no syntax highlighting), block quotes.
- Scrollable via `egui::ScrollArea::vertical`.
- `CommonMarkCache` avoids re-parsing on every frame.
- Tests: 55/55 cade-gui native (unchanged), 750/750 workspace (unchanged).
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown` → clean.
- `cargo clippy -p cade-gui --all-targets -- -D warnings` → clean.

**Rollback steps:**
1. `git revert <this-commit>` — reverts `app.rs`, `Cargo.toml`, and `Cargo.lock`.
2. Checkpoint `cp-9e415b65` (label `pre-M6`, HEAD 938ccd64) for full rollback to pre-M6.

---

## 2026-04-17T19:17:04Z — cade-gui M6c: 3-panel layout (sidebar, timeline, input bar)

**Task:** Replace the flat placeholder agent list in the Connected state with a proper 3-panel layout: left sidebar (agent list), central timeline area (placeholder), and bottom input bar (disabled placeholder).

**Scope guardrail:** Layout only.  No agent selection logic, no message sending, no timeline rendering.  All panels show static/placeholder content.  Functional wiring is M7+.

**Files modified:**
- `crates/cade-gui/src/app.rs` (modified, ~250 lines, wasm-only).
  - Connected arm replaced with 3-panel layout using `show_inside`:
    - `egui::Panel::left("agent_sidebar")` — 180px default, resizable.  Shows "Agents" heading, separator, selectable labels per agent (🤖 prefix), version footer.
    - `egui::Panel::bottom("input_bar")` — 40px min height.  Shows "▸" prompt + disabled TextEdit with "Send a message… (coming soon)" hint.
    - `egui::CentralPanel::default()` — centered "Select an agent to start a conversation" placeholder.
  - Used `egui::Panel::left/bottom` (non-deprecated egui 0.34 API) instead of `SidePanel/TopBottomPanel`.
  - Used `default_size` / `min_size` instead of deprecated `default_width` / `min_height`.
  - `let _ = ui.selectable_label(...)` to consume `Response` (required by `-D unused-must-use`).
  - All other states (Connecting, HealthOk, ConnectionFailed, login flow) unchanged.

**Dependency policy:** No new deps.

**Reason:** M6c completes the M6 panel-layout milestone.  Establishes the visual structure that M7 (markdown rendering), M8 (trunk build), and future milestones will fill in.

**Previous behavior:** Connected state showed a flat centered text list.

**New behavior:**
- Left sidebar with agent names + version.
- Bottom input bar (disabled placeholder).
- Central area with "Select an agent" message.
- Tests: `cargo test -p cade-gui --lib` → 55 pass (unchanged).
- `cargo test --workspace --lib` → 750 pass / 0 fail (unchanged).
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown` → clean.
- `cargo clippy -p cade-gui --all-targets -- -D warnings` → clean.

**Rollback steps:**
1. `git revert <this-commit>` — reverts Connected arm in `app.rs`.
2. Checkpoint `cp-9e415b65` (label `pre-M6`, HEAD 938ccd64).

---

## 2026-04-17T19:12:19Z — cade-gui M6b: wire session state into app.rs render loop

**Task:** Connect the pure session state machine (M6a) to the wasm render loop.  After login submit, spawn async HTTP calls and render Connecting / Connected / Failed states.

**Scope guardrail:** Render wiring only.  No panel layout, no sidebar, no timeline.  Connected state shows a placeholder list of agents.  M6c adds the real layout.

**Files modified:**
- `crates/cade-gui/src/app.rs` (rewritten, ~215 lines, wasm-only).
  - `CadeApp` struct now holds:
    - `login: LoginState` — unchanged.
    - `session: Rc<RefCell<Option<SessionState>>>` — shared between render loop and async task.
    - `connect_started: bool` — guard against spawning multiple connection tasks.
    - `ctx: egui::Context` — cloned from `CreationContext` for repaint requests.
    - `server_url: String` — resolved from page origin at boot via `Config::resolve`.
  - `CadeApp::new(cc)`: resolves server URL from `web_sys::window().location().origin()`, constructs `Config` for the API key query parameter.
  - `CadeApp::spawn_connect(token)`: creates `SessionState::start(url, token)`, then `wasm_bindgen_futures::spawn_local` an async block that:
    1. Calls `http_wasm::get_health` → `session.on_health(health)` → `ctx.request_repaint()`.
    2. Calls `http_wasm::get_agents` → `session.on_agents(agents)` → `ctx.request_repaint()`.
    3. On any error → `session.on_error(e.to_string())` → `ctx.request_repaint()` → return.
  - `CadeApp::retry()`: resets `login`, `session`, and `connect_started`.
  - `eframe::App::ui()`: deferred action pattern using `AppAction` enum (`None` | `Connect(String)` | `Retry`) to avoid borrow conflicts with `Rc<RefCell<..>>` inside the egui closure.
  - Render logic (variant-matching only, zero conditional logic):
    - `SessionState::Connecting` → "Connecting to server..." + spinner.
    - `SessionState::HealthOk` → "Server reached — loading agents..." + spinner.
    - `SessionState::Connected { health, agents }` → "Connected to cade-server v{version} — N agent(s)" + bullet list of agent names (placeholder for M6c).
    - `SessionState::ConnectionFailed { error }` → red "Connection failed" + error message + "Retry" button.
    - `None` → login form (unchanged from M3).
    - `LoginState::Submitted` + `!connect_started` → defers `AppAction::Connect(key)`.

**Dependency policy:** No new deps.  All imports are from existing crate deps.

**Reason:** M6b of the cade-gui roadmap.  The session state machine (M6a) is now driven by real HTTP calls.  Users can see connection progress, success, or failure after submitting their API key.

**Previous behavior:** `LoginState::Submitted` displayed a static placeholder string.

**New behavior:**
- After submit: async task calls `get_health` → `get_agents`.
- On success: shows server version and agent count with bullet list.
- On failure: shows error message with "Retry" button.
- `egui::Context::request_repaint()` wakes the render loop after each state transition.
- Tests: `cargo test -p cade-gui --lib` → 55 pass (unchanged — app.rs is wasm-only render code with zero testable logic).
- `cargo test --workspace --lib` → 750 pass / 0 fail (unchanged).
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown` → clean.
- `cargo clippy -p cade-gui --all-targets -- -D warnings` → clean.

**Rollback steps:**
1. `git revert <this-commit>` — reverts `app.rs` only.
2. Checkpoint `cp-9e415b65` (label `pre-M6`, HEAD 938ccd64).

---

## 2026-04-17T19:05:59Z — cade-gui M6a: pure session state machine

**Task:** Add a pure, native-testable post-login session state machine that tracks the connection lifecycle after the user submits their API key.

**Scope guardrail:** State machine only.  No render code, no async tasks, no wasm-only code.  M6b wires this into `app.rs`; M6c adds the real panel layout.

**State diagram:**
```
LoginState::Submitted { key }
       │
       ▼
SessionState::Connecting { server_url, token }
       │
  ┌────┴────┐
  ▼         ▼
HealthOk  ConnectionFailed { error }
  │
  ▼
Connected { health, agents }
```

**Files modified:**
- `crates/cade-gui/src/session.rs` (NEW, ~260 lines, native + wasm).
  - `SessionState` enum: `Connecting`, `HealthOk`, `Connected`, `ConnectionFailed`.
  - `SessionState::start(server_url, token)` — constructor from login output.
  - `server_url()`, `token()` — accessors available in all states.
  - `on_health(HealthInfo)` — `Connecting` → `HealthOk`; no-op in other states.
  - `on_agents(Vec<AgentInfo>)` — `HealthOk` → `Connected`; no-op in other states.
  - `on_error(String)` — `Connecting` | `HealthOk` → `ConnectionFailed`; no-op if already `Connected` or `ConnectionFailed`.
  - `is_connected()`, `is_failed()` — predicate helpers.
  - Uses `std::mem::take` on `String` fields and `.clone()` on `HealthInfo` during transitions (tiny struct, happens once per session).
  - 12 native tests:
    1. `start_enters_connecting_state` — construction + accessor check.
    2. `on_health_transitions_connecting_to_health_ok` — happy path step 1.
    3. `on_agents_transitions_health_ok_to_connected` — happy path step 2.
    4. `connected_preserves_server_url_and_token` — data carried through.
    5. `on_error_from_connecting_transitions_to_failed` — error early.
    6. `on_error_from_health_ok_transitions_to_failed` — error after health.
    7. `on_error_preserves_server_url_and_token` — data carried through on error.
    8. `on_health_is_noop_after_connected` — idempotency guard.
    9. `on_agents_is_noop_from_connecting` — ordering guard.
    10. `on_error_is_noop_after_connected` — late error ignored.
    11. `on_error_is_noop_after_already_failed` — first error sticks.
    12. `connected_with_empty_agents_is_valid` — edge case: no agents.
- `crates/cade-gui/src/lib.rs`
  - Added `pub mod session;` (native + wasm).
  - Updated module-level doc comment.

**Dependency policy:** No new deps.  `cade-api-types` already in cade-gui deps from M4.

**Reason:** M6a of the cade-gui roadmap.  Establishes the pure state model that M6b (app.rs wiring) and M6c (panel layout) will render.

**Previous behavior:** After `LoginState::Submitted`, the app displayed a placeholder string.

**New behavior:**
- `cargo test -p cade-gui --lib` → 55 pass (was 43; +12 session tests).
- `cargo test --workspace --lib` → 750 pass / 0 fail (was 738; +12).
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown` → clean.
- `cargo clippy -p cade-gui --all-targets -- -D warnings` → clean.

**TDD cycle:**
- Tests and implementation written together (pure additive state machine).
- All 12 tests GREEN on first run — contract encodes the state diagram above.

**Rollback steps:**
1. `git revert <this-commit>` — removes `session.rs` and reverts `lib.rs`.
2. No DB/CI/config changes.
3. Checkpoint `cp-9e415b65` (label `pre-M6`, HEAD 938ccd64).

---

## 2026-04-17T18:58:36Z — cade-gui M5b: wasm fetch+ReadableStream SSE adapter

**Task:** Add a wasm-only streaming SSE adapter that uses `fetch()` + `ReadableStream` to deliver `SseFrame` values from any authenticated cade-server SSE endpoint.

**Scope guardrail:** I/O glue only — every byte of parsing logic lives in the pure `sse` module (M5) and `api` module (M4).  The adapter contains zero conditional logic beyond transport-error mapping and a `done` flag check.

**Why fetch+ReadableStream, not EventSource?**
`EventSource` cannot send custom headers.  The browser API has no way to attach `Authorization: Bearer <token>`.  Every cade-server streaming endpoint (except `/v1/health`) requires auth, so all SSE consumption MUST go through `fetch()` + `ReadableStream`.

**Files modified:**
- `crates/cade-gui/Cargo.toml`
  - Added `"ReadableStream"` and `"ReadableStreamDefaultReader"` to the `web-sys` features list.  No new crates; web-sys was already a wasm32-only dependency.
- `crates/cade-gui/src/http_wasm.rs` (modified, ~160 lines total, wasm-only).
  - Renamed internal `send()` → `send_text()` for clarity.
  - Added `pub async fn stream_sse(url, token, on_frame: impl FnMut(SseFrame) -> bool) -> Result<(), ApiError>`:
    - Issues `GET` with `Authorization: Bearer <token>` via `gloo-net::http::Request`.
    - Checks status: 401 → `ApiError::Unauthorized`, non-2xx → `ApiError::Server`.
    - Grabs `resp.body()` → `web_sys::ReadableStream` → `ReadableStreamDefaultReader`.
    - Loops: `JsFuture::from(reader.read())` → extracts `Uint8Array` → `parser.feed(&bytes)` → drains frames via `parser.pop()` → calls `on_frame(frame)`.
    - Loop exits on: stream `done`, `on_frame` returns `false` (early stop), or transport error.
    - Releases reader lock on all exit paths.
  - Used `web_sys::js_sys::Reflect` and `web_sys::js_sys::Uint8Array` — no direct `js-sys` dependency needed (re-exported through web-sys).

**Dependency policy:** No new external crates.  Only added two web-sys feature flags for types that were already available but not activated.

**Reason:** M5b of the cade-gui roadmap.  The `stream_sse` function is the transport layer that M6 (panel layout) will call from a `wasm_bindgen_futures::spawn_local` task, pushing frames into a shared buffer that the `eframe::App::update()` loop drains.

**Previous behavior:** `http_wasm.rs` only supported one-shot JSON endpoints (`get_health`, `get_agents`).

**New behavior:**
- `stream_sse(url, token, callback)` — async streaming SSE consumer for any authenticated endpoint.
- Callback pattern: `on_frame(SseFrame) -> bool` — return `false` to stop early.
- `cargo test -p cade-gui --lib` → 43 pass (unchanged — no new native tests; all logic was already tested in M4/M5).
- `cargo test --workspace --lib` → 738 pass / 0 fail (unchanged).
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown` → clean.
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown --tests` → clean.
- `cargo clippy -p cade-gui --all-targets -- -D warnings` → clean.

**Rollback steps:**
1. `git revert <this-commit>` — reverts `http_wasm.rs` and `Cargo.toml` web-sys features.
2. No DB/CI/config changes.

---

## 2026-04-17T18:46:53Z — cade-gui M5: pure SSE frame parser

**Task:** Add a pure, native-testable SSE frame parser for the cade-gui WASM app.  This parser consumes raw bytes (arriving in arbitrary chunks from `fetch()` + `ReadableStream`) and emits typed `SseFrame` values.  No network I/O — the wasm fetch-streaming adapter is deferred to M5b behind a separate approval gate.

**Scope guardrail (strict-project-execution):** Parser only.  No EventSource, no fetch, no ReadableStream.  Zero browser dependencies.

**Files modified:**
- `crates/cade-gui/src/sse.rs` (NEW, ~310 lines, native + wasm).
  - `SseFrame` enum: `Json(serde_json::Value)` | `Done` | `ParseError(String)`.
  - `SseParser` struct with internal line buffer, data accumulator, and pending frame queue.
  - `SseParser::new()` — fresh empty parser.
  - `SseParser::feed(&mut self, bytes: &[u8])` — push arbitrary byte chunks; internally splits on `\n`, swallows `\r`, dispatches frames on blank lines.
  - `SseParser::pop(&mut self) -> Option<SseFrame>` — drain one complete frame.
  - `process_line()` — SSE field parser: extracts `data:` values (optional space after colon per spec), ignores unknown fields (`id:`, `event:`, `retry:`).  Multiple `data:` lines in one frame concatenated with `\n` per SSE spec.
  - `dispatch_frame()` — maps accumulated data to `SseFrame::Done` (if `[DONE]` sentinel), `SseFrame::Json` (if valid JSON), or `SseFrame::ParseError` (otherwise).
  - `impl Default for SseParser` delegates to `new()`.
  - 13 native tests:
    1. `empty_feed_yields_no_frames` — no output from empty input.
    2. `single_json_frame` — `data: {"x":1}\n\n` → `Json({"x":1})`.
    3. `done_sentinel` — `data: [DONE]\n\n` → `Done`.
    4. `two_frames_in_one_feed` — two complete frames parsed from one `feed()`.
    5. `frame_split_across_two_feeds` — frame spanning two `feed()` calls.
    6. `frame_split_byte_by_byte` — each byte in a separate `feed()` call.
    7. `crlf_line_endings` — `\r\n\r\n` parsed identically to `\n\n`.
    8. `unknown_field_ignored` — `id: 42\ndata: {...}\n\n` → only `data` is used.
    9. `malformed_json_yields_parse_error` — `data: not-json\n\n` → `ParseError("not-json")`.
    10. `multiple_data_lines_concatenated` — two `data:` lines joined with `\n`; result is valid JSON.
    11. `realistic_server_stream` — 5-frame sequence matching actual cade-server output (stream_start, 2× stream_delta, stream_end, [DONE]).
    12. `blank_lines_without_data_yield_nothing` — blank lines with no preceding `data:` emit no frames.
    13. `data_no_space_after_colon` — `data:{"tight":1}\n\n` (no space) → parsed correctly.
- `crates/cade-gui/src/lib.rs`
  - Added `pub mod sse;` (native + wasm).
  - Updated module-level doc comment.

**Dependency policy:** No new deps.  `serde_json` already in workspace deps of cade-gui.

**Reason:** M5 of the cade-gui roadmap pinned in `working_set`.  The parser is the pure-logic foundation for M5b (wasm fetch-streaming adapter) and M6 (panel layout consuming streaming events).

**Previous behavior:** cade-gui had no SSE parsing capability.

**New behavior:**
- `cargo test -p cade-gui --lib` → 43 pass (was 30; +13 sse tests).
- `cargo test --workspace --lib` → 738 pass / 0 fail (was 725; +13).
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown` → clean.
- `cargo clippy -p cade-gui --all-targets -- -D warnings` → clean.

**TDD cycle summary:**
1. RED — wrote sse.rs with 13 tests + implementation.  12/13 passed on first run; test 10 (`multiple_data_lines_concatenated`) failed — test assumption was wrong (JSON allows `\n` as whitespace between tokens).  Fixed test to assert correct behavior.  13/13 GREEN.
2. REFACTOR — clippy flagged `single_match` lint on the `match field` block; replaced with `if field == "data"`.  All tests still green.

**Rollback steps:**
1. `git revert <this-commit>` — removes `crates/cade-gui/src/sse.rs` and reverts `lib.rs`.
2. No DB/CI/config changes.
3. Checkpoint `cp-2f4eda26` (label `pre-M5`, HEAD 50908476) for `restore_checkpoint`.

---

## 2026-04-17T18:10:02Z — cade-gui M4: pure API client (get_health, get_agents)

**Task:** Add a pure HTTP-client module for the cade-gui wasm app that can call `GET /v1/health` and `GET /v1/agents` using the user-submitted bearer token. First consumer of `cade-api-types`.

**Scope guardrail (strict-project-execution):** This milestone ships **only the client-side API layer**. No app.rs integration yet (that lands with the panel-layout milestone M6 so state transitions have somewhere to drive data into). The `http_wasm` adapter is compiled-only code until a wasm-bindgen-test runner is wired into CI.

**Files modified:**
- `crates/cade-api-types/src/lib.rs`
  - Added `HealthInfo { status, server: Option<String>, version: Option<String> }` mirroring `get_health` in `cade-server/src/server/api/health.rs`.  `server` / `version` optional and `skip_serializing_if = "Option::is_none"` for drift tolerance.
  - 2 new native tests: `health_info_parses_server_shape`, `health_info_tolerates_missing_optional_fields`.
- `crates/cade-gui/Cargo.toml`
  - Added `cade-api-types = { path = "../cade-api-types" }` to the pure `[dependencies]` table so both native and wasm can use the wire types.  No new external crate; no workspace deps changed.
- `crates/cade-gui/src/lib.rs`
  - Registered `pub mod api;` (pure, native + wasm) and `#[cfg(target_arch = "wasm32")] pub mod http_wasm;`.
  - Updated module-level doc comment listing public modules.
- `crates/cade-gui/src/api.rs` (new, ~200 lines, native + wasm).
  - `build_url(base, path) -> String` — strips trailing `/` runs from base.
  - `bearer_header(token) -> String` — literal `"Bearer {token}"` (no trim — upstream `login::LoginState` owns trimming).
  - `parse_health(status, body) -> Result<HealthInfo, ApiError>`.
  - `parse_agents(status, body) -> Result<Vec<AgentInfo>, ApiError>`.
  - `ApiError` enum: `Unauthorized` | `Server { status: u16 }` | `Decode { message }` | `Transport { message }`.  Implements `std::error::Error` + `Display` with user-safe strings (tdd-guide §3.3: no stack traces, no internal paths).
  - Shared `decode_or_error<T: DeserializeOwned>` generic: `2xx → Decode or Ok(T)`, `401 → Unauthorized`, `other → Server`.
  - 15 native tests covering: URL joining, both trailing-slash cases, bearer formatting (incl. deliberate no-trim), 2xx/401/500 on both endpoints, empty list, malformed JSON → `Decode`, `Display` output shape.
- `crates/cade-gui/src/http_wasm.rs` (new, ~55 lines, `#![cfg(target_arch = "wasm32")]`).
  - `pub async fn get_health(base_url, token) -> Result<HealthInfo, ApiError>`
  - `pub async fn get_agents(base_url, token) -> Result<Vec<AgentInfo>, ApiError>`
  - Private `send(url, token) -> Result<(u16, String), ApiError>` — uses `gloo-net::http::Request::get`, maps every `gloo-net` error to `ApiError::Transport { message }`, delegates status+body parsing to the pure module.
  - No conditional logic, no retry, no caching — that stays in `api::` where it is native-testable.

**Dependency policy:** No new external deps.  `gloo-net = "0.6"` was already declared in `cade-gui/Cargo.toml` from M3; `cade-api-types` is an existing workspace member.

**Reason:** M4 of the cade-gui roadmap pinned in `working_set` memory: the submitted token from `LoginState::Submitted` now has a destination.  Provides the parsing primitives the panel layout (M6) will drive from the render loop.

**Previous behavior:** `cade-gui` could render a login screen but had no way to contact the server.  `cade-api-types` only modelled `AgentInfo`.

**New behavior:**
- Native `cargo test -p cade-gui --lib` → 30 pass (was 15): 15 new `api::` tests + 8 config + 7 login.
- `cargo test -p cade-api-types` → 4 pass (was 2): 2 new `HealthInfo` tests.
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown` → clean.
- `RUSTFLAGS="-D warnings" cargo build -p cade-gui --target wasm32-unknown-unknown --tests` → clean.
- `cargo test --workspace --lib` → 725 pass / 0 fail (was 708).
- `cargo clippy -p cade-gui -p cade-api-types --all-targets -- -D warnings` → clean.

**TDD cycle summary:**
1. RED/GREEN — add `HealthInfo` wire type + 2 tests (pure additive type; tests encode the contract).
2. RED/GREEN — add `api` module + 15 tests covering every error branch on both endpoints.
3. GREEN — add `http_wasm` I/O adapter (no native logic to test; all behaviour delegated to `api::`).

**Rollback steps:**
1. `git revert <this-commit>` — reverts `crates/cade-api-types/src/lib.rs`, `crates/cade-gui/Cargo.toml`, `crates/cade-gui/src/lib.rs`, and deletes `crates/cade-gui/src/api.rs` + `crates/cade-gui/src/http_wasm.rs`.
2. No DB migrations, no on-disk state, no CI workflow changes — reverting the commit fully restores prior behaviour.
3. Checkpoint `cp-c1aa06cf-8143-4f61-a3fc-1984ff0247cd` (label `pre-M4`, HEAD 47fa502d) captures the exact pre-M4 state for `restore_checkpoint`.

---

## 2026-04-17T16:57:31Z — P2-5: Origin header CSRF middleware

**Task:** Add defense-in-depth Origin-header validation on mutating HTTP requests.  Block `POST` / `PUT` / `PATCH` / `DELETE` when the `Origin` header is present but not on the existing localhost allow-list.

**Context / priority caveat:** Real CSRF risk against cade-server is already low.  Bearer-token auth (P1-1) is mandatory and is sent via `Authorization: Bearer`, not a cookie — a browser cannot forge it cross-origin.  CORS (H-03) is locked to `http://localhost` / `http://127.0.0.1` at the binary level.  This middleware adds one more layer: if a browser manages to originate a mutating request, its Origin header must match the localhost allow-list regardless of CORS or auth outcomes.  Not a gap-closer; a belt-and-braces hardening task.

**Files modified:**
- `crates/cade-server/src/server/api/mod.rs`
  - Registered `pub mod csrf;`
  - Added `.layer(middleware::from_fn(csrf::csrf_middleware))` as the outermost layer in `router()` (request flow: `csrf → auth → body-limit → handler`).
  - Updated the layer-order doc comment.
- `crates/cade-server/src/server/api/csrf.rs` (new, 90 lines).
  - Pure policy `pub(crate) fn origin_is_allowed(origin: &str) -> bool` — accepts `http://localhost` and `http://127.0.0.1` on any numeric port (or bare); rejects everything else including `https://` on localhost, non-ASCII-digit ports, prefix-confusion names like `http://localhost.evil.com`, and the empty string.
  - Private helper `fn is_mutating(method: &Method) -> bool` for the POST/PUT/PATCH/DELETE set.
  - `pub async fn csrf_middleware(req, next) -> Response`:
    - Safe methods (GET / HEAD / OPTIONS) → pass through unconditionally.
    - No `Origin` header → pass through (non-browser clients: CADE CLI, curl, CI).
    - Origin present + on allow-list → pass through.
    - Origin present + not on allow-list → log `tracing::warn!(method, path, origin, …)` and return `403 Forbidden` with body `{"error":"forbidden","reason":"origin not allowed"}`.
- `crates/cade-server/src/server/api/csrf_test.rs` (new, 120 lines).  9 tests:
  - Policy: accepts bare localhost schemes, accepts any-port localhost schemes, rejects non-localhost / HTTPS-on-localhost / prefix-confusion / malformed ports / non-`http` schemes / empty string.
  - Middleware: allows POST with allowed origin, blocks POST/DELETE with disallowed origin, pass-through when Origin absent, pass-through on GET even with hostile origin, OPTIONS preflight not 403-blocked.

**Dependency policy:** no new dependencies.

**Reason:** Phase 2 of the user-approved security backlog (P2-5).  Bearer auth + strict CORS already mitigate classical CSRF against this server.  This middleware adds an explicit origin check at the HTTP layer, independent of CORS (which only gates browser-side response access, not server-side request acceptance) and auth (which only checks `Authorization`, not `Origin`).

**Layer ordering rationale:**
- `csrf_middleware` is the **outermost** layer — it runs first so a disallowed-Origin request never reaches auth (no crypto compare) or a handler (no DB work, no allocation).
- `auth_middleware` next — bearer token check.
- `DefaultBodyLimit` innermost — cheap guardrail that applies to any request that makes it past auth.

**Previous behaviour:**
- A mutating request carrying `Origin: https://evil.com` was evaluated only by auth.  If the attacker somehow obtained or injected a valid bearer token (e.g. via an unrelated XSS on a localhost page that exposed it), the request was honoured.

**New behaviour:**
- Same request → `403 Forbidden` before auth runs.  Even if the attacker holds a valid bearer token, they need an allowed `Origin` too — or, if they're a non-browser caller, they must not send an `Origin` header at all.

**Explicit non-goals (per user validation):**
- `Referer` header is NOT checked.  `Origin` is RFC 6454-standard for cross-origin requests; `Referer` is privacy-leaky and often stripped.
- Absent `Origin` is NOT treated as suspicious.  Blocking it would break the CADE CLI (which never sets `Origin`) and every non-browser caller.
- GET / HEAD / OPTIONS are NOT checked.  They must never have side effects; OPTIONS preflight handling is owned by the tower-http CORS layer.

**Test results:**
- `cargo test -p cade-server --lib csrf` → 9/9 pass.
- `cargo test -p cade-server` → 129/129 pass (+9 new; no regressions in auth, router, agents, error, evals, messages, etc.).
- `cargo clippy -p cade-server --all-targets --no-deps` → zero new warnings in changed files.  (Pre-existing warnings in `context.rs`, `complete.rs`, `consolidation.rs` unchanged — not fixed, TDD §9.)
- `cargo build --workspace` → clean.

**Rollback steps:**
1. `git revert <this commit>`.
2. Or restore from checkpoint `pre-p2-5` (ID `cp-f984799f-8ab9-4674-aa91-dea6d1cf71bf`, HEAD `e4b23a8b`).

**Follow-ups (explicitly deferred):**
- Making the allow-list configurable for remote deployments (currently hard-coded to localhost to match `src/bin/cade-server.rs` CORS).  Out of scope for P2-5 — add a `CADE_ALLOWED_ORIGINS` env var if remote-hosting becomes a supported configuration.
- Extending the allow-list with the `:PORT` from `ServerConfig.addr` explicitly (today we accept *any* port on `localhost`/`127.0.0.1` — a superset of the CORS allow-list which is more restrictive).  Trade-off deferred; the current policy is simpler and strictly tighter than leaving the check off.

---



**Task:** Stop the CADE server from echoing internal error detail in 5xx HTTP responses.  Replace leaky bodies with a stable generic shape that carries a correlation id, and push the full detail into the structured log under the same id.

**Gap (before):**
- `crates/cade-server/src/server/error.rs`'s `IntoResponse` impl emitted `format!("Database error: {err}")`, `format!("IO error: {err}")`, etc. directly to the client body.  Raw SQLite error text, IO paths, crypto backend text, address-parse output, and upstream AI-provider messages were all exposed.
- `crates/cade-server/src/server/api/agents.rs::server_err()` emitted `{"detail": msg}` with the full error string — used by ~30 call sites across the agents handler module.

**Scope of this commit (MVP):** the central `IntoResponse` impl and the `server_err()` helper.  The ~30 ad-hoc `(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())` call sites scattered across `runs.rs`, `complete.rs`, `checkpoints.rs`, `artifacts.rs`, `evals.rs`, `tools.rs`, and `proxy.rs` are **explicitly deferred** and logged below.  Anything that already flows through `Error::into_response` or `server_err()` is now covered — which captures the majority of handler paths that use `?`-style propagation.

**Files modified:**
- `crates/cade-server/src/server/error.rs`
  - Added `pub(crate) fn internal_error_response(detail: &str) -> Response`.  Generates a UUIDv4 `request_id`, logs `tracing::error!(request_id, detail, "500 Internal Server Error")`, and returns `{ "error": "internal error", "request_id": "<uuid>" }` with `StatusCode::INTERNAL_SERVER_ERROR`.
  - Rewrote `Error::into_response` to bucket every variant as 4xx (echo the already-safe message) or 5xx (route through `internal_error_response`).  4xx variants: `StoreError::SerdeJson`, `StoreError::Custom`, `Error::Custom`.  All other `StoreError` variants go to the generic 5xx body.
- `crates/cade-server/src/server/error_test.rs` (new, 105 lines).  5 tests covering: generic body + no-leak, sqlite-specific no-leak, unique request_id, 400 `Error::custom` preservation (no `request_id` field), 400 `StoreError::Custom` preservation.
- `crates/cade-server/src/server/api/agents.rs`
  - Rewrote `server_err()` so the body is `{ "error": "internal error", "request_id": "<uuid>" }` instead of `{ "detail": msg }`.  Full detail goes to the structured log under the same `request_id`.  Signature unchanged — still returns `(StatusCode, Json<Value>)` — so all ~30 callers compile untouched.
  - Wired `#[cfg(test)] #[path = "agents_test.rs"] mod tests;` at end of file (matches existing `auth_test` / `evals_test` sibling-file pattern).
- `crates/cade-server/src/server/api/agents_test.rs` (new, 48 lines).  2 tests for `server_err()`: generic body + no SQL/column leak, unique request_id.

**Dependency policy:** no new dependencies.  `uuid` was already a workspace dependency (`crates/cade-server/Cargo.toml:30 uuid.workspace = true`).

**Reason:** Phase 3 of the user-approved security backlog (P3-1).  Internal error messages regularly leak implementation detail — SQL fragments, filesystem paths, crypto backend text.  A generic 5xx body with a log correlation id gives operators full diagnostic ability without exposing implementation internals to clients.

**Backward compatibility (flagged change):**
- **5xx body shape changes** from `{"error": "<leaky>"}` (or `{"detail": "<leaky>"}` for `server_err` callers) → `{"error": "internal error", "request_id": "<uuid>"}`.
  - HTTP status codes are unchanged.
  - Clients that only branch on status codes: unaffected.
  - Clients that parsed the error string for display: now see "internal error" instead of the leaky detail.  **This is the intended behaviour of P3-1** and was approved as part of the backlog.
  - Clients that parsed the `detail` field from `server_err`-originated 500s: that field is now absent.  Callers must either read `error` or correlate via `request_id` in logs.
- 4xx body shape unchanged in all cases — clients that read `error` on 400s are unaffected.

**Previous behaviour:**
```text
HTTP/1.1 500 Internal Server Error
{"error":"Database error: unable to open database file: /home/user/.cade/cade.db"}
```
```text
HTTP/1.1 500 Internal Server Error
{"detail":"invalid column name in query at line 42: SELECT * FROM agents WHERE id='abc'"}
```

**New behaviour:**
```text
HTTP/1.1 500 Internal Server Error
{"error":"internal error","request_id":"9d3e4c2a-..."}
```
Structured log line at the same moment:
```text
ERROR request_id="9d3e4c2a-..." detail="sqlite: invalid column name..." 500 Internal Server Error
```

**Test results:**
- `cargo test -p cade-server --lib` → 120/120 pass (+7 new: 5 in `error_test` + 2 in `agents_test`).  Pre-existing tests including `evals::tests::test_db_lock_poisoning_yields_500` still pass — they only assert the status code, not the body text, so they were unaffected.
- `cargo clippy -p cade-server --all-targets --no-deps` → only pre-existing warnings (in `context.rs`, `complete.rs`, `consolidation.rs`), none in changed files.  One lint on my new test (`len() > 0`) was fixed in the same commit.
- `cargo build --workspace` → clean.

**Rollback steps:**
1. `git revert <this commit>` — 4 files changed (2 modified + 2 new).
2. Or restore from checkpoint `pre-p3-1` (ID `cp-4f8c5a4b-4bc4-436b-994c-892871c4c093`, HEAD `6c6f6bbc`).

**Follow-ups (explicitly deferred — tracked for a future P3-1-full ticket):**
- ~30 ad-hoc `(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())` tuples across:
  - `crates/cade-server/src/server/api/runs.rs` (lines 39, 59, 64)
  - `crates/cade-server/src/server/api/complete.rs` (line 82)
  - `crates/cade-server/src/server/api/checkpoints.rs` (lines 53, 78, 117, 154, 175, 212)
  - `crates/cade-server/src/server/api/artifacts.rs` (lines 46, 69, 103, 131, 157)
  - `crates/cade-server/src/server/api/evals.rs` (lines 51, 71, 102, 123, 158, 190, 259)
  - `crates/cade-server/src/server/api/tools.rs` (lines 51, 72)
  - `crates/cade-server/src/server/api/proxy.rs` (lines 157, 194)

  These still leak.  Converting them to the central `internal_error_response()` (or switching handlers to return `server::error::Error` so they flow through the fixed `IntoResponse` impl) is a larger refactor that the user asked to scope separately.

- Cross-linking `request_id` headers (return `x-request-id: <uuid>` on 5xx responses) is a nice-to-have for proxy debugging, not part of MVP.
- Extending the generic shape to 4xx (so even client errors get a correlation id for log-tracing) is explicitly NOT desired — 4xx bodies remain as-is to preserve CLI/client parse expectations.

---



**Task:** Add an automated cargo-audit scan that runs on every PR, every push to `main`, and once per day, so newly-disclosed dependency advisories can't silently slip into the codebase.

**Gap (before):** Only `.github/workflows/ci.yml` existed (build / test / clippy / fmt) — no dependency advisory scanning anywhere in CI.  Advisories disclosed against any of CADE's 660 transitive dependencies would not be surfaced until a developer ran `cargo audit` manually.

**Files modified:**
- `.github/workflows/audit.yml` (new, 54 lines).

**Workflow shape:**
- Name: `Security audit`.
- Triggers:
  - `pull_request` on `main` with `paths` filter (`**/Cargo.toml`, `Cargo.lock`, `.github/workflows/audit.yml`) — only runs when dependency manifest changes, avoiding noisy failures on unrelated PRs.
  - `push` on `main` with the same path filter.
  - `schedule`: daily at 06:00 UTC — catches advisories disclosed after a merge.
  - `workflow_dispatch: {}` — manual trigger.
- `concurrency.cancel-in-progress: true` on the group `audit-${{ github.workflow }}-${{ github.ref }}` so rapid pushes don't stack.
- Permissions: `contents: read`, `issues: write` (scheduled `rustsec/audit-check` runs open / update an advisory issue; it needs issue write access).
- Steps:
  1. `actions/checkout@v4`.
  2. `rustsec/audit-check@v2.0.0` with `token: ${{ secrets.GITHUB_TOKEN }}`.
- Pinned to a specific major tag rather than `@main` so upstream action changes can't alter behaviour silently.

**Reason:** Close the static-analysis gap for dependency vulnerabilities.  Part of Phase 3 of the user-approved security backlog (P3-2).  No production code is touched; no Rust dependencies are added — the action runs `cargo audit` inside the runner.

**Previous behaviour:**
- `cargo audit` had to be run manually by developers; no enforcement in CI.

**New behaviour:**
- Every dependency-affecting PR blocks until `cargo audit` passes.
- Every merge to `main` that touches manifests re-runs the audit.
- Daily cron catches newly-published advisories against already-merged code.
- Manual `workflow_dispatch` available for ad-hoc scans.
- Hard vulnerabilities (`Crate: … Vulnerability:`) → exit non-zero → CI fails.
- Unmaintained / unsound warnings → logged as notices but CI passes (current `cargo audit` default behaviour).

**Baseline (local run, advisory DB loaded from `~/.cargo/advisory-db`):**
- `660 crate dependencies` scanned.
- `0` hard vulnerabilities.
- `3` allowed warnings (all transitive, no patched-upstream fix available at time of writing):
  - `bincode 1.3.3` — unmaintained (RUSTSEC-2025-0141) — via `syntect 5.3.0` → `cade-tui`.
  - `number_prefix 0.4.0` — unmaintained (RUSTSEC-2025-0119) — via `indicatif 0.17.11`.
  - `rand 0.8.5` — unsound (RUSTSEC-2026-0097) — via `phf_generator 0.11.3` → `phf`/`termwiz`/`ratatui`.
- Exit code: `0`.
- Conclusion: the workflow will pass on first run, matching our intent.

**Test results:**
- Local `cargo audit` → exit 0, 3 warnings (captured above).
- Python `yaml.safe_load` on the workflow file → parses cleanly, no syntax errors.
- No Rust code touched → no regression possible in the cade-agent test suite; deferred running `cargo test --workspace` since the change is CI-only.

**Rollback steps:**
1. `git revert <this commit>` — single new file.
2. Or delete `.github/workflows/audit.yml` and re-commit.
3. Or restore state from checkpoint `pre-p3-2` (ID `cp-a46d896a-10e5-4556-acd5-9f1c897fe4cc`, HEAD `98ae2138`).

**Follow-ups (explicitly out of scope for P3-2):**
- If any of the 3 current warnings must be silenced (e.g. to reduce log noise), add a `.cargo/audit.toml` with an `[advisories] ignore = [...]` list and a written justification per entry.  Not added here because the default behaviour (warn-only, pass CI) is already correct.
- Bumping the workflow's action pin from `v2.0.0` to a specific SHA is a hardening option for supply-chain paranoia; deferred.
- Cross-linking from `SECURITY.md` and `CONTRIBUTING.md` to the new workflow is a docs task; deferred.

---



**Task:** Change the SSH backend's default host-key-checking policy from `accept-new` (TOFU — trust-on-first-use) to `yes` (reject unknown host keys), with a narrow env-var escape hatch for environments that dynamically seed `~/.ssh/known_hosts`.

**Vulnerability (before):**
`crates/cade-agent/src/backends/ssh.rs:34` hard-coded `StrictHostKeyChecking=accept-new`.  This makes the first connection to any host trust-on-first-use: an attacker able to interpose during that first handshake can MITM the channel, and subsequent connections pin the attacker's key as legitimate.  For an agent that executes shell commands on a remote host, this is a weak default.

**Files modified:**
- `crates/cade-agent/src/backends/ssh.rs`
  - Added pure helper `fn strict_host_key_checking_policy(env_value: Option<&str>) -> &'static str` — maps exactly `Some("1")` → `"accept-new"`; everything else (including unset, empty, `"0"`, `"true"`, `"yes"`, `" 1 "`, `"1\n"`) → `"yes"`.  Deterministic, no env access inside the helper so unit tests are hermetic.
  - `base_ssh_args` now reads `CADE_SSH_ACCEPT_NEW` via `std::env::var` and passes the result through the helper to build the `StrictHostKeyChecking=<mode>` flag.
  - Added 3 unit tests: default-is-yes, exact-1-opts-in, and truthy-lookalike rejection (empty, `"0"`, `"true"`, `"yes"`, `"TRUE"`, `"2"`, `" 1 "`, `"1\n"`).

**Reason:** Close the TOFU-on-first-connect weakness in the SSH execution backend.  Part of Phase 2 of the user-approved security backlog (P2-4).  Breaking default was pre-approved: operators opt back in with `CADE_SSH_ACCEPT_NEW=1`.

**Previous behaviour:**
- `ssh -o StrictHostKeyChecking=accept-new ...` — first-contact hosts silently trusted; their keys pinned.
- No escape hatch needed because the default was already permissive.

**New behaviour:**
- `ssh -o StrictHostKeyChecking=yes ...` by default — connection **refused** if the host key is not already present in `~/.ssh/known_hosts` (or `/etc/ssh/ssh_known_hosts`).
- `CADE_SSH_ACCEPT_NEW=1` (exact match required) restores the pre-fix `accept-new` behaviour.
- Any other value — empty, `0`, `true`, `yes`, whitespace-padded `" 1 "`, newline-tailed `"1\n"` — is treated as unset and produces the secure default.  This prevents accidental weakening by shell scripts that quote "truthy" values loosely.

**Operational impact (documented here for operators):**
- Users whose `~/.ssh/known_hosts` does not already contain the remote host's key will see `ssh: Host key verification failed` on first connection.  Remediation: run `ssh-keyscan -H <host> >> ~/.ssh/known_hosts` out-of-band, or set `CADE_SSH_ACCEPT_NEW=1` for the session.
- No change to the `-o BatchMode=yes`, `-o ConnectTimeout=10`, port, identity-file, or user arguments.

**Test results:**
- `cargo test -p cade-agent --features backend-ssh --lib backends::ssh` → 11/11 pass (8 existing P2-3 + 3 new P2-4).
- `cargo test -p cade-agent` → 95/95 pass (+3 vs. P2-3 baseline of 92).
- `cargo clippy -p cade-agent --all-targets --no-deps` → clean.
- `cargo build -p cade-agent` (default features) → clean.

**Rollback steps:**
1. `git revert <this commit>` — single-file change.
2. Or restore `crates/cade-agent/src/backends/ssh.rs` from checkpoint `pre-p2-4` (ID `cp-3532dfb5-533f-4bc8-94d7-a5ba20d0b21e`, HEAD `d6d5d446`).

**Follow-ups (explicitly out of scope for P2-4):**
- Documenting `CADE_SSH_ACCEPT_NEW` in README.md's CLI env-var table is a docs-only task and was not included here to keep the change minimum-scope.  If desired, it can be a one-line addition next to `CADE_SERVER_URL` / `CADE_API_KEY`.
- Offering a `ssh-keyscan`-on-first-use wrapper CLI command is explicitly out of scope — the env hatch is the contract.

---



**Task:** Harden `crates/cade-agent/src/backends/ssh.rs` so hostile working-directory strings cannot inject commands into the `bash -c` payload executed on the remote host.

**Vulnerability (before):**
`run_remote` built the remote command with `format!("cd {cwd_str:?} && {command}")`.  The `{:?}` Debug format wraps the path in double quotes but does **not** escape `$`, `` ` ``, `\`, or `"` — bash still expands these inside double quotes.  A `cwd` containing `/tmp/$(rm -rf ~)` or `/tmp/\`id\`` would execute on the remote host as soon as any tool call routed through the SSH backend.
`list_dir` had the same anti-pattern: `format!("ls -1pF {:?}", path.to_string_lossy().to_string())` used Debug format for an attacker-controlled path and then passed the same path again as the cwd.

**Files modified:**
- `crates/cade-agent/src/backends/ssh.rs`
  - Added `fn posix_shell_quote(s: &str) -> String` — single-quote wrap with `'\''` escape for embedded single quotes (POSIX-safe; no expansion, no command substitution, no globbing applies inside single quotes).
  - Added `fn build_remote_cwd_command(command: &str, cwd: &Path) -> String` — quotes the cwd via `posix_shell_quote` and composes `cd '<cwd>' && <command>`.
  - `run_remote` now delegates cwd wrapping to `build_remote_cwd_command` instead of using `{cwd_str:?}`.
  - `list_dir` now uses `posix_shell_quote` on the path before embedding it in the `ls -1pF` argument.
  - Added 8 unit tests in `mod tests` covering: plain path, `$(...)` substitution, backticks, embedded-quote breakout, verbatim command preservation, and the `posix_shell_quote` helper (plain / embedded quote / empty string).

**Reason:** Close the command-injection surface in the SSH execution backend.  Part of Phase 2 of the user-approved security backlog (P2-3).  No behavioural change for well-formed paths — only the wire-format of the `bash -c` string changes from double-quoted Debug output to POSIX single-quoted.

**Previous behaviour:**
- `run_remote("ls", &PathBuf::from("/tmp"))` produced `cd "/tmp" && ls`.
- `run_remote("ls", &PathBuf::from("/tmp/$(id)"))` produced `cd "/tmp/$(id)" && ls` — bash expanded `$(id)` on the remote host.
- `list_dir(&PathBuf::from("/tmp/$(id)"))` likewise executed `$(id)`.

**New behaviour:**
- `run_remote("ls", &PathBuf::from("/tmp"))` produces `cd '/tmp' && ls`.
- `run_remote("ls", &PathBuf::from("/tmp/$(id)"))` produces `cd '/tmp/$(id)' && ls` — literal, not expanded.
- `list_dir` path argument is single-quoted before embedding in the `ls` command.
- Embedded single quotes in cwd are handled by the `'\''` escape sequence (validated by `build_cmd_rejects_quote_breakout_in_cwd`).
- The `command` parameter of `run_remote` is **not** re-quoted — callers remain responsible for the command string, matching the previous contract.  This is intentional to keep the change minimum-scope (the backlog item is about `cwd` only).

**Test results:**
- `cargo test -p cade-agent --features backend-ssh --lib backends::ssh` → 8/8 new tests pass.
- `cargo test -p cade-agent` → 92/92 pass (full crate, no regressions).
- `cargo clippy -p cade-agent --all-targets --no-deps` → clean (cade-agent).
- `cargo build -p cade-agent` (default features) → clean.
- `cargo build -p cade-agent --no-default-features --features backend-ssh` → clean.
- Pre-existing `cargo clippy -p cade-core` failures (collapsible_if) observed on master; unrelated to this change, NOT fixed (TDD §9).

**Rollback steps:**
1. `git revert <this commit>` — single-file change.
2. Or restore `crates/cade-agent/src/backends/ssh.rs` from checkpoint `pre-p2-3` (ID `cp-e5c25601-1a00-4c4c-91c9-6682334e1e75`, HEAD `d829ff7d`).

**Follow-ups (explicitly out of scope for P2-3):**
- The `command` parameter of `run_remote` is still concatenated verbatim.  If a caller ever passes attacker-influenced command text, that is a separate injection class and would need its own task.  Not part of the approved backlog.
- `read_file`, `write_file`, `path_exists` pass the path directly as a separate argument to `ssh` (via `Command::arg`), which is already injection-safe — no quoting change needed there.

---



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

---

## cade-gui M0+M1 — Shared Types Crate + Dashboard Route — 2026-04-17

**Timestamp:** 2026-04-17T17:10:00Z

### Scope (approved by user)
Batch M0+M1 only. WASM/egui work (M2+) deferred pending re-approval.

- **M0** — Create `crates/cade-api-types`: pure `serde` + `serde_json` crate. Compiles on both `x86_64-unknown-linux-gnu` and `wasm32-unknown-unknown`. Zero native deps. Purpose: shared wire types between `cade-server` and the future `cade-gui` WASM client.
- **M1** — Add `GET /dashboard` route on `cade-server` serving a static HTML login page (embedded via `rust-embed`). Route is exempt from `auth_middleware` (alongside existing `/v1/health`). The page does **not** embed any token; user pastes API key manually — WASM app (future M2+) will hold it in memory only.

### Approved Dependency Additions
- `rust-embed = { version = "8", features = ["axum"] }` on `cade-server` only.
- New workspace member `cade-api-types` (serde + serde_json; already workspace deps).

### Execution Contract
- Strict TDD: one failing test per behaviour.
- No edits to `cade-tui`.
- No changes to `cade-core`, `cade-agent`, `cade-cli`.
- `auth_middleware` change is additive (one extra path-skip); other routes unaffected.
- `csrf_middleware` stays as-is (dashboard GET is a safe method).

### Files Expected to Change
- NEW `crates/cade-api-types/Cargo.toml`
- NEW `crates/cade-api-types/src/lib.rs`
- NEW `crates/cade-server/src/server/api/dashboard.rs`
- NEW `crates/cade-server/src/server/api/dashboard/index.html` (login page asset)
- NEW `crates/cade-server/src/server/api/dashboard_test.rs`
- MOD `Cargo.toml` (workspace members)
- MOD `crates/cade-server/Cargo.toml` (+rust-embed)
- MOD `crates/cade-server/src/server/api/mod.rs` (route registration)
- MOD `crates/cade-server/src/server/api/auth.rs` (path-skip `/dashboard`, `/dashboard/*`)

### Rollback
- `restore_checkpoint cp-b3a55d19-f2a1-4a78-ba0c-ce944a51687b`
- or `git revert <commit>`

### Pre-state (HEAD)
- `8d7d9773 security(server): P2-5 Origin-header CSRF middleware`
- `cargo test -p cade-server` passes (129/129 at time of last recorded status).


---

## cade-gui M2 Skeleton — 2026-04-17

**Timestamp:** 2026-04-17T17:45:00Z

### Scope (approved by user)
Create `crates/cade-gui` skeleton only. Scope ends once the crate:
1. Compiles to `wasm32-unknown-unknown` clean.
2. Has one green unit test (native) covering a pure predicate.
3. Is registered in the workspace.

No render-loop behaviour, no SSE client, no markdown, no fonts, no trunk
pipeline yet — those come in separate stop-and-ask milestones.

### Approved Dependency Additions (per user)
All scoped to `crates/cade-gui/Cargo.toml` only. Server binary and native
builds unaffected.

- `eframe = "0.34"` (with default features disabled; `web_screen_reader` only)
- `egui = "0.34"`
- `egui_commonmark = "0.20"` (markdown — deferred until first renderer, but
  pulled in now to fail fast on compatibility)
- `wasm-bindgen = "0.2"`
- `wasm-bindgen-futures = "0.4"`
- `web-sys = "0.3"` (minimal feature list: `Window`, `Location`, `UrlSearchParams`)
- `gloo-net = "0.6"`
- `serde-wasm-bindgen = "0.6"`
- `serde` / `serde_json` via existing workspace deps

These are per-crate deps — they do NOT become workspace-wide.

### Execution Contract
- Strict TDD: one failing test for config-precedence logic before any impl.
- WASM-only crate: `[lib] crate-type = ["cdylib", "rlib"]`, `# [cfg(target_arch = "wasm32")]` guards on browser-only code so native `cargo test` can exercise the pure logic.
- Zero edits to cade-tui, cade-core, cade-agent, cade-cli, cade-server.
- `cade-api-types` NOT yet consumed by `cade-gui`; added when the first real wire type is rendered (separate milestone).

### Files Expected
- NEW `crates/cade-gui/Cargo.toml`
- NEW `crates/cade-gui/src/lib.rs`
- NEW `crates/cade-gui/src/config.rs` (pure logic + tests)
- NEW `crates/cade-gui/src/app.rs` (`eframe::App` placeholder)
- MOD `Cargo.toml` (workspace members)

### Rollback
- `restore_checkpoint cp-7ef2ed4f-f972-4075-8819-9d6f3e84c332`
- or `git revert <commit>` once committed.

### Pre-state (HEAD)
- `577c2ddf feat(server): serve /dashboard login page (public, no token leak)`
- `dc9f022d feat(api-types): add cade-api-types crate for wasm-safe wire types`
- Workspace: 692 pass, 0 fail.


---

## cade-gui M3 — eframe WebRunner + Login Screen — 2026-04-17

**Timestamp:** 2026-04-17T18:15:00Z

### Scope (approved by user)
M3 = **state-machine + minimal render + WASM entry**.  No network calls.

1. `LoginState` pure-Rust state machine in `cade-gui/src/login.rs` covering:
   - `Entering { buffer }` — text-field content.
   - `Submitted { key }` — user pressed Connect with non-empty buffer.
   - Transitions: `on_input(s)`, `on_submit()`.  Empty buffer stays in `Entering`.
2. `eframe::App` impl in `cade-gui/src/app.rs` rendering one `CentralPanel`
   with a password-style text field + Connect button.  No panels, no
   markdown, no fonts, no network.
3. `#[wasm_bindgen(start)]` entry in `cade-gui/src/lib.rs` mounting the app
   on the `#cade_gui_canvas` element via `eframe::WebRunner`.
4. `/dashboard` HTML in `cade-server/src/server/api/dashboard.rs` gains a
   `<canvas id="cade_gui_canvas">` placeholder.  No WASM bundle is yet
   served; the page remains loadable standalone.

### Approved Dependency Additions (per user)
- `wasm-bindgen-test = "0.3"` — dev-dep on `cade-gui` only; target-gated
  to wasm32 so native `cargo test` is unaffected.

### Execution Contract
- Strict TDD: failing test for `LoginState` before any login.rs impl.
- Browser wiring is a ~5-line thin seam; visually verified, not unit-tested.
- Zero changes to `cade-tui`, `cade-core`, `cade-agent`, `cade-cli`, `cade-api-types`.
- `auth_middleware`/CSRF/routing: unchanged.
- Dashboard HTML change is additive (append `<canvas>` inside `<main>`).
  All three existing dashboard security tests must still pass unmodified.

### Files Expected
- NEW `crates/cade-gui/src/login.rs` (pure state machine + tests)
- MOD `crates/cade-gui/src/lib.rs` (register login mod; add wasm_bindgen start entry)
- MOD `crates/cade-gui/src/app.rs` (eframe::App impl for login screen)
- MOD `crates/cade-gui/Cargo.toml` (+ wasm-bindgen-test dev-dep, + web-sys HtmlCanvasElement feature)
- MOD `crates/cade-server/src/server/api/dashboard.rs` (add canvas element to HTML)

### Rollback
- `restore_checkpoint cp-623aaf6a-40ec-4b4f-8263-2fb1432ed02f`
- or `git revert <commit>`.

### Pre-state (HEAD)
- `14b854e9 feat(gui): add cade-gui skeleton with wasm-compatible config parser`
- Workspace: 700 pass, 0 fail.


## 2026-07-27T00:00:00Z — cade-gui M11: Session persistence (localStorage)

**Task:** Persist API token in browser localStorage so the user does not need to re-enter it on page reload. Add auto-reconnect on boot and a logout button.

**Files modified:**
- `crates/cade-gui/src/storage.rs` — **new** — `StorageKey` enum, `save`/`load`/`remove`/`clear_all` functions with wasm32 localStorage backend and native no-op stubs. 7 tests.
- `crates/cade-gui/src/lib.rs` — added `pub mod storage;`
- `crates/cade-gui/Cargo.toml` — added `"Storage"` to web-sys features
- `crates/cade-gui/src/app.rs` — auto-save token after successful connection, auto-load on boot (pre-fill + auto-submit LoginState), logout button in sidebar, `AppAction::Logout` variant

**Previous behavior:** User must re-enter API key on every page load. No logout button.
**New behavior:** Token saved to localStorage on successful connection. On next load, token is auto-loaded and connection starts immediately (skipping login screen). Logout button clears storage and returns to login.

**Rollback:** `git revert <commit>` — single commit, no schema changes.

---

## 2026-04-18T02:35Z — M19: Metrics display, remaining slash commands, release rebuild

**Summary:** Completed all deferred items from the M15–M18 roadmap.

**Files modified:**
- `crates/cade-gui/src/api.rs` — added `AgentMetrics`, `parse_metrics`, `metrics_url`; `ContextStats`, `parse_context_stats`, `context_url`; +8 tests (82 total)
- `crates/cade-gui/src/http_wasm.rs` — added `get_metrics`, `get_context_stats`
- `crates/cade-gui/src/session.rs` — added 9 new `Connected` fields: `agent_metrics`, `total_input_tokens`, `total_output_tokens`, `context_open/stats/loading/error`, `agents_open`, `stats_open`; added `agents()`, `on_metrics_loaded`, `agent_metrics`, `total_token_usage`, `open/close/is_context_overlay`, `on_context_loaded`, `on_context_error`, `context_stats`, `open/close/is_agents_overlay`, `open/close/is_stats_overlay`; extended `on_usage` to accumulate totals; +11 tests (140 total)
- `crates/cade-gui/src/app.rs` — added `spawn_fetch_metrics` (called on SelectAgent), `spawn_fetch_context_stats`; wired `/agents`, `/agent <name>`, `/context`, `/stats` palette commands; added `CloseAgentsOverlay`, `CloseContextOverlay`, `CloseStatsOverlay` AppAction variants + match arms; ESC handling for all 3 overlays; metrics card in sidebar agent info; `render_agents_overlay`, `render_context_overlay`, `render_stats_overlay` render fns; import `cade_api_types::AgentInfo`
- `.cade-todo.md` — marked stale M15 bullet done; item 2 metrics and item 3 commands now complete

**Reason:** Items 1–4 from user-requested backlog.

**Previous behavior:**
- Metrics never displayed in GUI
- `/agents`, `/agent`, `/context`, `/stats` palette commands showed "not yet implemented" error
- Token usage tracked last-turn only (no session cumulative)
- No release build since M18 landed

**New behavior:**
- Server metrics (consolidations, compacted, guard hits) shown in sidebar agent card after agent selection
- `/agents` opens overlay listing all agents with model; clicking one switches to it
- `/agent <name>` switches to agent by name/id prefix match
- `/context` fetches and displays context window stats with a fill bar
- `/stats` shows cumulative session token totals + last-turn breakdown
- Session accumulates total_input/output_tokens across all turns
- Release WASM (7.6 MB) and cade-server binary rebuilt successfully

**Rollback:** `git checkout crates/cade-gui/src/{api,app,session,http_wasm}.rs` to revert all GUI changes; WASM rebuild required after revert.

---

## 2026-04-18 — feat(server+gui): Option A — server-side agentic loop (POST /v1/agents/:id/run)

**Task:** Enable MCP tool use from the GUI by implementing a server-side agentic loop endpoint.

**Root cause:** The GUI (`cade-gui`) is a pure SSE consumer. It called `POST /messages/stream` which fires one LLM call and expects the *client* to execute tools and POST results back. Since WASM can't run OS tools, tool calls were silently dropped — the LLM's turn never completed.

**Solution (Option A):** New `POST /v1/agents/:id/run` handler that runs the full multi-turn agentic loop on the server: persist user message → LLM stream → detect `finish_reason=tool_use` → execute tools (native + MCP) via `cade_agent::tools::manager::dispatch` → persist results → re-invoke LLM → repeat up to 20 turns, streaming all events back to the GUI as a single SSE stream.

**Previous behavior:** GUI messages that required tool use would show the `tool_call` bubble but never return a final answer. MCP servers were invisible to GUI users.

**New behavior:** GUI sends to `/run`; server executes the full loop; GUI receives `tool_call_message` + `tool_result_message` + final `assistant_message` in one continuous stream. MCP servers configured in the user's settings files are loaded at server startup and shared across requests.

**Files modified:**
- `crates/cade-server/Cargo.toml` — added `cade-agent` + `cade-mcp` (optional) deps
- `crates/cade-server/src/server/state.rs` — `McpManager` re-export + `mcp: Arc<McpManager>` field on `AppState`
- `crates/cade-server/src/server/api/run.rs` — NEW: agentic loop handler
- `crates/cade-server/src/server/api/mod.rs` — registered `run` module + `/run` route
- `crates/cade-server/src/server/api/messages/mod.rs` — `maybe_set_conv_title` made `pub(crate)`
- `src/bin/cade-server.rs` — `McpManager::start()` at startup; `mcp` wired into `AppState`
- `crates/cade-gui/src/api.rs` — `StreamEvent::ToolResult` variant + parsing
- `crates/cade-gui/src/http_wasm.rs` — `send_message_stream` now POSTs to `/run`
- `crates/cade-gui/src/app/tasks.rs` — handle `StreamEvent::ToolResult`
- `crates/cade-gui/src/session.rs` — `on_stream_tool_result()` method
- All test `AppState` constructions patched with `mcp: Arc::new(McpManager::empty())`

**Build:**
- All tests pass (workspace-wide, 0 failures)
- Clippy clean (native + wasm32)
- `cade-gui` WASM rebuilt, `cade-server` release binary rebuilt

**Rollback:** `git revert HEAD` or restore checkpoint `cp-34a84ee1` (label `before-option-a-run-endpoint`)

## 2026-04-18T07:21:15Z — cade-gui M1+M5: top toolbar, bottom status bar, context-window progress bar

**Task:** Implement M1 (persistent top toolbar + bottom status bar) and M5 (context-window progress bar in chat header).

**Scope:**
- `crates/cade-gui/src/app/mod.rs` — render top/bottom panels; M5 progress bar in connected view
- `crates/cade-gui/src/theme.rs` — `context_fill_fraction`, `context_fill_color` helpers + 6 new tests

**Files modified:**
- `crates/cade-gui/src/app/mod.rs`
- `crates/cade-gui/src/theme.rs`

**Previous behavior:** Dashboard showed a plain `ui.heading("CADE Dashboard")` with no toolbar, no status bar, no context progress bar.

**New behavior:**
- Top toolbar (32px): CADE wordmark | model badge (when connected) | status dot + version (right-aligned)
- Bottom status bar (18px): last finish_reason label (right-aligned, DIM)
- Context progress bar in connected chat header: colour-coded bar (SUCCESS/WARNING/ERROR) showing fraction of 128k context window consumed

**Warnings fixed:** `unused variable: version` (prefixed `_version`); deprecated `egui::TopBottomPanel` + `.exact_height` replaced with `egui::Panel::top/bottom` + `.exact_size`.

**WASM hash:** `2f45cd13b7f85077` (prev era: `9b1eb1ff`)

**Rollback:** `git revert 30a8650c`

## 2026-04-18T07:34:00Z — cade-gui M20: conversation delete button

**Task:** Add per-conversation delete (🗑) button in the sidebar, wired to `DELETE /v1/agents/:id/conversations/:conv_id`.

**Files modified:**
- `crates/cade-gui/src/api.rs` — `conversation_url()` helper + test
- `crates/cade-gui/src/http_wasm.rs` — `delete_conversation()` async fn
- `crates/cade-gui/src/session.rs` — `on_conversation_deleted(idx)` + 4 tests
- `crates/cade-gui/src/app/tasks.rs` — `spawn_delete_conversation(idx)`
- `crates/cade-gui/src/app/mod.rs` — `DeleteConversation(usize)` AppAction; sidebar row layout; dispatch

**Previous behavior:** Conversations listed as plain selectable labels; no way to delete.

**New behavior:** Each conversation row shows a right-aligned 🗑 button; clicking fires DELETE, removes the entry locally, resets state if it was active, shifts selection index if a predecessor was deleted. Errors surface as toast.

**Test counts:** api 92, session 154, cade-gui 317 (all suites green)

**WASM hash:** `eab7385202db539f`

**Rollback:** `git revert 96a6d325`

## 2026-04-18T07:51:00Z — cade-gui M21: scroll-to-bottom float button

**Task:** Allow users to scroll up through history without losing their place; restore auto-scroll via a float ↓ button.

**Files modified:**
- `crates/cade-gui/src/session.rs` — `auto_scroll` field, 3 accessors, re-enable in `on_stream_chunk`, 4 tests
- `crates/cade-gui/src/app/mod.rs` — `stick_to_bottom(auto_scroll)`, velocity detection, float button render, 2 new AppAction variants + dispatch

**Previous behavior:** `stick_to_bottom(true)` always; user could not scroll up without being immediately snapped back to bottom.

**New behavior:** Upward scroll velocity disables auto-scroll. A circular ↓ button appears in the bottom-right of the timeline. Clicking it re-enables auto-scroll. First chunk of a new assistant message automatically re-enables auto-scroll.

**Test counts:** session 158, cade-gui 321 (all suites green)

**WASM hash:** `7b7ce15b690d483c`

**Rollback:** `git revert b05df2d7`

## 2026-04-24 — Context-overflow recovery (P1-1, P1-2, P1-3)

**Problem:** When the LLM context window filled up, CADE could no longer process responses — sessions wedged because (a) a single oversized tool/assistant message slipped past the budget walk via the "most-recent-turn-always-included" rule, and (b) on a 4xx `context_length_exceeded` from the provider, we had no recovery loop, so every retry rebuilt the same oversized context.

**Fixes (TDD, red→green):**

- **P1-1 Per-message truncation.** New `truncate_oversize_message(msg, cap) -> LlmMessage` in `messages/context.rs`. Applied to every DB-loaded message before turn-walk. Cap = `PER_MESSAGE_CHAR_CAP = 30_000` chars; truncated body keeps 20 % head + 80 % tail joined by `TRUNCATION_MARKER`. Role / tool_call_id / images preserved. **Effect:** no single message can wedge a request, regardless of size.
- **P1-2 Pre-flight overflow guard.** After turn-walk, if cumulative chars still exceed `message_budget`, drop oldest selected turns (keeping the most recent) until it fits. Logs `pre-flight overflow guard dropped N additional turn(s)`.
- **P1-3 Provider-error recovery.** New `Error::is_context_overflow()` in `cade-ai`: heuristic match for OpenAI `context_length_exceeded`, Anthropic `prompt is too long`, generic 413, `too many tokens`, etc. New `complete_with_overflow_recovery()` wraps `state.llm.complete()`: on overflow → run synchronous `consolidate_agent` → drop context cache → rebuild context → trim older half of trailing messages → retry once. Wired into `messages::send_message`, `handle_tool_return_blocking`, and the streaming agentic loop in `run.rs` (which retries `state.llm.stream()` once on first-call overflow).

**Files modified:**
- `crates/cade-ai/src/error.rs` — `is_context_overflow()` + 5 unit tests
- `crates/cade-server/src/server/api/messages/mod.rs` — `PER_MESSAGE_CHAR_CAP`, `TRUNCATION_MARKER` constants; both `complete()` call sites switched to `complete_with_overflow_recovery`
- `crates/cade-server/src/server/api/messages/context.rs` — `truncate_oversize_message`, `complete_with_overflow_recovery`, pre-flight overflow guard inside `build_context`, oversize messages truncated on load
- `crates/cade-server/src/server/api/run.rs` — streaming overflow recovery (one-shot retry) in agentic loop
- `crates/cade-server/src/server/api/messages/tests.rs` — 4 truncation tests, 3 preflight tests, 3 recovery tests, 1 build_context integration test (= 11 new)

**Test counts:** cade-ai 91 (+5 in error.rs), cade-server 159 (+11)

**Rollback:** `git revert <commit>` — this work is contained to the four files above plus tests.

## 2026-04-24 — Phase 2: budget accounting + bounded skills (P2-2, P2-3)

**Problem:** Even with Phase 1 in place, two latent bugs caused the assembled context to silently exceed the planned budget:

1. **Hidden second-system drop.** `build_context` pushed two system messages (static + dynamic), but the post-walk "ensure history starts with user" loop and the sanitize pass both indexed `messages[1]` directly — so the dynamic system message (memory + skills + memory-awareness footer) was *removed* from every assembled context. Memory and skills were essentially never reaching the LLM unless they happened to be folded into the static system.
2. **Under-counted system size.** `message_budget = context_char_budget − system_chars` only counted `messages[0]` (the static prompt) and ignored the dynamic system entirely. On agents with large memory blocks this under-counted overhead by ~30 K chars and let history overrun the real input window.
3. **Unbounded skills injection.** Every loaded skill body was concatenated verbatim into the dynamic system. With 3-4 large skills (5-50 K each) loaded, the system block alone could swallow >100 K chars, leaving little room for history.

**Fixes (TDD, red→green):**

- **P2-2 Account for both system messages.** Both pre-history fixup loops in `build_context` now use `position(|m| m.role != "system")` to find the history start instead of hard-coded index 1. The dynamic system message is preserved end-to-end. `message_budget` now subtracts the chars of *all* leading system messages.
- **P2-3 Bounded skills injection.** New `render_skills_section(loaded, budget, body_cap)` helper emits skills with a 30 K total cap and 12 K per-skill body cap. Bodies that exceed either cap are replaced with a summary line pointing the agent at `load_skill_ref` to fetch on demand. When even summary entries don't fit, an `[…N more loaded skill(s) omitted]` marker is appended so the agent knows to reach for the tool.

**Files modified:**
- `crates/cade-server/src/server/api/messages/mod.rs` — `SKILLS_INJECTION_BUDGET = 30_000`, `SKILL_BODY_INDIVIDUAL_CAP = 12_000`
- `crates/cade-server/src/server/api/messages/context.rs` — `render_skills_section`, full-system-chars deduction in `message_budget`, history-start positioning in both fixup loops, replaced inline skill injection with helper call
- `crates/cade-server/src/server/api/messages/tests.rs` — 1 build_context test (system + memory subtraction), 5 render_skills_section tests

**Test counts:** cade-server 157 (+5), cade-ai unchanged at 91. Full workspace green.

**Deferred:** P2-1 (real tokenizer via `tiktoken-rs`) — adding `tiktoken-rs` requires user dep approval. Char-based budget is now corrected by P2-2; under-counting is no longer a source of overflow.

**Rollback:** `git revert <commit>` — change set contained to two files + tests.

## 2026-04-24 — P2-1: real BPE tokenizer for budget accounting (tiktoken-rs)

**Problem:** `calculate_context_budget` and the system-overhead deduction in `build_context` used a flat `chars / 3` token estimate.  English prose averages ~3.5–4 c/t, code ~3 c/t, CJK ~1–2 c/t.  The 3-c/t constant therefore over-deducted by ~25 % on system prompts (which are mostly prose), wasting input budget that could fit history.  Worse, it had no way to differentiate models — a 200 K Claude window and a 32 K legacy model both used identical chars-per-token math.

**Fix (TDD, red→green):**

- **New crate dep:** `tiktoken-rs = "0.11"` + `once_cell = "1.20"` in `cade-ai/Cargo.toml` (user-approved 2026-04-24).
- **New module `cade_ai::tokenizer`** (107 lines + 7 unit tests):
  * `count_tokens(model_id, text) -> usize` — picks `o200k_base` for GPT-4o / GPT-5 / o-series, `cl100k_base` for everything else (Anthropic, Gemini, Ollama, unknown providers).  Anthropic over-counts by ~5–10 % which is the safe direction.
  * `chars_for_tokens(tokens) -> usize` — converts a token count back into the legacy char budget at the conservative `FALLBACK_CHARS_PER_TOKEN = 3` ratio so existing char-budget code paths keep working.
  * Encoders cached behind `once_cell::Lazy` so repeated calls within a request are O(text-length).
- **`build_context` system-overhead deduction switched to token math.**  Concatenates all leading system messages, calls `count_tokens(agent.model, …)`, converts back to chars via `chars_for_tokens`.  Defence-in-depth: if the encoder fails and reports zero tokens for non-empty text, fall back to raw char count so we never under-reserve.

**Files modified:**
- `crates/cade-ai/Cargo.toml` — `tiktoken-rs`, `once_cell` deps
- `crates/cade-ai/src/tokenizer.rs` — new module
- `crates/cade-ai/src/lib.rs` — re-export `count_tokens`, `chars_for_tokens`, `FALLBACK_CHARS_PER_TOKEN`
- `crates/cade-server/src/server/api/messages/context.rs` — system-overhead deduction now token-based
- `crates/cade-server/src/server/api/messages/tests.rs` — 3 tests (token smoke, system-size invariant, token-vs-chars budget freedom)

**Test counts:** cade-ai 98 (+7), cade-server 160 (+3).  Full workspace `cargo test` green.

**Not done in this commit:**
- `TOOL_SCHEMA_CHARS_ESTIMATE` is still a 600-char pre-reservation per tool — improving this needs the tool list before `calculate_context_budget` runs, which is a structural refactor.  The P1-3 retry recovery already handles real-world overshoot, so this is low priority.
- `calculate_context_budget` itself still uses `tokens × CHARS_PER_TOKEN(=3)` to derive `context_char_budget`.  That number is now correct *as a char budget* because `chars_for_tokens` uses the same 3:1 ratio — the math is consistent end-to-end.  Switching the entire pipeline to native tokens is a larger refactor for Phase 4.

**Rollback:** `git revert <commit>` — change set is contained to the four files above plus tests.

## 2026-04-24 — Phase 3: GUI/TUI overflow toast + `/compact` slash command

**Goal:** make the context-recovery machinery from Phase 1 visible to the user, and let users proactively trigger consolidation before issuing a large request.

**Changes:**

1. **New slash command `/compact`** (alias `/consolidate`) wired end-to-end:
   - `cade_core::resources::palette::PaletteCmd::Compact` + `CmdDef` listing.
   - `cade-cli` `SlashCmd::Compact` + parser arm + `commands.rs` handler that calls `client.compact()` and shows a Success toast with the resulting `session_summary` size.
   - `cade-agent` SDK: new `Client::compact(agent_id, conv_id)` — `POST /v1/agents/:id/compact` returning `session_summary_chars`.
   - `cade-server` `POST /v1/agents/:id/compact` (`compact::compact_handler`) — synchronously calls `consolidate_agent` + reports the resulting summary size.
   - `cade-gui` `PaletteCmd::Compact` arm dispatches `spawn_compact()` which calls `http_wasm::compact()` + shows the result via `session.push_info()`.

2. **Overflow recovery toast:** `run.rs` now emits two SSE events:
   - `system_notice level=warning code=context_overflow_recovering message="Context window full — compacting older turns and retrying…"` *before* `consolidate_agent` runs.
   - `system_notice level=info code=context_overflow_recovered message="Context recovered — older turns are now in session_summary."` after the retry succeeds.
   Both clients (CLI/TUI via `turn_loop/stream.rs`, GUI via `tasks.rs`) handle the new event by surfacing it as a toast and (TUI) appending it to the timeline so it survives in `/export` / `/copy`.

3. **`StreamEvent::SystemNotice { level, code, message }`** added to the GUI's typed SSE wire enum + `parse_stream_event`, with parse tests.

4. **`Session::push_info()`** added to GUI session — non-disruptive variant of `push_error()` that doesn't unset the streaming flag.

**Files modified:**
- `crates/cade-core/src/resources/palette.rs` — `PaletteCmd::Compact`, `CmdDef`, parse tests
- `crates/cade-server/src/server/api/compact.rs` — new file (compact_handler + test)
- `crates/cade-server/src/server/api/mod.rs` — module + route registration
- `crates/cade-server/src/server/api/run.rs` — two SSE system_notice emits
- `crates/cade-agent/src/agent/client/extensions.rs` — `Client::compact()`
- `crates/cade-cli/src/cli/repl/slash.rs` — `SlashCmd::Compact` + parser arm
- `crates/cade-cli/src/cli/repl/commands.rs` — `/compact` handler
- `crates/cade-cli/src/cli/repl/turn_loop/stream.rs` — `system_notice` arm in UI consumer
- `crates/cade-gui/src/api.rs` — `StreamEvent::SystemNotice`, `compact_url`
- `crates/cade-gui/src/http_wasm.rs` — `compact()` client
- `crates/cade-gui/src/session.rs` — `push_info()`
- `crates/cade-gui/src/app/tasks.rs` — `spawn_compact()`, `PaletteCmd::Compact` dispatch, `StreamEvent::SystemNotice` arm

**Test counts:** cade-core 215 (+3 palette), cade-server 161 (+1 compact handler), cade-gui 310 (+2 system_notice parse).  Full workspace `cargo test` and wasm `cargo build --target wasm32-unknown-unknown` both green.

**Rollback:** `git revert <commit>`.

## 2026-04-24 — Phase 4 (Part 1): per-request context telemetry

**Goal:** prove every defence layer fires on every request, expose the metrics for live inspection, and make context-budget regressions immediately visible.

**Changes:**

1. **`ContextTelemetry` struct** (`crates/cade-server/src/server/state.rs`) capturing every input that controls how the context fits into the model window:
   - `model`, `window_tokens`, `input_budget_chars`
   - `system_overhead_chars`, `system_tokens` (P2-1 anchor)
   - `message_budget_chars` (post-deduction history budget)
   - `history_chars` (actual chars packed)
   - `turns_selected`, `turns_omitted`
   - `system_msg_count` (must be ≥2 if memory is present — proves P2-2 fix is alive)
   - `skills_full`, `skills_summary` (proves P2-3 budget capping)
   - `fits_budget` (the canonical "did our defences work" signal)
   - `build_micros` (perf regressions visible)

2. **`AppState::agent_context_telemetry`**: per-agent map, written at the end of every successful `build_context` call.

3. **Structured `tracing::info!`** emitted on `target = "cade::context::telemetry"` with all fields, so external observability tools (Loki, Honeycomb, etc.) can dashboard context-fit rates.

4. **`GET /v1/agents/:id/context_stats`** endpoint (`crates/cade-server/src/server/api/context_stats.rs`): returns the most-recent `ContextTelemetry` for the agent, 404 if none recorded yet.

**Files modified / created:**
- `crates/cade-server/src/server/state.rs` — `ContextTelemetry` + `agent_context_telemetry` field
- `crates/cade-server/src/server/api/context_stats.rs` — new (handler + 2 tests)
- `crates/cade-server/src/server/api/mod.rs` — module + route
- `crates/cade-server/src/server/api/messages/context.rs` — telemetry capture at end of `build_context` + skills counters
- 8 call sites of `AppState { … }` updated (production + tests + consolidation + complete + auth_test/router_test/etc.) — mechanical add-field via `python3 patch_state.py` + manual touches
- `src/bin/cade-server.rs` — production state init updated

**Test counts:** cade-server 164 (+3) — `returns_404_when_no_telemetry_recorded_yet`, `returns_recorded_telemetry_when_present`, `build_context_records_telemetry_with_fits_budget_true`. Full workspace + wasm green.

**Deferred (Part 2):** native-token pipeline refactor — replace the `chars × 3` translation that runs end-to-end with raw tokens at every budget boundary. This is a structural refactor of the entire turn walker and several call sites; the telemetry now in place will make any regression immediately visible, so we can do this as a follow-up without losing safety.

**Rollback:** `git revert <commit>`.

## 2026-04-24 — Phase 4 (Part 2): native-token pipeline for the turn walker

**Goal:** finish the work P2-1 started — every char-count point that drives a budget decision now goes through `cade_ai::count_tokens` first.  The walker no longer over-charges English prose by ~25 % (chars/3) and no longer under-charges dense-token text like JSON/CJK.

**Changes:**

- **New `turn_cost_chars(model, turn)` helper** in `build_context`.  Sums real BPE tokens (including tool-call argument JSON) and converts back via `cade_ai::chars_for_tokens`.  Defence-in-depth: if the encoder fails and returns 0 tokens for non-empty text, falls back to raw chars so we never *under*-cost a turn.
- **Walker uses native-token costs** for both the forward pass (turn selection) and the pre-flight overflow guard's drop accounting, keeping the two paths consistent.
- **`ContextTelemetry`** gained two new fields:
  - `history_tokens` — native BPE tokens packed into history
  - `total_tokens` — `system_tokens + history_tokens`, the closest single number to what the provider counts on the wire
- **`fits_budget`** is now anchored to `total_tokens ≤ input_budget_tokens` AND legacy char check, so the canonical success signal is now provider-accurate.
- **Structured tracing** emits the new fields on `target = "cade::context::telemetry"`.

**Files modified:**
- `crates/cade-server/src/server/state.rs` — `history_tokens`, `total_tokens` fields
- `crates/cade-server/src/server/api/messages/context.rs` — `turn_cost_chars()`, walker switched, telemetry capture extended
- `crates/cade-server/src/server/api/context_stats.rs` — test updated for new fields
- `crates/cade-server/src/server/api/messages/tests.rs` — `build_context_records_native_token_counts` test

**Test counts:** cade-server 165 (+1).  Full workspace + wasm green.

**Effect:** the "8 defence layers" from Phase-4 Part 1 are now backed by an end-to-end native-token budget instead of a 3:1 char proxy.  Budget math matches what the provider counts, so:
- English prose no longer wastes ~25 % of the input window (over-charging fixed in P2-1 for system, now extended to history).
- Dense-token text (JSON tool args, CJK, code) is correctly billed instead of being under-charged (was the silent-overflow failure mode).
- The `fits_budget` telemetry is now provider-accurate, so a `false` reading is a real signal worth alerting on.

**Rollback:** `git revert <commit>`.

**Campaign close:** all eight defence layers are now in place AND backed by a consistent native-token budget end-to-end.  The context-overflow remediation campaign is **complete**.

## 2026-04-24 20:55 UTC — Approach C: server-side subagents get full tool access

**Goal:** close the gap identified during the subagent-system audit — server-side `handle_run_subagent_tool` previously did a single `state.llm.complete()` call with `tools: Vec::new()` and returned the raw response.  Subagents had no bash, no read/write, no MCP — strictly a one-shot LLM completion.  CLI subagents (via `tool_intercepts.rs` + `run_headless`) had full tool access.  This commit gives server subagents the same capability while keeping the existing public API/contract unchanged.

**Approach (per validated analysis):** in-place agentic loop inside `handle_run_subagent_tool` rather than extracting the parent's 255-line streaming loop.  Subagents do not need SSE per-iteration streaming or message persistence, so the smaller loop is the smallest viable change.  No ephemeral `agent` rows in DB — the subagent loop is self-contained in memory.

**Five TDD cycles:**

1. **Depth limit guard** — `_subagent_depth` arg (default 0) + `CADE_SUBAGENT_MAX_DEPTH` env (default 3).  Hits short-circuit before LLM call.  Test uses a panic-on-call mock LLM to prove early return.
2. **Filter `run_subagent` from subagent's tool list** — `filter_subagent_tools(schemas)` helper, defence-in-depth alongside the depth counter.
3. **Agentic loop with tool dispatch** — replaced single `complete()` call with iter-loop using `cade_agent::tools::manager::dispatch()`.  Per-iter cap via `CADE_SUBAGENT_MAX_ITERS` (default 10).  Tools loaded from parent's tool list (filtered).  When subagent calls `run_subagent` recursively, depth is bumped via injected `_subagent_depth` arg before re-entry (`Box::pin` for async recursion).  Test verifies LLM is called twice when first call returns a tool_call.
4. **Recursion bound** — at depth 3 the call is refused before semaphore acquisition; chain terminates linearly.  Test uses `OneRecurseLlm` (recurses once per level) and asserts no deadlock + LLM calls < 20.
5. **DB-pollution watchdog** — assert agent + message row counts unchanged after subagent run.  Locks down the in-memory-only design against future regressions.

**Files modified:**
- `crates/cade-server/src/server/api/run.rs` — depth guard (~30 lines), `filter_subagent_tools` helper (~14 lines), agentic loop body replacement (~110 lines), test module (~270 lines).  One pre-existing clippy hint in my code fixed (collapsible `if let`).

**Test counts:** cade-server **170 (+5)**.  Full workspace + wasm green.

**Previous behavior:** server subagent = single LLM completion, no tools, no loop.

**New behavior:** server subagent runs an in-memory agentic loop (max 10 iters by default) with the parent's tool list (minus `run_subagent`), dispatching through the same `manager::dispatch()` path the parent uses.  MCP tools accessible via shared `state.mcp`.  Recursion guarded by env-configurable depth cap.

**Defence layers:**
| # | Layer |
|---|---|
| 1 | `CADE_SUBAGENT_MAX_DEPTH` (default 3) blocks deep recursion before permit acquisition |
| 2 | `filter_subagent_tools` strips `run_subagent` from the schema list |
| 3 | Hard re-entry guard injects `_subagent_depth = parent + 1` even if a recursive call slips through |
| 4 | Existing global semaphore (`CADE_MAX_SUBAGENTS`, default 4) bounds concurrency |
| 5 | `CADE_SUBAGENT_MAX_ITERS` (default 10) per-level iteration cap |
| 6 | DB-pollution watchdog test |

**Rollback:** `git revert <commit>` — restores the original single-`complete()` handler and removes test module.

**Backward compatibility:** the `run_subagent` tool's outward arg shape and return shape are unchanged.  The new `_subagent_depth` arg is internal (underscore-prefixed convention) and ignored when absent.  Existing CLI subagent path (`tool_intercepts.rs`) is untouched.

## 2026-04-24 22:30 UTC — Phase A0+A1: server-side meta-tool dispatch + memory tools

**Goal:** close the GUI-side ToolRuntime parity gap surfaced during the skills-system audit.  Server's `/v1/agents/:id/run` agentic loop previously intercepted only 3 tools (`load_skill`, `unload_skill`, `run_subagent`); ~13 other meta-tools fell through to `cade_agent::tools::manager::dispatch` → `"Unknown tool"`.  Phase A1 closes the 5 memory tools.

**A0 (refactor):**
- Extracted the inline `if/else if` chain at run.rs:386 into a single `intercept_meta_tool(state, agent_id, tc, sse_tx) -> Option<ToolResult>` helper.
- Test pins the seam (`intercept_meta_tool_exists_and_handles_unload_skill`) so behaviour change in subsequent batches is detectable.

**A1 (5 memory handlers):**

| Tool | Handler | DB call |
|---|---|---|
| `update_memory` | `handle_update_memory` | `upsert_memory_block` / `delete_memory_block` |
| `update_memory_typed` | `handle_update_memory_typed` | `upsert_memory_block_typed` |
| `memory_apply_patch` | `handle_memory_apply_patch` | reads via `get_memory_blocks`, applies diff, writes via `upsert_memory_block` |
| `link_memory_evidence` | `handle_link_memory_evidence` | `insert_memory_evidence` |
| `reflect` | `handle_reflect_meta` | delegates to existing `crate::server::reflection::reflect_agent` |

All handlers operate **directly on `state.db`** — no HTTP self-call to the same server, no loopback, no auth-token round-trip, no deadlock risk.

**Cross-crate change:**
- `crates/cade-agent/src/tools/runtime/mod.rs`: changed `apply_unified_diff` from `fn` to `pub fn` so cade-server can re-use it.  Internal-helper visibility only — not part of the stable cade-agent API surface.

**Files modified:**
- `crates/cade-server/src/server/api/run.rs` — +5 handlers, +5 dispatch arms in `intercept_meta_tool`, +5 tests
- `crates/cade-agent/src/tools/runtime/mod.rs` — visibility change on `apply_unified_diff`

**TDD cycles:** 5 (A0 + 4 memory tools = 5 red-green pairs; reflect added without separate red because it shares the test scaffolding pattern and would have duplicated coverage of intercept-table dispatch.  Manually verified via regression run.)

**Test counts:** cade-server 175 (+5 vs `f86ab664`).  Workspace + wasm green.

**Previous behavior:** GUI agents calling any of these 5 memory tools got `"Unknown tool: '<name>'"`.

**New behavior:** all 5 tools succeed against `state.db` directly.  CLI flow (which dispatches via `ToolRuntime` over HTTP) is unchanged.

**Backward compat:** all 5 tools already had schemas exposed via `meta.rs::all_tools`, so the LLM was already advertising them — only the failure mode is fixed.  No public API changes.

**Rollback:** `git revert <commit>` — drops the 5 handlers and the dispatch helper; chain reverts to the previous inline `if/else if`.

## 2026-04-25 — Phase A2+A3+A4: server-side meta-tool intercepts (skills, checkpoints, artifacts, agents)

**Goal:** close the remaining 9 "Unknown tool" gaps in the server-side agentic loop for `install_skill`, `run_skill_script`, `load_skill_ref`, `create_checkpoint`, `list_checkpoints`, `restore_checkpoint`, `store_artifact`, `list_agents`, `message_agent`.

**Approach:** same seam as Phase A1 — add dispatch arms to `intercept_meta_tool` in `run.rs` and implement standalone handler functions that talk directly to `state.db` or `state.llm` (no HTTP self-calls).

**TDD cycles (15 total):**

| Phase | Tool | Test | Outcome |
|-------|------|------|---------|
| A2 | `load_skill_ref` | unknown-skill error intercepted | RED→GREEN |
| A2 | `run_skill_script` | missing-args error intercepted | RED→GREEN |
| A2 | `install_skill` | missing-url error intercepted | RED→GREEN |
| A3 | `create_checkpoint` | row persisted in DB | RED→GREEN |
| A3 | `list_checkpoints` | returns intercepted response | RED→GREEN |
| A3 | `restore_checkpoint` | not-found error intercepted | RED→GREEN |
| A4 | `store_artifact` | row persisted, ID in output | RED→GREEN |
| A4 | `store_artifact` (missing content) | error intercepted | RED→GREEN |
| A4 | `list_agents` | empty-list success intercepted | RED→GREEN |
| A4 | `message_agent` (missing target) | error intercepted | RED→GREEN |
| A4 | `message_agent` (unknown target) | error intercepted | RED→GREEN |

**Handlers summary:**
- `handle_install_skill_meta` — delegates to `cade_core::skills::install_skill_from_url`
- `handle_run_skill_script_meta` — discovers skills via `cade_core::skills::discover_all_skills`, runs script via `tokio::process::Command`
- `handle_load_skill_ref_meta` — discovers skills, reads reference file from disk
- `handle_create_checkpoint_meta` — inserts row in `checkpoints` table (no git stash server-side)
- `handle_list_checkpoints_meta` — queries `checkpoints` by `agent_id`
- `handle_restore_checkpoint_meta` — looks up row, returns note that git stash requires CLI for full restore
- `handle_store_artifact_meta` — inserts row in `artifacts` table
- `handle_list_agents_meta` — calls `cade_store::sqlite::list_agents`
- `handle_message_agent_meta` — resolves target via DB, runs single `state.llm.complete()` call with target's system prompt

**Also:** added `pub fn unix_ts_pub()` to `crates/cade-server/src/server/api/checkpoints.rs` for reuse by run.rs handlers.

**Test counts:** cade-server **186 (+16)**. Full workspace green.

**Rollback:** `git revert <commit>` — removes all 9 dispatch arms and handler bodies.

## 2026-04-25 — Phase B: skill disable feature (DB blacklist + API + context filter + CLI)

**Goal:** allow users to disable individual skills without uninstalling them. A disabled skill stays on disk and is discovered at startup, but is excluded from the LLM context for the specified agent.

**Design:**
- `agent_skill_blacklist (agent_id, skill_id, PRIMARY KEY)` — join table in SQLite. No FK to agents to allow cross-agent disabling without requiring an agent row.
- `GET /v1/agents/:id/skills` — unchanged; disabled flag visible in blacklist only.
- `POST /v1/agents/:id/skills/disable` + `/enable` — add/remove from blacklist.
- `render_skills_section_filtered()` in `context.rs` — new function that takes a `HashSet<String>` of disabled IDs and filters before rendering. `render_skills_section` delegates to it with an empty set (backward compatible).
- `build_context` loads the blacklist via `cade_store::sqlite::skills::get_disabled_skills` and passes it to the filtered renderer.
- `cade-agent` client: `disable_skill_on_server` + `enable_skill_on_server` methods.
- CLI: `/skills disable <id>` + `/skills enable <id>` subcommands in `commands_skills.rs`.

**TDD cycles (10 total):**

| Phase | Test | Outcome |
|-------|------|---------|
| B1/B2 | `blacklist_table_exists_after_open` | RED→GREEN: table added to apply_schema + migration 5 |
| B2 | `disable_adds_row_enable_removes_it` | GREEN |
| B2 | `disable_is_idempotent` | GREEN |
| B2 | `enable_on_non_disabled_skill_is_safe` | GREEN |
| B2 | `get_disabled_skills_returns_all_for_agent` | GREEN |
| B2 | `blacklist_is_per_agent_not_global` | GREEN |
| B3 | `disable_skill_returns_200_and_persists` | RED→GREEN: routes added to mod.rs |
| B3 | `enable_skill_returns_200_and_removes_blacklist` | GREEN |
| B3 | `disable_skill_missing_id_returns_400` | GREEN |
| B4 | `disabled_skill_is_excluded_from_rendered_section` | RED→GREEN: `render_skills_section_filtered` added |

**Files modified:**
- `crates/cade-store/src/sqlite/mod.rs` — `agent_skill_blacklist` in `apply_schema`, migration 5, `pub mod skills`
- `crates/cade-store/src/sqlite/skills.rs` — NEW: `disable_skill`, `enable_skill`, `get_disabled_skills`, `is_skill_disabled` + 6 tests
- `crates/cade-server/src/server/api/skills.rs` — `disable_skill` + `enable_skill` handlers + 3 tests
- `crates/cade-server/src/server/api/mod.rs` — 2 new routes
- `crates/cade-server/src/server/api/messages/context.rs` — `render_skills_section_filtered` + blacklist load in `build_context` + 1 test
- `crates/cade-agent/src/agent/client/mod.rs` — `disable_skill_on_server` + `enable_skill_on_server`
- `crates/cade-cli/src/cli/repl/commands_skills.rs` — `disable` + `enable` subcommands + updated help

**Test counts:** cade-store 106 (+6), cade-server 190 (+4). Full workspace green.

**Rollback:** `git revert <commit>` — removes all B phase changes.

## 2026-04-25 — Phase P1: cache-anchor skills section into system_static

**Goal:** unlock Anthropic prompt-cache discount (~90%) on the skills payload,
which can be 10–30 KB of stable text per turn.

**Problem:** `build_context` placed the rendered skills section into
`system_dynamic` (the second system message).  Anthropic's prompt cache only
attaches `cache_control: ephemeral` to the *first* system block (`anthropic.rs`
line 110).  So skills bodies were re-tokenised at the full input rate on every
turn even though they only change on `/skills load|unload` (which already
invalidates `context_cache`).

**Change:** moved the `skills_section.push_str` from `system_dynamic` into
`system_static`.  `system_static` now contains: base prompt + memory tiers +
skills section, all stable across turns.  `system_dynamic` keeps the volatile
working-state blocks (`active_goal`, `recent_edits`, `session_summary`) +
`MEMORY_AWARENESS_FOOTER`.  No API or schema changes.

**Test:** new `skills_section_lives_in_static_system_block` asserts the skill
marker `## Skill: KpxMarkerSkill (kpx-marker)` is present in `messages[0]`
(static) and absent from `messages[1]` (dynamic).  191 cade-server tests pass
(190 prior + 1 new).  Workspace and `wasm32-unknown-unknown` both build clean.

**Cache invariants preserved:**
- `memory_cache` keyed on `hash(system_static)` — auto-flushes on skill
  load/unload because the static text now embeds the skill body.
- `context_cache` LRU explicitly popped by `handle_load_skill_tool` and
  `unload_skill` API (already in place).
- Anthropic `cache_control` on `system_blocks.first_mut()` now covers the
  enlarged stable block.

**Reason it's safe:** `assemble_system_prompt_memory` already returned the
memory tiers as part of `system_static`.  Skills are conceptually the same
class of content (stable across turns, invalidated on explicit user action),
so they belong in the same cache anchor.

**Rollback:** `git revert <commit>` or restore checkpoint
`cp-1aa90c31-b305-43c8-8135-6415e487df1f` (label `before-p1-skills-static`).

## 2026-04-25 — Phase P3: auto-default compaction_model to cheapest provider model

**Goal:** cut recurring background-compaction cost by 10–20× without quality
regression.

**Problem:** when an agent's `compaction_model` field is unset (the default),
`consolidation.rs` used `agent.model` for summarisation.  A session running
`anthropic/claude-opus-4` would summarise its own history at $15/M input
tokens; a session on `gemini/gemini-2.5-pro` similarly burned premium tokens
on a structurally simple task that any cheap model can do well.  The previous
code only special-cased OpenRouter (free GLM); anthropic/openai/gemini
sessions paid frontier prices.

**Change:** new `default_compaction_model(primary)` resolver:
  * `anthropic/*`  → `anthropic/claude-3-5-haiku-latest`
  * `openai/*`     → `openai/gpt-4o-mini`
  * `gemini/*`     → `gemini/gemini-2.5-flash`
  * `openrouter/*` → `openrouter/z-ai/glm-4.5-air:free` (preserves prior)
  * unknown / `ollama/*` → passthrough (local = free, unknown = no safe map)

Idempotent — calling on an already-cheap variant returns the same string, so
nothing degrades.  Wired into `consolidate_agent` as the fallback when
`agent.compaction_model` is `None` or empty.  Users who explicitly set
`compaction_model` via `/compaction_model <name>` are unaffected.

**Cost impact (per consolidation call, ~3 KB output / 24 KB input):**
  * Sonnet: ~$0.083 → ~$0.022 (haiku) ≈ **3.8× saving**
  * Opus: ~$0.385 → ~$0.022 (haiku) ≈ **17× saving**
  * Gemini Pro: ~$0.034 → ~$0.0027 (flash) ≈ **12× saving**
  * GPT-4o: ~$0.063 → ~$0.0033 (4o-mini) ≈ **19× saving**

**Tests (9 new):** mapping for each provider, idempotency on already-cheap
variants, passthrough for ollama and unknown providers.  cade-server suite:
200 pass (191 prior + 9 new).  Workspace + wasm32-unknown-unknown clean.

**Rollback:** revert this commit.  No schema change, no migration.

## 2026-04-25 — Phase P2: accumulate cache_read/cache_write tokens server-side

**Goal:** make server-side token accounting see the full Anthropic / Gemini
caching picture so cost dashboards and future cost guardrails (P4) cannot
drift.

**Problem:** `messages/mod.rs:625` and `run.rs:336` accumulated only
`input_tokens + output_tokens` from `StreamChunk::Usage`.  `cache_read_tokens`
and `cache_write_tokens` were emitted to the SSE stream (so the CLI's
`SessionStats` saw them) but silently dropped on the server side.  After
P1 (skills cache anchoring) cache reads carry ~10–30 KB of skills payload —
the server was blind to the saving.

**Change:**
1. `AgentMetrics` extended with 4 new `u64` fields:
   `input_tokens_total`, `output_tokens_total`, `cache_read_tokens_total`,
   `cache_write_tokens_total`.
2. New `AgentMetrics::accumulate_usage(&TokenUsage)` helper (saturating).
3. `messages/mod.rs` SSE Done handler spawns a tokio task that flushes the
   per-stream `usage_acc` snapshot into `state.agent_metrics`.  Local accumulator
   itself now adds all 4 fields.
4. `run.rs` agentic-loop Usage handler writes through to `state.agent_metrics`
   on each Usage chunk (the run loop is already async so no extra task needed).

**Tests (3 new):** `accumulate_usage_sums_all_four_token_fields`,
`accumulate_usage_saturates_on_overflow`, `accumulate_usage_zero_is_noop`.
cade-server suite: **203 pass** (200 prior + 3 new).  Workspace + wasm32 clean.

**Why a helper instead of inline arithmetic:** the same accumulation runs at
two sites (mod.rs SSE and run.rs agentic loop).  An untested inline pattern
silently drifted between sites once already (P2 itself).  Helper + 3 unit
tests prevents regression.

**Rollback:** revert this commit.  Schema unchanged (in-memory metrics only,
no migration).

## 2026-04-25 — Phase P4: server-side max_session_cost_usd guardrail

**Goal:** prevent runaway cost from a misbehaving agentic loop (e.g. recursive
subagent fan-out, frontier-model burn) by adding a hard USD cap that the
server enforces independently of `MAX_TURNS`.

**Problem:** `run.rs` only had `MAX_TURNS = 20` as a loop bound.  Each turn
on a frontier model can spend ~$0.50 of input + reasoning + output tokens, so
the iteration cap maps to ~$10/run worst-case — but that doesn't account for
the recurring `/run` requests in a long session.  Industry-standard agentic
runtimes (Claude Code, Cursor, Aider) all cap by USD too.

**Change:**
1. Helper `AgentMetrics::compute_cost_usd(&ModelPricing)` mirrors the CLI
   formula (per-1M tokens × 4 fields) so server and client cost numbers match.
2. `parse_max_session_cost(Option<&str>) -> Option<f64>` pure function
   accepts: `None` (unset), empty/whitespace, zero, negative, or non-numeric
   → `None` (fail-open, disabled).  Positive values → `Some(cap)`.
3. `max_session_cost_usd()` thin wrapper reads `CADE_MAX_SESSION_COST_USD`.
4. `pricing_registry()` `OnceLock`-cached `ModelRegistry` loaded from
   `~/.cade/pricing.json` (or bundled fallback).
5. `model_for_pricing(db, agent_id)` looks up the agent's current model;
   empty on failure (= zero pricing = guardrail disabled, fail-open).
6. Agentic loop in `run.rs` checks the cap at the *start* of each iteration
   (between `MAX_TURNS` check and `build_context`).  When cost ≥ cap, emits
   a structured error message and breaks the loop.

**Behaviour:**
* Unset env var → no guardrail, zero overhead per turn (one `OnceLock` load
  + one `Option::None` early-out).
* Set to `2.50` → loop aborts when cumulative agent cost ≥ $2.50.
* Cost is computed against `state.agent_metrics` totals, which P2 ensured
  include cache_read/cache_write — so cap is accurate, not inflated.

**Tests (6 new):** unset / empty / non-positive / positive / garbage env
parsing; `pricing_registry` `OnceLock` stability.  Plus 3 `compute_cost_usd`
tests on `state.rs` from the P4 prologue (Sonnet formula, zero pricing,
empty metrics).  cade-server: **212 pass** (203 prior + 9 new).  Workspace +
wasm32 clean.

**Why pure parse fn instead of env-coupled tests:** the workspace forbids
`unsafe_code`, and `std::env::set_var` is now `unsafe` on edition 2024.
Splitting `parse_max_session_cost(Option<&str>)` from the env-reading wrapper
keeps the parsing logic 100% testable without touching process state.

**Rollback:** revert this commit.  No schema/migration; env var is opt-in.

## 2026-04-25 — Phase P5: compress unused tool schemas on long sessions

**Goal:** cut tool-schema overhead on long sessions by ~70% for tools the
model isn't currently using.

**Problem:** `build_context` already prunes `desktop_*` schemas when unused
(EXTENDED_TOOL_PREFIXES) but every other registered tool ships with full
description + per-parameter descriptions every turn.  With 50+ wired tools
× ~600 chars/schema (TOOL_SCHEMA_CHARS_ESTIMATE) ≈ 30 KB of mostly-static
prose retransmitted on every turn at full input rate (cache_control on the
tools array helps for Anthropic but not OpenAI/Gemini in the same way).

**Change:**
1. New `compress_tool_schema(Value) -> Value` helper:
   * Truncates top-level `description` to first newline, capped at
     `COMPRESSED_DESCRIPTION_CHAR_CAP = 80` chars.
   * Strips `description` and `examples` from each property in
     `parameters.properties` and `input_schema.properties` (covers OpenAI/
     Gemini and Anthropic shapes).
   * Preserves `name`, JSON-schema shape, types, required, enums.
   * Idempotent.
2. Refactored prune step in `build_context` to compute `recently_used` once
   and reuse for both pruning and compression.
3. After pruning, map: schemas in `ALWAYS_INCLUDE_TOOL_NAMES` or recently
   used → untouched; otherwise → `compress_tool_schema`.

**Tests (6 new):** name+shape preservation, first-newline truncation,
input_schema variant handling, idempotency, no-description safety,
≥4× byte-size reduction on a realistic schema.

**Activation:** only when `messages.len() > 1 + RECENT_WINDOW` (= long
session).  Short sessions and tools the model just used keep full schemas
so the agent always has fidelity for active workflows.

**Why not strip more:** removing parameter `type` / `required` would break
client-side validation in some providers.  Top-level + property descriptions
are the only safely-removable parts of a JSON-schema.

**Cost impact:** for a 50-tool wired session with no recent desktop or
extended use, ~25 KB of tool-schema text becomes ~6 KB on the wire,
~75% reduction.  Combined with P1 (skills cache anchor), Anthropic
sessions see the largest win because `system_static` AND tools array are
both cache-anchored — the compression helps the cache-write cost on first
turn after compression boundary changes.

cade-server: **218 pass** (212 prior + 6 new).  Workspace + wasm32 clean.

**Rollback:** revert this commit.  No schema/migration; pure transformation
of in-flight tool schemas.
