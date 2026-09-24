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

## Interactive Operational Examples

### Example 1: Dynamic Responsive Layout & Window Resizing
CADE's dashboard dynamically adapts to small laptop displays, split screens, and ultra-wide monitors:
- **Collapsible Sidebar**: Click the toggle button (`◀` / `▶`) at the top of the sidebar. In collapsed mode, the sidebar shrinks to `w-16` (64px) with centered icon navigation, giving 100% focus to your active workspace.
- **Vertical Scroll Containment**: On displays with short vertical heights, the navigation links scroll smoothly (`overflow-y-auto`) while the brand header and bottom settings remain permanently pinned.
- **Chat Threads Toggle**: Inside the **Chat** page, click the `💬` toggle in the top header to collapse or reveal the conversation threads sidebar, giving your message timeline maximum horizontal width without clipping.

### Example 2: Managing Tools & Security Approvals
Navigate to the **Tools & Approvals** tab (`🛠`):
1. **Security Approvals**: When a background subagent requests permission to execute a high-impact operation (`bash`, `delete_file`), review its formatted JSON payload arguments and click **`✓ Approve`** or **`✕ Deny`** for instant zero-refresh processing.
2. **Tool Catalog**: Search the live catalog (`GET /v1/tools`) across `Native Core`, `MCP Mesh`, `Memory & Context`, and `Planning & Tasks` categories.
3. **MCP Gateway**: Inspect live server health states (`Ready`, `Failed` with diagnostic error, `Timeout`, `Disabled`) and explore the tools provided by each connected process.

### Example 3: Visualizing and Dispatching Workflows DAG
Navigate to the **Workflows DAG** tab (`🔄`):
1. Select a pipeline (e.g. `ci-validation`) from the grid.
2. The visual DAG canvas dynamically renders all sequential and fan-out steps with dependency connectors:
   ```text
   [ 1. cargo-check ] ────┬───➔ [ 2. cargo-clippy ]
                          └───➔ [ 3. cargo-test   ]
   ```
3. Click **`▶ Run Pipeline`** to dispatch execution with real-time SSE progress indicators and step completion alerts.

### Example 4: Managing Knowledge Graph Triples & SVG Canvas
Navigate to the **Memory Blocks** tab (`🧠`):
1. Open the **Knowledge Graph Triples** subtab to review structured facts stored in the SQLite database (`GET /v1/knowledge/edges`).
2. Use the **`+ Insert Knowledge Edge`** inline form to create new grounding facts (`AuthEngine` ➔ `validates` ➔ `BearerToken`).
3. Switch to the **Interactive Force-Directed Canvas** subtab to explore a live hardware-accelerated SVG orbit diagram dynamically centered around the CADE knowledge hub.

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

Output goes to `crates/cade-gui/dist/`. The `cade-server` binary bakes all assets in `dist/` directly into `RawDistAssets` at compile time via `rust-embed`.

## Offline Pre-Compiled Tailwind & Asset Delivery Architecture

To guarantee 100% offline, air-gapped functionality with zero external CDN dependencies, CADE serves all styling and client assets locally from the server binary:

```mermaid
flowchart TD
    subgraph Browser [Client Browser]
        DASH[GET /dashboard]
        TAIL[GET /dashboard/tailwind.js]
        WASM[GET /dashboard/cade-gui-*.wasm]
    end

    subgraph Server [cade-server API Gate (crates/cade-server)]
        SITE[DashboardSite Seam]
        ASSETS[DashboardAssets Loader]
        DEV_CHECK{CADE_DEV=1 or override?}
        FS[Filesystem: cade-gui/dist/]
        EMBED[Embedded: RawDistAssets via rust-embed]
    end

    DASH --> SITE
    TAIL --> SITE
    WASM --> SITE
    SITE --> ASSETS
    ASSETS --> DEV_CHECK
    DEV_CHECK -- Yes --> FS
    DEV_CHECK -- No (Production) --> EMBED
    EMBED -->|200 OK + no-cache| Browser
    FS -->|200 OK + no-cache| Browser
```

### Key Security & Reliability Features:
1. **Zero External CDN Dependencies**: The Tailwind CSS engine is bundled locally at `/dashboard/tailwind.js` (398 KB). Adblockers, firewalls, and air-gapped corporate environments cannot break dashboard styling.
2. **Trunk Copy-File Automation**: `crates/cade-gui/index.html` registers `<link data-trunk rel="copy-file" href="tailwind.js" />`, ensuring `tailwind.js` is automatically copied from source to `dist/` on every `trunk build`.
3. **Defensive SVG Layout Constraints**: The CADE logo in `crates/cade-gui/src/components/login.rs` includes explicit `width="32"`, `height="32"`, and inline style constraints (`min-width: 32px; min-height: 32px; flex-shrink: 0;`), alongside an `svg.w-8` fallback rule in `<style>`. Even if JavaScript or stylesheets lag, the logo can never expand to 100% viewport width.
4. **Dynamic Development Asset Overrides**:
   - `CADE_DEV=1`: When set, `cade-server` bypasses compile-time embedded assets and reads directly from `crates/cade-gui/dist/` on disk, allowing instant hot-reloads of GUI changes without recompiling `cade-server`.
   - `CADE_DASHBOARD_DIR=/path/to/dist`: Point the running server to an arbitrary custom frontend distribution folder.

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
