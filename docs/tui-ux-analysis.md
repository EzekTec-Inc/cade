# Terminal UI Architecture vs CADE TUI — Analysis & Recommendations

> Status: research/advisory. Companion implementation spec: `docs/tui-ux-spec.md`.

## 1. How the Reference Terminal UI works (precise summary)

### 1.1 Technology & threading

- **Stack**: TypeScript, SolidJS (fine-grained reactive state), `@opentui/solid` terminal renderer. UI is a JSX component tree reconciled into terminal output; there is no manually-managed draw loop or diff-buffer cache — reactivity decides what re-renders.
- **Two threads**: a UI/main thread and a business thread.
  - Mode A (HTTP): boot a real HTTP server, talk over HTTP+SSE.
  - Mode B (default, remote/`server`): business logic runs in a **Worker thread** and the UI calls it through `createWorkerFetch()` — a shim that implements the `fetch`/SSE shapes as RPC over the worker message channel. UI stays responsive while the agent runs.
- **Routing**: two pages — **Home** (session list) and **Session** (conversation). `RouteProvider` switches; navigation is event-driven.
- **Context**: ~16 providers (Args, Exit, KV prefs, Toast, Route, SDK, Sync, Theme, Local, Keybind, PromptStash, Dialog, Command, Frecency, PromptHistory, PromptRef).

### 1.2 Layout ("split-footer" design)

- Main content = transcript inside a `ScrollBoxRenderable` with **sticky scroll**: it auto-follows new content, pauses when the user scrolls up, resumes when back at the bottom.
- Persistent **footer** = prompt input (`TextareaRenderable`) + status + `SubagentFooter` (live subagent states).
- **Right sidebar** when a session is active: session title, file-change history, modified-files list.
- **DiffViewer** as a feature plugin: split/unified, file-tree navigation.
- Messages render as markdown with syntax highlighting; tool calls as **inline tool rows** (`InlineToolRow`) with pending/complete/failed states; thinking/reasoning blocks toggleable; permission and question prompts are inline interactive overlays.

### 1.3 Interaction model

- **Leader-key system**: default leader `ctrl+x`, then a second key (e.g. `ctrl+x n` new session). `leader_timeout` (default 2000 ms). Exists to avoid clobbering terminal keybindings.
- **Configurable keybinds** in `tui.json` (JSON, merged over defaults; values support multi-key strings, arrays, `"none"` to disable, and objects with `key`/`event`/`preventDefault`/`fallthrough`).
- **Command palette** (`ctrl+p`): VS-Code-like fuzzy searchable list of 40+ commands, ranked by **frecency** (FrecencyProvider) — recently & frequently used commands float to top.
- **Prompt autocomplete**: `@` fuzzy file/agent/slash completion, line ranges (`@file.ts#10-20`), `Tab` to complete, arrow navigation.
- **Prompt editing**: multiline, shift-enter newline, external editor (`ctrl+e`/`<leader>e`), input history, **stash** (unsaved prompts cached and restorable), undo/redo, word/emacs navigation.
- **Scroll**: PageUp/PageDown, Home/End, half-page jumps; `scroll_acceleration` (macOS-style) and `scroll_speed` config.
- **Focus**: auto-focus input on typing (except when terminal panel open on desktop).
- **Toggles**: thinking visibility, timestamps, tool output, conceal.
- **Attention**: config-gated desktop notifications + sounds when terminal is blurred.
- **tui.json knobs**: `theme`, `keybinds`, `leader_timeout`, `scroll_acceleration`, `scroll_speed`, `diff_style` (`auto`/`stacked`), `cursor` (style/blinking), `mouse` (default `true`; `false` restores native terminal selection/scrolling), `attention`, `linux_clipboard_selection` (`clipboard`/`primary`/`both`).

### 1.4 Copy & paste (as it actually behaves)

- **Copy**:
  - OSC52 written first (with tmux/screen passthrough `\x1bPtmux;…`), then OS-native (osascript / `wl-copy` / `xclip -selection clipboard` / `xsel` / PowerShell via stdin), then `clipboardy`.
  - On Linux, by default copies to **both** clipboard and PRIMARY selection (middle-click) — configurable.
  - Drag-select in transcript copies on mouse release and **retains** the highlight until the next unconsumed key or click.
  - Copy is acknowledged by a toast; failures are surfaced (errors propagated, not silently swallowed).
- **Paste**:
  - Bracketed paste everywhere; also forwards empty bracket hints for image-only clipboards on Windows Terminal.
  - Clipboard read prefers **images** (macOS osascript PNG; Windows PowerShell; `wl-paste -t image/png`; `xclip -selection clipboard -t image/png`), falls back to text.
  - `Ctrl+V` pastes with attachment/image detection (`pasteInputText`); `Ctrl+Alt+V` is a **plain-text paste** bypass that inserts literals (e.g. a copied file path stays a path, rather than becoming an attachment).
  - Multi-line pastes are collapsed into a `[pasted #N]` chip, stashed, and expanded back into the prompt when the message is submitted.
  - **Copy-selection-into-prompt**: `ctrl+x p` (`prompt.add_selection`) takes the retained selection, prefixes each line `> `, and inserts it as a collapsed pasted block — quote-as-context into the next message.

## 2. How CADE’s TUI works today (precise summary)

See `crates/cade-tui` and `crates/cade-cli/src/cli/repl/turn_loop/stream.rs` for the authoritative shape.

