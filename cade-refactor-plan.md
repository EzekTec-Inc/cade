# CADE TUI & CLI Refactor Plan (Inspired by OpenCode Analysis)

This document outlines architectural improvements and feature enhancements identified from the OpenCode TUI analysis and how they apply directly to CADE's Rust-based codebase (`crates/cade-tui`, `crates/cade-cli`, and `crates/cade-core`).

---

## 1. Leader-Key / "Which-Key" Chord System (`Ctrl+X` Chords)

- **Target Files:** `crates/cade-tui/src/app/input.rs`, `crates/cade-tui/src/app/layout/`
- **Concept:** Introduce a leader key sequence (e.g., `Ctrl+X`) that enters a leader state and displays a responsive popup/bottom hint bar showing available key chords:
  - `m` $\rightarrow$ Open Model Picker
  - `s` $\rightarrow$ Open Session Picker / Tree
  - `t` $\rightarrow$ Cycle / Select Theme
  - `u` $\rightarrow$ Undo last checkpoint
  - `r` $\rightarrow$ Redo checkpoint
  - `p` $\rightarrow$ Toggle Permissions mode
  - `?` $\rightarrow$ Show Help overlay
- **Benefits:** Faster keyboard-driven navigation without typing full slash commands or opening heavyweight modal overlays.

---

## 2. Global Detail & Diff View Controls (`/details`, Stacked vs Split Diffs)

- **Target Files:** `crates/cade-tui/src/app/timeline/render_item.rs`, `crates/cade-tui/src/app/state.rs`, `crates/cade-cli/src/cli/repl/slash.rs`
- **Concept:**
  - Add `/details` command (or shortcut toggle) to expand/collapse full stdout/stderr and raw tool payloads across all timeline items.
  - Implement side-by-side vs stacked diff rendering for file modification tool calls (`edit_file`, `write_file`), with syntax highlighting via `syntect`.
- **Benefits:** Significantly better code review experience within wide and compact terminal viewports.

---

## 3. Terminal Attention Cues & Completion Notifications

- **Target Files:** `crates/cade-cli/src/cli/repl/turn_loop.rs`, `crates/cade-tui/src/app/mod.rs`
- **Concept:**
  - Integrate terminal bell (`\x07`) or OSC 9/99 desktop notification sequences when an agent finishes long-running tool runs or requires user input / permission approval.
  - Optional sound / chime trigger on subagent completion or turn yield.
- **Benefits:** Seamless developer workflow when switching terminal tabs while long-running agent tasks or builds run in the background.

---

## 4. UI/UX Configuration Decoupling (`tui.toml` vs `cade.toml`)

- **Target Files:** `crates/cade-core/src/settings/`, `crates/cade-tui/src/colors.rs`
- **Concept:**
  - Separate TUI appearance settings into dedicated config schema (`tui.toml` or `~/.config/cade/tui.toml`):
    - Default theme, custom color palette overrides
    - Cursor styling (block, line, underline)
    - Scroll interpolation speeds & mouse acceleration
    - Keybinding / leader chord remappings
- **Benefits:** Cleaner separation of visual concerns from agent runtime policies, model providers, and MCP definitions.

---

## 5. Terminal Drag-and-Drop Path / URI Sanitization

- **Target Files:** `crates/cade-tui/src/editor.rs`, `crates/cade-tui/src/autocomplete.rs`
- **Concept:**
  - Enhance editor paste/bracketed paste handling to recognize dropped file paths, URLs, and escaped terminal strings.
  - Automatically sanitize escaped paths (e.g. `file:///...` or backslash-escaped spaces) into `@<path>` context tokens or prompt attachments.
- **Benefits:** Frictionless drag-and-drop context injection from file managers into CADE.
