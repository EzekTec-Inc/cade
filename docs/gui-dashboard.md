# GUI Dashboard

CADE ships a WASM-based dashboard at `/dashboard`. It mirrors most of
the TUI's capabilities through a beautiful, responsive browser UI built with **Dioxus v0.5** (HTML/CSS).

## Quick start

1. Start `cade-server` as usual.
2. Open `http://localhost:8284/dashboard` in any modern browser.
3. The dashboard is **public / unauthenticated** by default — see the
   auth section below for production.

The dashboard is bundled with the server via `rust-embed` — no separate
deployment needed.

## What it does

- **Connects to the native REST/SSE API** with zero-latency WASM reactive signals (Dioxus v0.5).
- **Global Command Palette (`Cmd+K` / `Ctrl+K`)**: Instant fuzzy search across all 13 views, autonomous agents, and MCP tools with full keyboard navigation.
- **Multi-Model Arena Matrix**: Simultaneous 2–4 lane stream multiplexing with real-time Tokens/sec (`tok/s`) gauges, Time-To-First-Token (`TTFT`) latency meters, and synchronized comparative diffing.
- **Chat & Execution Traces**: Collapsible `<reasoning>` thought accordions with elapsed duration badges, structured tool execution cards with status badges and duration metrics, and working single-click clipboard copy triggers.
- **Swarm Topology & Workflow DAG Canvas**: Interactive multi-agent supervisor/worker tree with token consumption metrics, active state pulses, and live IPC Intercom Telemetry Stream & Supervisor Log.
- **3-Tier Context Allocation & Knowledge Graph Studio**: Stacked context allocation heatmap (Pinned, Short-Term, Long-Term tiers) against 128k–1M token budgets, Knowledge Graph Triples (`Entity` ➔ `Relation` ➔ `Target`) browser, and hybrid semantic vector search test playground.
- **Developer API Workbench**: Embedded, copyable SDK examples in Rust (`cade-sdk`), Node.js, Python, and cURL with syntax highlighting.
- **Full Conversation & Provider Management**: Create, list, switch, and delete chat sessions; configure live model providers (Anthropic, OpenAI, Gemini, Ollama, OpenRouter).

## Layout

```
┌──────────────┬────────────────────────────┐
│ Sidebar      │ Timeline                   │
│  - agents    │  - chat history            │
│  - status    │  - streaming reveal        │
│  - plan      │  - tool cards              │
├──────────────┤  - subagent cards          │
│              │                            │
│              ├────────────────────────────┤
│              │ Editor (input bar)         │
│              │  - / triggers palette      │
└──────────────┴────────────────────────────┘
```

Overlays open on top:

- Command palette (`Ctrl+P` or `/` at empty input)
- Memory viewer / editor
- Checkpoints browser
- Artifacts list
- MCP / tools / skills lists
- Model picker, theme picker, permissions, hooks
- Pricing, stats, context breakdown

## Command palette

Press `Ctrl+P`. Same triggers as the TUI palette (see
[slash-commands.md](slash-commands.md)). Some commands surface a toast
saying *"available in the CADE CLI/TUI — GUI panel coming soon"* when
they require a terminal-only feature (e.g. `/mouse`, `/export`).

The palette uses `cade-core::resources::palette::CMD_DEFS` for entries —
adding a new entry there makes it discoverable in **both** the TUI and
GUI palettes.

## Keyboard shortcuts (GUI)

| Key | Action |
|---|---|
| `Cmd+K` / `Ctrl+K` | Open global Command Palette & Search Overlay |
| `Ctrl+N` | Start a new chat session / jump to Chat |
| `Ctrl+,` | Open global Settings panel |
| `Esc` | Close palette/modal or return focus to Chat |
| `Enter` | Send message / execute selected command |
| `Shift+Enter` | Multi-line line break in chat input |
| `↑` `↓` (in palette) | Navigate active command selection |
| `Ctrl+S` (in memory editor) | Save memory block changes |
| `↓` button (timeline) | Scroll-to-bottom + re-enable auto-scroll |

Auto-scroll: scrolls with new content unless the user scrolls **up**,
which disables auto-scroll until the next manual `↓` click.

## Inline question widget

When a tool emits an `ask_user_question`, the GUI renders an inline
widget directly in the timeline (instead of a blocking modal). Single-
or multi-select; arrow keys move the highlight.

## Auth (production)

The dashboard route is exempt from the auth middleware in `cade-server`
**by design** — local-first means trust the loopback. For a public
deployment:

1. Reverse-proxy the server behind nginx / Caddy / Cloudflare
2. Add basic auth or OIDC at the proxy layer
3. Or wrap the entire CADE server in WireGuard / Tailscale
4. Set `CADE_ALLOWED_ORIGIN` to your origin to harden CORS

The `/v1/*` API routes still require `Authorization: Bearer <token>`
even when the dashboard is open — `CADE_API_KEY` controls this.

## Build / dev

The dashboard lives in `crates/cade-gui/`. Build with `trunk`:

```bash
cd crates/cade-gui
trunk build --release
```

Output goes to `crates/cade-gui/dist/`. The `cade-server` build script
watches that directory and re-embeds on next compile (see
`crates/cade-server/build.rs`).

For dev iteration without a full server rebuild, point `trunk serve` at
a running `cade-server` instance:

```bash
cd crates/cade-gui
trunk serve --port 9000 --proxy-backend http://localhost:8284/v1/
```

Then open `http://localhost:9000`.

## Architecture notes

- **Pure-Rust SSE parser** — `crates/cade-gui/src/sse.rs` is wasm-free and
  unit-testable on native; the WASM adapter wraps `fetch()` +
  `ReadableStream`.
- **Session state machine** — `crates/cade-gui/src/session.rs` owns all
  state; the render loop is a pure projection. Heavily covered by tests
  (310+).
- **Pure components** — `app/views.rs`, `app/overlays/*` take state by
  reference and emit `AppAction` events; no async work in render.
- **API types shared** — `cade-api-types` (re-exported by both
  `cade-server` and `cade-gui`) keeps the wire format single-sourced.

## Known GUI-only quirks

- `/compaction-model` (no arg) shows usage error instead of clearing
  the override — clear via the CLI to avoid surprise.
- Some commands marked `Unsupported` in the palette intentionally route
  through the CLI/TUI today.
- File-paste images are not yet supported (CLI-only via `Ctrl+V`).
