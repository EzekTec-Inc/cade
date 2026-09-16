# CADE TUI — Replicate OpenCode TUI UX (Implementation Spec)

> Audience: an implementing LLM/engineer working in this repo.
> Context/analysis: `docs/opencode-tui-analysis.md`.
> Do not regress existing behavior. Each section lists: scope, files to touch, exact behavior, acceptance criteria.

Working tree: `/home/engrubanese/Downloads/02 Rust-project/cade`
- TUI library: `crates/cade-tui` (ratatui). Host: `crates/cade-cli/src/cli/repl/`.
- State owner: `TuiApp` (`crates/cade-tui/src/app/mod.rs`). Draw: `app/render.rs`. Input: `app/input.rs`. State change: `app/reducer.rs`, `app/state.rs` (`TuiApp::push`). Timeline: `app/timeline/`. Clipboard: `app/clipboard.rs`. Editor: `app/editor.rs`. TUI config: `cade-core/src/settings/tui.rs` (`tui.toml`).
- SSE→UI: `crates/cade-cli/src/cli/repl/turn_loop/stream.rs` (UI consumer task), `repl/mod.rs` (`Repl` holds `settings: Arc<Mutex<SettingsManager>>` and `app: Arc<Mutex<TuiApp>>`).

---

## 0. Principles

1. **Keybind collisions:** all new chords use a leader key (`ctrl+x`) unless the binding is a terminal-safe standard (arrows, page/home/end, `ctrl+u/d`, `esc`). Never hijack raw `ctrl+c`/`ctrl+d`/`ctrl+z`.
2. **Config over constants:** every tunable lands in `TuiSettings` (`tui.toml`), following the existing `#[serde(default)]` pattern in `cade-core/src/settings/tui.rs`.
3. **Async rule (R-04):** UI mutations and `draw()` only happen on the UI consumer task; do not block the SSE `on_event` path. Reuse `stream.rs`’s channel pattern for any new background work.
4. **No forcing nerd-fonts:** every new glyph has an ASCII/Unicode fallback via `icons.rs` helpers and `use_nerd_fonts`.
5. **Preserve paste losslessness:** collapsed-paste markers (`[paste #N: M lines]`, `[image #N: WxH]`) in `editor.rs` stay; only extend them.

---

## A. Session cost cap — settings.json (done; verify)

The cap is read from `.cade/settings.json` key `max_session_cost_usd`, merged project > global, with the `CADE_MAX_SESSION_COST_USD` env override and `$120` default:
- `cade-core/src/settings/models.rs`: `GlobalSettings`/`ProjectSettings.max_session_cost_usd: Option<f64>`.
- `cade-core/src/settings/resolver.rs` `SettingsManager::max_session_cost_usd()`.
- Server loop: `crates/cade-server/src/server/api/run/mod.rs` `max_session_cost_usd(settings_cap)`.
- TUI gauge: `resolve_session_cost_cap` in `app/mod.rs` feeding `session_cost_cap_usd` → `SidebarState`.
**Acceptance:** changing `max_session_cost_usd` in `.cade/settings.json` moves the gauge % and the server abort threshold without restart.

---

## B. Selection & clipboard parity (opencode behavior) — P0

### B.1 Retained drag-selection
**Scope:** `app/input.rs` (`handle_message_area_mouse_event`, `copy_selected_text`), `app/mod.rs` (`mouse_selection`, `copy_highlight`), `app/rendering` of selection highlight (`timeline/render_item.rs`).
**Behavior (mirrors opencode Selection.copy `{retain:true}`):**
- On mouse release, copy the selected text (B.2) and **keep** the highlight painted.
- Dismiss the highlight when: the next key is **struck and unconsumed** by any binding/overlay, **or** a mouse-down occurs on non-selectable content, **or** the user explicitly runs the “drop selection” action.
- While a dialog/overlay is open, typing to filter it must NOT clear the selection.
- Scrolling/arrow keys that are consumed by bindings must NOT clear selection (only unconsumed keys do).
**Acceptance:** select text → highlight persists; type an unknown key → highlight clears; open palette and type filter → highlight persists until palette closes.