- **Stack**: Rust + ratatui. Single full-screen repaint driven by `content_version`-invalidated, pre-wrapped timeline cache (`TimelineLayoutEngine`). All output goes through `TuiApp` (alternate screen, raw mode); no partial-viewport hacks.
- **Threading analogue**: SSE I/O is decoupled from rendering by an unbounded `mpsc` channel + a dedicated UI consumer task (`stream.rs`), so the SSE loop never blocks on draw/lock. (Decouples I/O from rendering cleanly.)
- **State**: `TuiApp` owns lines (`RenderLine`), streaming, thinking, scroll/follow, budget/cost, toasts, overlays, clipboard. Mutations funnel through `push` (`state.rs`).
- **Layout**: 8-slot vertical layout; left content + optional right **sidebar** (agent/model/cwd, cost gauge, status, plan, modified files) at width ≥ breakpoint, else breadcrumb bar. Header pinned top, status row, input area with mode badge, separator lines, footer + hotkey bar. Plan panel overlays content.
- **Interaction**: `read_input`/`handle_key_input`; `OverlayComponent` stack (permission, questions, palette, copy overlay, pickers, help); autocomplete towers for `/`, `@`, agents, MCPs; input modes (`!`, `!!`, `/`); mouse drag-select → copy, click-to-copy highlight; V-04 scroll clamping; `Shift+J` follow; toasts with decay.
- **Theming**: shared `cade_core::resources::Theme`, `ThemeColorsExt`, syntect syntax highlighting, Nerd Font icons with ASCII fallback, markdown via pulldown-cmark.
- **Existing polish**: all `docs/ui-ux-polish-plan.md` items landed (timeline expansion toggle, anchor-retaining resize, floating `/` autocomplete, cost gauge, toast decay, `@` picker).
- **TUI config**: `TuiSettings` in `cade-core/src/settings/tui.rs`, loaded from `tui.toml` (separate from `settings.json`).

## 3. Quantitative comparison

| Dimension | Reference Reactive TUI | CADE |
|---|---|---|
| Renderer | SolidJS + @opentui (reactive component tree) | ratatui retained-timeline, full-redraw cache |
| Backend split | UI thread ⇄ business thread (RPC or HTTP) | SSE channel + UI consumer task |
| Routes/pages | Home (sessions) + Session | Single conversation view (+ pickers) |
| Leader key + which-key | ✅ ctrl+x + hints | ❌ none |
| Configurable keybinds | ✅ tui.json (merge, none, arrays) | ⚠️ `tui.toml` settings, hardcoded bindings |
| Command palette + frecency | ✅ ctrl+p, 40+ cmds, fuzzy+frecency | ⚠️ ctrl+p palette, no frecency |
| Select → copy | ✅ w/ retain + toast | ✅ w/ transient highlight, no toast path parity |
| Select → quote into prompt | ✅ ctrl+x p | ❌ |
| Plain-text paste bypass | ✅ ctrl+alt+v | ❌ (all paste goes through collapse/attachment path) |
| Clipboard+PRIMARY (Linux) | ✅ default `both` | ⚠️ OSC52+arboard only |
| Sticky/auto-follow scroll | ✅ + acceleration | ✅ Shift+J follow + pending badge; no acceleration config |
| Half-page/page jumps | ✅ ctrl+d/u, pgup/pgdn, home/end | ⚠️ partial (page/home/end not all bound) |
| Diff viewer | ✅ plugin (split/unified) | ✅ DiffViewEngine (sidebar/fullscreen) |
| Thinking toggle | ✅ (`/thinking`) | ⚠️ reasoning rendered; toggle via palette |
| Session timeline | ✅ ctrl+x g | ⚠️ session tree picker exists |
| Auto-focus on typing | ✅ | ⚠️ |
| Attention (notif/sound) | ✅ config-gated | ⚠️ TerminalNotifier exists; no unified gate/sounds |
| Mouse disable for native select | ✅ config | ❌ |
| External editor | ✅ ctrl+e | ⚠️ unknown/partial |

## 4. Recommendations to make CADE best-in-class

Ranked by (impact × effort), all detailed in `docs/tui-ux-spec.md`.

1. **Selection & clipboard parity** (highest user-visible value): retained selection highlight, copy to clipboard **+ Linux PRIMARY**, copy-toast w/ real failure, and *“add selection to prompt as `>` quoted context”* command.
2. **Leader-key system (`ctrl+x`) + configurable keybinds** (tui.toml/tui.json merge incl. `"none"`, arrays) + which-key-style hint popup. This unblocks deep keyboard workflows without colliding with terminal shortcuts.
3. **Command palette v2**: fuzzy filter + **frecency ranking** + all existing commands/actions exposed.
4. **Sticky-follow scroll refinements**: configurable `scroll_speed`/`scroll_acceleration`, half-page (`ctrl+d`/`ctrl+u`), `Home`/`End`, pause-on-scroll-up.
5. **Session timeline + Home page**: a session list/timeline view (`ctrl+x l`/`ctrl+x g`) reusing the existing session-tree picker data.
6. **Prompt editing + stash**: input history, unsent-prompt stash with restore, external `$EDITOR`, plain-text paste bypass.
7. **Toggles**: thinking/timestamps/tool-output reveal, persisted in `tui.toml`.
8. **Mouse & attention config**: `mouse = false` → native terminal selection/scrolling; attention (desktop notify + sounds) behind config.
9. **Auto-focus input on typing** when no overlay is open.

> Copy & paste is called out separately because it is both the single biggest parity gap today and the easiest to get wrong — the replication spec gives it its own section with hard acceptance criteria.
