# CADE Enhancement Implementation Plan

This document outlines a zero-bloat, high-leverage enhancement plan for CADE, focusing on developer experience (DX), guardrails, and testing.

---

## Phase 1: Granular Path Protections (Zero-Bloat Security)

**Goal:** Prevent the agent from accidentally or maliciously modifying sensitive repository or system files (e.g., `.git/`, `.env`, `.ssh/`), even in `--yolo` (bypass permissions) mode.

1. **Define Protected Patterns:**
   - In `crates/cade-core/src/permissions/mod.rs` (or a new `protected_paths.rs`), define a hardcoded list of globally protected paths/globs (e.g., `**/.git/**`, `**/.env*`, `~/.ssh/**`).
2. **Update Permission Manager:**
   - Modify `PermissionManager::is_blocked` to intercept file-write tools (`edit_file`, `write_file`, `apply_patch`, `bash` operations involving redirects).
   - If a tool targets a protected path, return a strict `HookOutcome::Block` with a specific security reason.
3. **Tests:**
   - Add unit tests in `crates/cade-core` verifying that attempts to write to `.env` or `.git/config` are blocked across all permission modes (including `bypassPermissions`).
4. **Validation:**
   - Run `cargo test -p cade-core`
   - Run `cargo clippy -p cade-core -- -D warnings`

---

## Phase 2: Automatic Pre-Execution Git Checkpoints

**Goal:** Automatically stash or snapshot the working tree before the agent makes its first destructive file edit in a session, enabling an instant `/undo`.

1. **Configuration:**
   - Add `auto_checkpoint: bool` (default: `true`) to the `ProjectSettings` struct in `crates/cade-core/src/settings/manager.rs`.
2. **State Tracking:**
   - In the `cade-cli` REPL loop (`crates/cade-cli/src/cli/repl.rs`), track whether a checkpoint has been taken for the current turn/session.
3. **PreToolUse Hook Integration:**
   - Inject logic into the built-in `PreToolUse` hook handling for `edit_file`, `write_file`, and `apply_patch`.
   - If `auto_checkpoint` is enabled and no checkpoint exists for this turn, trigger the existing `create_checkpoint` API endpoint automatically.
4. **Undo Command:**
   - Map a new `/undo` slash command to fetch the latest auto-checkpoint and call `restore_checkpoint`.
5. **Validation:**
   - Run `cargo test -p cade-core -p cade-cli`
   - Run `cargo clippy -p cade-cli -- -D warnings`

---

## Phase 3: Config Hot-Reloading

**Goal:** Watch `~/.cade/settings.json` and `.cade/settings.local.json` for changes to instantly reload MCP tools, hooks, and permissions without restarting the server or TUI.

1. **File Watcher Integration:**
   - Add the `notify` crate (already standard for Rust filesystem watching, minimal bloat) to `cade-server` and/or `cade-cli`.
2. **Settings Reload Logic:**
   - Spawn a lightweight background thread on startup that watches the settings paths.
   - On `Modify` events, trigger the existing settings reload logic and re-bind MCP servers/hooks.
3. **Validation:**
   - Run `cargo test --workspace`
   - Run `cargo clippy --workspace -- -D warnings`

---

## Phase 4: Final Workspace Validation & Build

**Goal:** Ensure all enhancements compile cleanly, introduce zero warnings, and pass the comprehensive test suite.

1. **Format Check:**
   ```bash
   cargo fmt --all -- --check
   ```
2. **Linting (Clippy):**
   ```bash
   cargo clippy --workspace --all-targets -- -D warnings
   ```
3. **Test Suite:**
   ```bash
   cargo test --workspace
   ```
4. **Release Build:**
   ```bash
   cargo build --release --workspace
   ```
5. **Runtime Smoke Test:**
   - Start `./target/release/cade-server` in the background.
   - Start `./target/release/cade` and verify the TUI loads without errors.
   - Execute `/info` to confirm the agent connects successfully.