### B.2 Write to clipboard **and** Linux PRIMARY
**Scope:** `app/clipboard.rs` (`write_to_clipboard`, `read_clipboard_text`).
**Behavior:**
- Keep OSC52 first-write but add tmux/screen passthrough: if `TMUX` or `STY` env set, wrap as `\x1bPtmux;\x1b]52;c;<b64>\x07\x1b\\`.
- After OSC52 (or on failure), write via the existing arboard path, and additionally on Linux update the PRIMARY selection (`xclip -selection primary -i` / `wl-copy --primary`).
- Add `TuiSettings.linux_clipboard_selection: "clipboard" | "primary" | "both"` (default `both`). `"primary"` skips OSC52 and the clipboard buffer; `"both"` writes both.
- Copy success/failure feedback: on error, show a toast (severity `Error`) — never a silent “Copied” claim.
**Acceptance:** middle-click after drag-select pastes the selection on Linux; `linux_clipboard_selection="primary"` makes ctrl+v read behave per read-path note below; copy errors surface as toasts.

### B.3 Add selection to prompt as quoted context
**Scope:** `app/editor.rs`, `app/input.rs`, `app/reducer.rs` (`TuiAction`), `app/overlay_component.rs`.
**Behavior (mirrors `prompt.add_selection`, warm-key `<leader>p`, `ctrl+shift+c`):**
- New action binds `selection` → for each selected line prefix `> ` (skip empty lines; do not double-prefix lines already starting `> `), then insert into the editor **as a marker** reusing the paste-collapse machinery (`[paste #N: …]`) so it stashes/restores/expands on submit exactly like a pasted block.
- Runs from a command in the palette and from the leader chord `<leader>p`.
- Read the **live** retained selection at invocation time; clear retention after a successful insert (marker path), and also after failure (with a toast).
**Acceptance:** select transcript lines → `ctrl+x p` → prompt contains `> line` block marker that expands to the quoted text on send.

## C. Leader-key system + configurable keybinds — P0

### C.1 Config model
**Scope:** `cade-core/src/settings/tui.rs` (`TuiSettings`); new file `crates/cade-tui/src/keys.rs`.
- Add `TuiSettings.leader: String` (default `"ctrl+x"`) and `leader_timeout_ms` (default 2000).
- Add `keybinds: HashMap<String, KeybindSpec>` where `KeybindSpec` = one string, comma/array of strings, `"none"`/`false`, or an object `{ key, event?, preventDefault? }`. Merge over a built-in default map (user file wins). `"none"` disables a built-in.
- Parse chords: modifier+key strings (`ctrl+x`, `shift+return`, `<leader>`, `alt+u`…). Normalize `return`, `backspace`, arrows, `pageup`, `home`, etc.

### C.2 Input plumbing
**Scope:** `app/input.rs` `handle_key_input` (+ `read_input`), `app/mod.rs`.
- Add a `Keymap` consulted **after** overlay stack and **before** editor-globals; map chord → `TuiAction`/action id.
- Leader state machine: on leader press, set `pending_leader=true` + deadline; if second chord arrives in time, execute; on timeout, consume leader with no-op (or show hint per D). While `pending_leader`, raw typing is buffered (do not insert into editor).
- Must coexist with `KeyEventKind::{Press,Repeat,Release}` filtering already in place.

