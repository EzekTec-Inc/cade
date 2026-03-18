# PLAN.md — Change Log

---

## 2026-03-18T00:00:00Z — Add CI/CD pipeline

**Summary:** Created GitHub Actions CI workflow for the workspace.

**Files modified:**
- `CREATED` `.github/workflows/ci.yml`
- `MODIFIED` `CLAUDE.md` (marked item #2 as completed)

**Reason:** Item #2 in CLAUDE.md — the project had no CI pipeline.

**Previous behavior:** No automated build/test/lint on push or PR.

**New behavior:** On push to `main` or PR against `main`, the workflow runs:
1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo build --workspace`
4. `cargo test --workspace`

Single job, stable Rust toolchain, ubuntu-latest. System dependencies installed
via apt-get (libssl-dev, libdbus-1-dev, libpipewire-0.3-dev, libwayland-dev,
libx11-dev, libxcb-randr0-dev, libxcb-shm0-dev, libclang-dev, pkg-config).
Cargo build cache via `Swatinem/rust-cache@v2`.

**Rollback:** Delete `.github/workflows/ci.yml`. Revert `CLAUDE.md` item #2
text back to the 🟡 TODO version.

---

## 2026-03-18T00:01:00Z — Stale DB providers cleanup (Migration 8)

**Summary:** Added a startup migration that deletes provider rows whose
encrypted API key can no longer be decrypted.

**Files modified:**
- `MODIFIED` `crates/cade-server/src/server/storage/sqlite.rs`
  — added Migration 8 block in `run_migrations()` (lines ~232–256)
  — added `test_migration_8_removes_stale_providers` test
- `MODIFIED` `CLAUDE.md` (marked item #3 as completed)

**Reason:** Item #3 in CLAUDE.md — 3 stale provider rows in `~/.cade/cade.db`
encrypted by a previous `.cade-db.key` generated warnings on every startup.

**Previous behavior:** `list_providers()` logged a warning per undecryptable
row and skipped it. The rows remained in the DB permanently.

**New behavior:** `run_migrations()` (called once at DB open) iterates all
provider rows with non-empty `api_key`, attempts `crypto::decrypt()`, and
DELETEs any row where decryption fails. Providers with NULL/empty keys
(e.g. ollama) are not touched. The migration logs which providers were removed.

**Rollback:** Remove the "Migration 8" block (~25 lines) from `run_migrations()`
in `sqlite.rs`. Remove the `test_migration_8_removes_stale_providers` test.
Revert `CLAUDE.md` item #3 text back to the 🟡 TODO version. The stale rows
will reappear as startup warnings (harmless).

---

## 2026-03-18T00:02:00Z — Extract `cade-tui` crate from `cade-cli/src/ui/`

**Summary:** Moved the TUI layer (10 files, 5,390 LOC) into a standalone
`cade-tui` crate. `cade-cli` depends on it and re-exports all public items.

**Files modified:**
- `CREATED` `crates/cade-tui/Cargo.toml`
- `CREATED` `crates/cade-tui/src/lib.rs`
- `CREATED` `crates/cade-tui/src/{app,autocomplete,component,editor,markdown,markdown_test,menu,question,skills}.rs` (moved from `cade-cli/src/ui/`)
- `MODIFIED` `crates/cade-tui/src/app.rs` — `crate::ui::` → `crate::` (sed replacement, ~35 occurrences)
- `DELETED` `crates/cade-cli/src/ui/{app,autocomplete,component,editor,markdown,markdown_test,menu,question,skills}.rs`
- `MODIFIED` `crates/cade-cli/src/ui/mod.rs` — replaced with `pub use cade_tui::*;`
- `MODIFIED` `crates/cade-cli/Cargo.toml` — added `cade-tui` dep, switched `pulldown-cmark` to workspace
- `MODIFIED` `Cargo.toml` (workspace) — added `cade-tui` to members, added `pulldown-cmark` workspace dep
- `MODIFIED` `CLAUDE.md` — marked item #4 complete, updated dependency graph

**Reason:** Item #4 in CLAUDE.md — architectural separation of TUI rendering
from CLI orchestration logic. Improves incremental compile times.

**Previous behavior:** TUI code lived in `crates/cade-cli/src/ui/`. Any change
to the TUI or CLI recompiled both together.

**New behavior:** `cade-tui` compiles independently. `cade-cli` depends on it
and re-exports via `pub use cade_tui::*` — all `crate::ui::*` paths in
`repl.rs` resolve unchanged. The root crate's `pub use cade_cli::ui` chain
also works. Zero API changes, zero public interface changes.

**Rollback:**
1. Copy `crates/cade-tui/src/*.rs` back to `crates/cade-cli/src/ui/`
2. Restore the original `crates/cade-cli/src/ui/mod.rs` (module declarations + pub uses)
3. Undo `crate::` → `crate::ui::` in the restored `app.rs` (~35 lines)
4. Remove `cade-tui` from workspace members and `cade-cli` deps
5. Remove `pulldown-cmark` from workspace deps, restore `pulldown-cmark = "0.13.1"` in `cade-cli/Cargo.toml`
6. Delete `crates/cade-tui/`

---

## 2026-03-18T00:03:00Z — Extract `cade-mcp` crate from `cade-agent/src/mcp/`

**Summary:** Moved the MCP client layer (2 files, 673 LOC) into a standalone
`cade-mcp` crate. `cade-agent` depends on it and re-exports all public items.

**Files modified:**
- `CREATED` `crates/cade-mcp/Cargo.toml`
- `CREATED` `crates/cade-mcp/src/lib.rs` (moved from `cade-agent/src/mcp/mod.rs`)
- `CREATED` `crates/cade-mcp/src/watcher.rs` (moved from `cade-agent/src/mcp/watcher.rs`)
- `DELETED` `crates/cade-agent/src/mcp/watcher.rs`
- `MODIFIED` `crates/cade-agent/src/mcp/mod.rs` — replaced with `pub use cade_mcp::*;`
- `MODIFIED` `crates/cade-agent/Cargo.toml` — added `cade-mcp` dep, removed `rmcp`, `notify`, `dirs` (now in `cade-mcp`)
- `MODIFIED` `Cargo.toml` (workspace) — added `cade-mcp` to members
- `MODIFIED` `CLAUDE.md` — marked item #5 complete, updated dependency graph

**Reason:** Item #5 in CLAUDE.md — architectural separation of MCP client
logic from agent tool dispatch. Improves incremental compile times.

**Previous behavior:** MCP code lived in `crates/cade-agent/src/mcp/`. Any
change to MCP or agent tools recompiled both together.

**New behavior:** `cade-mcp` compiles independently. `cade-agent` depends on
it and re-exports via `pub use cade_mcp::*` — all `crate::mcp::*` paths in
`tools/manager.rs` and all `cade_agent::mcp::*` paths in `cade-cli` and root
crate resolve unchanged. Zero API changes, zero public interface changes.

**Rollback:**
1. Copy `crates/cade-mcp/src/lib.rs` back to `crates/cade-agent/src/mcp/mod.rs`
2. Copy `crates/cade-mcp/src/watcher.rs` back to `crates/cade-agent/src/mcp/watcher.rs`
3. Remove `cade-mcp` from workspace members and `cade-agent` deps
4. Restore `rmcp`, `notify`, `dirs` deps in `cade-agent/Cargo.toml`
5. Delete `crates/cade-mcp/`

---

## 2026-03-18T00:06:00Z — Rust Edition 2024 Migration

**Summary:** Upgraded the entire workspace to Rust Edition 2024 (per rust10x
recommendation). Switched workspace resolver to `3`, updated every crate’s
`edition` to `"2024"`, and addressed new strictness rules (pattern bindings,
unsafe env APIs). Introduced `cade_core::agent_env` so child process spawns get
the `AGENT_ID` context without touching the now-unsafe `std::env::set_var`.
Replaced env mutation tests with deterministic helpers. All 295 tests still pass.

**Key changes:**
- `Cargo.toml` (root): `edition = "2024"`, `resolver = "3"`
- All crate `Cargo.toml` files: edition bumped to 2024
- Added `cade_core::agent_env` module + re-export; `cade-desktop` now depends on
  `cade-core`
- Refactored all `std::process::Command` / `tokio::process::Command` call sites to
  call `cade_core::agent_env::apply_agent_env()` (bash tool, hooks, CLI slash cmds,
  desktop controls, MCP child processes, etc.)
- CLI now calls `cade_core::agent_env::set_agent_id(...)` instead of mutating the
  process environment
- `ServerConfig::from_env_with_port()` avoids `set_var`; CLI uses `.from_env_with_port`
- Crypto tests seed `.cade-db.key` instead of setting env vars
- Rate limiter gained `from_env_with_reader()` so tests no longer mutate env
- Fixed Edition 2024 pattern strictness (question multi-select filter, headless tool loop)

**Verification:** `cargo test --workspace`

**Rollback:**
1. Revert all `edition = "2024"` changes to `"2021"` and set workspace `resolver = "2"`
2. Remove `cade_core::agent_env` module and delete all `apply_agent_env`/`set_agent_id`
   calls; restore previous `std::env::set_var` usage (with unsafe blocks!)
3. Revert `ServerConfig`/rate limiter refactors and test updates

---

## 2026-03-18T00:05:00Z — rust10x Tier 1 Compliance Fixes

**Summary:** Implemented Tier 1 items from the rust10x audit: added `unsafe_code = "forbid"`
via `[lints.rust]` in every crate (including root package) and disabled doctests in all
library crates. Added `// region:    --- Modules` wrappers to every `main.rs`, `lib.rs`,
and `mod.rs` that declares modules or use-reexports (25 files). All 295 tests still pass
(`cargo test --workspace`).

**Files modified (highlights):**
- `Cargo.toml` — `[lints.rust]` block
- `crates/*/Cargo.toml` (8 crates) — added `[lib] doctest = false` + `[lints.rust]`
- Module files: `src/main.rs`, `src/lib.rs`, `crates/cade-*/src/lib.rs`, and every
  `mod.rs` under `crates/cade-*` now have a `Modules` region containing their `mod`/`use`
  declarations.

**Reason:** Addressed the three CRITICAL rust10x findings (missing lint guard, doctest
settings, missing module regions).

**Verification:** `cargo test --workspace`

**Rollback:** For each modified Cargo.toml remove the `[lib]` and `[lints.rust]` blocks
added in this change. In each touched Rust file, remove the inserted region comments.

---

## 2026-03-18T00:04:00Z — rust10x Compliance Audit

**Summary:** Conducted systematic audit of all 8 workspace crates against rust10x
guidelines from `~/.aipack-base/pack/installed/pro/rust10x/`. Generated comprehensive
findings report with severity classification and prioritized recommendations.

**Files created:**
- `RUST10X_AUDIT_2026-03-18.md` — 13.8KB audit report

**Files modified:**
- `CLAUDE.md` — marked item #6 complete

**Reason:** Item #6 in CLAUDE.md — systematic compliance check against rust10x
best practices to identify gaps and provide actionable recommendations.

**Findings:**
- **3 CRITICAL** (lints.rust missing, Edition 2021 instead of 2024, no doctest = false)
- **12 MAJOR** (error handling pattern, missing code regions, test structure, Cargo.toml sections)
- **28 MINOR** (code section markers, file structure, comment delimiters, examples)

**Key deviations:**
1. All crates use `anyhow`/`thiserror` — rust10x forbids these, mandates custom `Error` enum with `derive_more`
2. Edition 2021 — misses if-let chains, inline macro values, async closures
3. No `// region: --- Modules` wrappers in any lib.rs/main.rs/mod.rs
4. Test structure lacks `// region: --- Tests` wrappers and Setup/Exec/Check sections
5. Cargo.toml missing `[lints.rust]` blocks and dependency section comments

**Recommended tiers:**
- **Tier 1 (Do Now):** Safety/standards fixes — 3.25 hours
- **Tier 2:** Edition 2024 migration — 12 hours (requires Rust 1.85+)
- **Tier 3:** Test structure improvements — 33 hours
- **Tier 4:** Code organization — 14 hours
- **Tier 5 (DEFER):** Error handling migration (~300h), CLI refactor (60h)

**Rollback:** Delete `RUST10X_AUDIT_2026-03-18.md`, revert CLAUDE.md item #6 to 🟢 FUTURE.

**Next steps:** User to review audit report and select fixes to implement (if any).

---

## 2026-03-18T00:07:00Z — rust10x Tier 3: Test region wrappers + Result alias

**Summary:** Added `// region: --- Tests` / `// endregion: --- Tests` wrappers
and `type Result<T>` alias to all 21 test modules across the workspace, per
rust10x compliance audit items M3 and M8.

**Files modified:**
- `crates/cade-tui/src/editor.rs` — region + Result alias
- `crates/cade-tui/src/app.rs` — region + Result alias
- `crates/cade-tui/src/markdown.rs` — region + Result alias
- `crates/cade-core/src/skills/mod.rs` — region + Result alias
- `crates/cade-core/src/toolsets/mod.rs` — region + Result alias
- `crates/cade-core/src/hooks/mod.rs` — replaced `// ── Tests` with region + Result alias
- `crates/cade-core/src/settings/manager.rs` — region + Result alias
- `crates/cade-core/src/permissions/mod.rs` — added `#[allow(unused)]` to existing alias
- `crates/cade-server/src/server/rate_limit.rs` — region + Result alias
- `crates/cade-server/src/server/crypto.rs` — region + Result alias
- `crates/cade-server/src/server/storage/sqlite.rs` — region + Result alias
- `crates/cade-ai/src/lib.rs` — region + Result alias
- `crates/cade-ai/src/catalogue.rs` — region + Result alias
- `crates/cade-ai/src/anthropic.rs` — region + Result alias
- `crates/cade-ai/src/openai.rs` — region + Result alias
- `crates/cade-cli/src/cli/headless.rs` — region + Result alias
- `crates/cade-agent/src/tools/search.rs` — 2 regions (tests + glob_tests) + Result alias
- `crates/cade-agent/src/tools/bash.rs` — region + Result alias
- `crates/cade-agent/src/tools/fs.rs` — region + Result alias
- `crates/cade-agent/src/tools/manager.rs` — region + Result alias
- `tests/approval_tests.rs` — region wrapper around all 4 test modules

**Reason:** rust10x audit items M3 (test region wrappers) and M8 (test Result alias).

**Previous behavior:** Test modules had no region markers and no Result alias.

**New behavior:** Every `#[cfg(test)] mod tests { ... }` block is wrapped in
`// region: --- Tests` / `// endregion: --- Tests`. Each test module contains
`#[allow(unused)] type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;`
for future use when tests are migrated to return `Result<()>`.

**Verification:** `cargo test --workspace` — 295 tests pass, 0 failures.
`cargo clippy --workspace --all-targets` — no `type alias unused` warnings.

**Rollback:** Remove all `// region: --- Tests` / `// endregion: --- Tests`
comment lines. Remove all `#[allow(unused)]` + `type Result<T>` lines from
test modules. Restore `// ── Tests ──...` in `hooks/mod.rs`.

---

## 2026-03-18T00:08:00Z — rust10x Tier 4: Cargo.toml dependency section comments

**Summary:** Added `# -- Section Name` comments to group dependencies in all 9
Cargo.toml files (root + 8 crates), per rust10x audit item M5.

**Files modified:**
- `Cargo.toml` (root package) — reworded existing comments to `# --` style
- `crates/cade-core/Cargo.toml`
- `crates/cade-ai/Cargo.toml`
- `crates/cade-server/Cargo.toml`
- `crates/cade-agent/Cargo.toml`
- `crates/cade-cli/Cargo.toml`
- `crates/cade-desktop/Cargo.toml`
- `crates/cade-tui/Cargo.toml`
- `crates/cade-mcp/Cargo.toml`

**Reason:** rust10x audit item M5 — dependency sections missing `# -- Section`
comments for scanability.

**Previous behavior:** Dependencies listed without section grouping in crate Cargo.toml files.

**New behavior:** Dependencies grouped under `# -- Workspace crates`,
`# -- Error handling`, `# -- Serialisation`, `# -- Logging`, `# -- Async`,
`# -- HTTP`, `# -- Filesystem`, `# -- Misc utilities`, `# -- Server`,
`# -- Crypto`, `# -- Desktop`, `# -- CLI / TUI`, `# -- MCP`,
`# -- Clipboard / image` section comments as appropriate.

**Verification:** `cargo test --workspace` — 295 tests pass.

**Rollback:** Remove all `# --` comment lines from `[dependencies]` sections.

---

## 2026-03-18T00:09:00Z — rust10x Tier 4: Replace qualified serde_json::json! with use import

**Summary:** Replaced all 55 occurrences of `serde_json::json!()` with `json!()`
by adding `use serde_json::json;` imports, per rust10x audit item M11.

**Files modified:**
- `src/main.rs` — added `use serde_json::json;`, replaced 11 occurrences
- `crates/cade-agent/src/agent/client.rs` — already had import, replaced 6 occurrences
- `crates/cade-agent/src/tools/bash.rs` — expanded import, replaced 1 occurrence
- `crates/cade-agent/src/tools/desktop.rs` — expanded import, replaced 4 occurrences
- `crates/cade-agent/src/tools/fs.rs` — expanded import, replaced 4 occurrences
- `crates/cade-agent/src/tools/manager.rs` — added import in test module, replaced 4 occurrences
- `crates/cade-agent/src/tools/plan.rs` — expanded import, replaced 5 occurrences
- `crates/cade-agent/src/tools/search.rs` — expanded import, replaced 15 occurrences
- `crates/cade-ai/src/lib.rs` — added import in test module, replaced 1 occurrence
- `crates/cade-cli/src/cli/repl.rs` — added import, replaced 1 occurrence
- `crates/cade-server/src/server/rate_limit.rs` — added import, replaced 1 occurrence
- `crates/cade-server/src/server/storage/sqlite.rs` — added import in test module, replaced 2 occurrences

**Reason:** rust10x audit item M11 — macro imports should use `use` imports, not
qualified paths.

**Previous behavior:** `serde_json::json!({...})` used throughout codebase.

**New behavior:** `json!({...})` with `use serde_json::json;` at module/test scope.

**Verification:** `cargo test --workspace` — 295 tests pass.
`cargo clippy --workspace --all-targets` — no unused import warnings.

**Rollback:** Revert `use serde_json::json;` additions and replace `json!(` with
`serde_json::json!(` in all affected files.

---

## 2026-03-18T00:10:00Z — Update docs/roadmap.md short-term items

**Summary:** Marked 4 short-term roadmap items as complete and added 2 new
completed items (Edition 2024 and rust10x compliance).

**Files modified:**
- `docs/roadmap.md` — checked off completed short-term items

**Reason:** Roadmap was out of date — items completed but still marked `[ ]`.

**Previous behavior:** Short-term items showed as incomplete.

**New behavior:** All 6 short-term items marked `[x]`.

**Rollback:** Revert `[x]` back to `[ ]` and remove the 2 added lines.

---

## 2026-03-18T00:11:00Z — rust10x Tier 4: Convert em-dash section markers to // -- style

**Summary:** Converted 331 `// ── Section Name ──...` section markers to
`// -- Section Name` per rust10x audit items N11-N15.

**Files modified:** 34 `.rs` files across all crates, `src/`, and `tests/`.

**Reason:** rust10x uses `// --` (double hyphen) for section markers, not
`// ──` (em dash with trailing dash fill).

**Previous behavior:** Section markers used `// ── Name ─────...` em-dash style.

**New behavior:** Section markers use `// -- Name` double-hyphen style.

**Verification:** `cargo test --workspace` — 295 tests pass.

**Rollback:** Run the inverse regex replacement:
`s/^(\s*)(\/\/ -- (.+))$/\1\/\/ ── \3 ────.../` (would need to regenerate
trailing dashes to original length — simpler to `git revert`).

---

## 2026-03-18T00:12:00Z — Apply cargo clippy --fix auto-corrections

**Summary:** Applied `cargo clippy --fix --workspace --all-targets` to auto-fix
~100 clippy warnings. Changes include: collapsed if-statements, simplified
map_or calls, removed unnecessary references, replaced iterating on map values,
removed redundant closures, replaced extend with append, etc.

**Files modified:** 25 `.rs` files across all crates and root package.

**Reason:** Reduce clippy warning count from ~200 to ~80 (remaining are
non-auto-fixable: doc comments, too-many-args, MutexGuard across await, etc.)

**Previous behavior:** ~200 clippy warnings.

**New behavior:** ~80 clippy warnings (remaining require manual review / design changes).

**Verification:** `cargo test --workspace` — 295 tests pass.

**Rollback:** `git revert <commit>`.

---

## 2026-03-18T00:13:00Z — Fix MCP server settings (context7 + openviking)

**Summary:** Fixed `~/.cade/settings.json` MCP server commands so both
`context7` and `openviking` can start successfully.

**Files modified:**
- `~/.cade/settings.json` — updated `mcpServers.context7` and `mcpServers.openviking`

**Reason:** Both MCP servers failed to start on `/mcp reload`:
- `context7`: configured path `~/.gemini/extensions/context7/packages/mcp/dist/index.js`
  did not exist (source repo never built). Changed to `npx -y @upstash/context7-mcp`.
- `openviking`: configured `python3` (system) lacked the `mcp` pip package.
  Changed to the venv interpreter at `.../openviking/.venv/bin/python3`.

**Previous behavior:** `/mcp reload` → `✗ failed to start: context7, openviking`

**New behavior:** Both servers start, complete MCP handshake, and expose tools.

**Rollback:** Revert the two `mcpServers` entries in `~/.cade/settings.json` to
the original `"command": "node"` / `"command": "python3"` values.

---

## 2026-03-18T00:14:00Z — Add session footer metrics (tokens, cost, context, mode)

**Summary:** Render a compact session metrics line below the TUI footer, showing
session tokens, cache tokens, total cost, context usage vs model window, and
current permission mode:
`↑13k ↓50k R18M W248k $11.719 13.9%/1.0M (auto)`.

**Files modified:**
- `crates/cade-cli/src/cli/repl.rs`
  - Added helpers `fmt_tok_short`, `fmt_window_tokens_short`, `short_mode_label`.
  - Extended the per-turn UI consumer (`ui_task`) `"usage_statistics"` branch to:
    - Read session-level input/output token counters.
    - Aggregate cache-read/cache-write tokens and total USD cost from
      `SessionStats` (reusing existing `compute_cost`).
    - Compute per-turn context usage percentage using `input_tokens + cache_read_tokens`
      over `context_window_for_model(model)`.
    - Read the current `PermissionMode` from `TuiApp` and map to a short label
      (`auto|edits|plan|yolo`).
    - Build the metrics string and assign it to `app.footer_extra` for display
      in the extra footer row.
- `crates/cade-tui/src/app.rs` (read-only) — used existing `footer_extra` row and
  `context_pct` display; no structural changes.

**Reason:** Provide always-visible feedback in the TUI about:
- Session token usage (↑ input, ↓ output)
- Cache token usage (R read, W write)
- Approximate dollar cost (per-model pricing)
- Context usage for the latest turn vs the model's context window
- Current permission mode ("auto" = Default)

**Previous behavior:** Footer showed mode, CWD, agent name, model, reasoning
mode, and a simple `%` context bar. No compact summary of session tokens or
cost; `/usage`, `/stats`, and `/context` were separate commands.

**New behavior:** After the first `usage_statistics` SSE event in a session, the
TUI renders an extra row below the footer with a compact metrics string derived
from:
- `session_input_tokens` / `session_output_tokens` (↑ / ↓),
- `SessionStats.per_model` totals (R/W + cost),
- `cade_ai::catalogue::context_window_for_model` (window tokens),
- `TuiApp.context_pct` (context usage),
- `PermissionMode` (mapped to `(auto|edits|plan|yolo)`).

**Verification:** `cargo test --workspace` — 295 tests pass.
`cargo clippy --workspace --all-targets` — no warnings about the new helpers.

**Rollback:** In `crates/cade-cli/src/cli/repl.rs`:
1. Remove `fmt_tok_short`, `fmt_window_tokens_short`, and `short_mode_label`.
2. Revert the added `sess_in_tok_ui`, `sess_out_tok_ui`, and `sess_stats_ui`
   bindings in `agent_turn`.
3. Restore the `"usage_statistics"` match arm in the UI consumer to only call
   `set_context_pct` (as before) and remove the `footer_extra` update.

---

## 2026-03-18T00:15:00Z — Fix context wipe: page history by budget + raise max chars

**Summary:** Remove the fixed 80-row history cap. History is now paged (100-row
pages) until a soft budget (1.3× context char budget) is reached, so long
sessions keep as much context as the model window allows. Increased
`MAX_CONTEXT_CHARS` to 6_000_000 (≈2M tokens) so large-window models aren’t
artificially halved. Added a placeholder summary when auto-compaction fails so
hard trimming still preserves continuity.

**Files modified:**
- `crates/cade-server/src/server/api/messages.rs`
  - Replaced single `HISTORY_LIMIT` fetch with paged loading using
    `HISTORY_PAGE_SIZE=100` and a soft cap of 1.3× `context_char_budget`.
  - Raised `MAX_CONTEXT_CHARS` from 3_000_000 → 6_000_000.
  - On compaction failure, insert a placeholder system message before hard trim.
- `crates/cade-server/src/server/storage/sqlite.rs`
  - Added `list_messages_page` (limit/offset) and made `list_messages` call it.

**Reason:** Older turns were dropped entirely once 80 rows were exceeded, so
long tasks appeared “wiped”. Paging by budget keeps all history the context can
fit; larger max chars honor 2M-token models.

**Previous behavior:** Load newest 80 rows only; then trim to budget. Long
sessions lost early context regardless of model window. Compaction failures
fell straight to hard trim with no continuity hint.

**New behavior:** Page history until near budget; no arbitrary 80-row cap. Large
models get up to ~2M tokens of char budget. If compaction fails, a placeholder
summary is injected before trim.

**Verification:** `cargo test --workspace` — 295 tests pass.

**Rollback:**
1. In `messages.rs`, restore single `sqlite::list_messages(..., 80)` and remove
   paging/soft-cap loop; revert `HISTORY_PAGE_SIZE` to `HISTORY_LIMIT = 80` and
   `MAX_CONTEXT_CHARS` to 3_000_000; remove the compaction failure placeholder.
2. In `sqlite.rs`, remove `list_messages_page` and revert `list_messages` to the
   old LIMIT-only query (no OFFSET).