### C.3 Default bindings (mirror opencode; leader = ctrl+x)
- Session: `<leader>n` new, `<leader>l` list/resume, `<leader>c` compact, `<leader>g` timeline, `ctrl+r` rename session.
- Palette/help: `ctrl+p` command list, `<leader>h` help, `<leader>b` sidebar toggle.
- Copy/edit: `<leader>y` copy last/current message, `<leader>u` undo, `<leader>r` redo.
- Prompt: `shift+enter`/`ctrl+enter` = newline, `enter` = submit, `ctrl+k` delete-to-line-end, `ctrl+u` delete-to-line-start, `ctrl+alt+v` **plain-text paste**, `ctrl+e` external editor.
- Selection: `<leader>p` add-selection-to-prompt, `<leader>h` toggle conceal.
- Scroll: `pageup`/`pagedown`, `home`/`end`, `ctrl+alt+u`/`ctrl+alt+d` half-page (plus keep existing `Shift+J`).
- Toggles: thinking, timestamps, tool output (`/thinking` slash is already resumeable; add `<leader>t` family only if collision-free).
**Acceptance:** every listed action reachable via chord; `"none"` in `tui.toml` disables one; unknown chords fall through to the editor.

## D. Which-key style hints — P1
- When the leader is pending, overlay a small bottom hint listing the next-key prefixes registered for the leader (grouped). Reuse `overlay.rs` shell helpers (`render_overlay_shell`, `render_overlay_hint`) inside `app/layout/command_palette.rs` style primitives.
**Acceptance:** `ctrl+x` shows a hint strip; timeout removes it.

## E. Command palette v2 (frecency) — P1
**Scope:** `app/layout/command_palette.rs` (`CommandPaletteState`), `reducer.rs`, new `app/frecency.rs`.
- Expose all existing actions + new ones (B.3, C.3, toggles, diff, sessions) as commands with `{id, title, group, keybind}`.
- Fuzzy filter (reuse existing fuzzy matcher used by `@` picker / `fuzzy_score` in `cade-core::resources::palette`).
- Frecency ranking: persistent store keyed by command id → `(uses, last_use)`, score `uses / ln(age+2)`, written to `~/.cade/` or session file (follow existing persistence patterns, e.g. `SessionStore` locations), reloaded on palette open.
**Acceptance:** typing filters; recents first when both frequent and recent; selection executes the action with the same plumbing as the keybind.

## F. Sticky-follow scroll refinements — P1
**Scope:** `app/mod.rs` (scroll/follow fields, `draw_impl`), `app/input.rs`, `TuiSettings`.
- Add `scroll_speed` (default 3) and `scroll_acceleration { enabled }` (macOS-style: speed ramps with rapid wheel/scroll input).
- Bind half-page (`ctrl+alt+u`/`d`), page (`pageup`/`pagedown`), first/last (`ctrl+g`/`ctrl+alt+g`, `home`/`end`).
- Confirm opencode rule: auto-follow pauses when user scrolls away from bottom and resumes only when back at bottom (existing `follow`/`pending_lines` logic — keep, extend keys).
**Acceptance:** scrolled-up review stops auto-follow; reaching bottom resumes; page/home/end operations clamp within timeline (respect V-04 `max_skip`).

## G. Session timeline & list — P1
**Scope:** `crates/cade-tui/src/.../session_tree*` (existing picker), `app/mod.rs`, `render.rs` routing-level integration, `crates/cade-cli` command plumbing (`/resume`, `ctrl+x l`, `ctrl+x g`).
- Reuse the existing session-tree picker data to add a full-page/overlay view: session list (Home) and per-session message timeline (`<leader>g`) with fuzzy filter, `enter` to switch, `esc` close.
- Wire `ctrl+x n`/`l` to the established session switch flow in `cades-cli` (mirror how `/resume` and the session picker already switch conversations).
**Acceptance:** create/list/switch sessions via chords and palette without disturbing the current conversation.

## H. Toggles (thinking / timestamps / tool output / conceal) — P2
**Scope:** `app/mod.rs` fields, `stream.rs` event mapping, sidebar/hotkey bar.
- `display_thinking` (default on; toggle keeps current model’s reasoning blocks visible/hidden without losing them), `show_timestamps`, `reveal_tool_output` (`session_toggle_generic_tool_output` analogue), `conceal` (hides sensitive/user content).
- Persist in `TuiSettings` (`tui.toml`); expose in palette + hotkey bar line (extend the three-mode hotkey bar in `render.rs`).
**Acceptance:** each toggle changes rendering immediately, persists across restart, and is discoverable in the palette.

## I. Mouse & attention config — P2
**Scope:** `TuiSettings`, `app/input.rs` (mouse gate `EnableMouseCapture`), `crates/cade-cli` notifier path (`cade-tui` has `TerminalNotifier`).
- `mouse: bool` (default true). `false` → don’t enable mouse capture; native terminal selection/scrolling preserved (and in that mode, rely on `shift+click`/keyboard for copy fallback).
- `attention: { enabled, sounds }` (default off): when terminal is blurred (`FocusGained/FocusLost`) fire desktop notification on permission/question/error/session-complete events; play terminal bell optionally. Gate behind config; off = today’s behavior.
**Acceptance:** `mouse=false` allows native OS selection; `attention.enabled=true` produces notifications only for the four listed event classes.

## J. Auto-focus + prompt editing & stash — P2
**Scope:** `app/input.rs` (focus), `app/editor.rs`, new `app/prompt_stash.rs`.
- **Auto-focus:** typing any printable char when no overlay is open and input not focused pauses current mode handling and inserts into editor (open-mind: keep current editor behavior; add focus-state flag).
- **Stash:** `ctrl+x s`/`<leader>s` save unsent input to stash; restore via prompt (`<leader>` then select) after accidental submit/quit. Reuse collapsed-marker + history machinery; store in `SessionStore`/local JSON.
- **External editor:** `<leader>e`/`ctrl+e` open `$EDITOR` on current prompt; on close, replace prompt text.
- **Plain-text paste:** `ctrl+alt+v` reads clipboard text and inserts verbatim (bypasses image/attachment detection in B.4/A5 paths of `editor.rs`).
**Acceptance:** stash survives a submit (restore afterwards); external editor round-trips full prompt text; `ctrl+alt+v` pastes a copied file **path** as literal text.

## K. Copy & paste — hard acceptance criteria (all sections)
The following MUST hold end-to-end (Linux + macOS nominal, Windows best-effort):
1. Drag-select transcript → copy on release → paste works in any app (clipboard) **and** middle-click (PRIMARY) on Linux.
2. Selection highlight retained until unconsumed key/click; `ctrl+x p` quotes it into the prompt.
3. `Ctrl+V` pastes text; multi-line collapse to `[paste #N]` expands fully on submit losslessly (existing behavior preserved).
4. Pasting an image yields the existing `[image #N: WxH]` attachment path (unchanged).
5. `Ctrl+Alt+V` pastes literal text (paths stay paths).
6. OSC52 used first with tmux/screen passthrough; errors surface as ERROR toasts, never a false success.
7. Copy per `linux_clipboard_selection` (`clipboard`/`primary`/`both`, default `both`).

## L. Non-goals / explicitly out
- Rewriting the ratatui renderer or adopting a reactive JSX terminal framework.
- Implementing opencode’s plugin runtime or Home-page routing as a separate process.
- Changing the server/SSE protocol; all parity is client-side.

---

## M. Suggested implementation order
1. **B.1→B.2** (retain + dual-clipboard) — smallest, highest impact.
2. **B.3** (selection→quote) on top of retained selection.
3. **C** leader key + `keys.rs` config, then migrate hotkey-bar bindings to the map.
4. **E** palette frecency (reuses keymap action registry from C).
5. **F** scroll config/acceleration; **G** session views.
6. **D, H, I, J** remaining UX/config.

After each step: `cargo test -p cade-core -p cade-tui -p cade-server-lib -p cade-cli` and `cargo clippy --all-targets` must pass. TUI unit tests that require a TTY are expected to fail in headless CI — do not gate on them; add non-TTY unit tests for pure logic (keybind parsing, frecency scoring, selection→quote assembly, clipboard command construction).