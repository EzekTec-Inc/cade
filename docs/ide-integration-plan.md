# IDE Integration — Milestone Plan

The `cade-ide-mcp` crate is the long-term bridge between CADE agents and
the user's editor. It speaks MCP over stdio so any MCP-capable agent
(CADE, Claude Desktop, etc.) can introspect and drive the editor.

Status: **M-IDE-1a + M-IDE-1b + M-IDE-1c complete** (state layer, channel
abstraction, 7 read tools, 9 write tools over 7 callbacks, stdio binary,
TCP loopback adapter transport). Next up: **M-IDE-2** (VS Code extension).
Later phases add the JetBrains plugin.

---

## Architecture at a glance

```
┌─────────────────┐    MCP     ┌─────────────────┐  TCP loopback   ┌─────────────────┐
│   CADE agent    │ ◀────────▶ │  cade-ide-mcp   │ ◀─────────────▶ │  editor adapter │
│  (TUI / GUI)    │  stdio /   │  (this crate)   │  protocol.rs    │ (VS Code, etc.) │
└─────────────────┘  HTTP      └─────────────────┘                 └─────────────────┘
                                    │        │
                                    ▼        ▼
                           ┌──────────┐  ┌──────────────┐
                           │EditorState│  │ ChannelSlot  │  ← hot-swapped on
                           │(Arc clone)│  │(Arc<RwLock>) │    connect/disconnect
                           └──────────┘  └──────────────┘
```

* `EditorState` — thread-safe snapshot of the editor's live state (open
  files, active file, selection, diagnostics, workspace folders, visible
  range). Clones share storage.
* `EditorChannel` — the trait editor adapters implement. Lifecycle
  methods only in M-IDE-1a; M-IDE-1b adds mutating callbacks (apply
  edit, run task, terminal, debugger); M-IDE-1c provides
  `ProtocolEditorChannel`, a real impl backed by the TCP transport.
* `ChannelSlot` — hot-swappable `Arc<dyn EditorChannel>` wrapper;
  replaced whenever an adapter connects or disconnects.
* `IdeMcpServer` — rmcp `ServerHandler` wrapping both. Reads the state,
  routes mutations through the live `ChannelSlot`.

---

## Phases

### M-IDE-1a — Read-only tool surface ✅

State layer with shared storage, `EditorChannel` trait, seven read
tools, stdio binary. Complete as of commit `fd264a0c`.

Tools shipped:

| Tool                     | Description                                           |
| ------------------------ | ----------------------------------------------------- |
| `get_active_file`        | Path of the currently-focused file, or `null`.        |
| `get_open_files`         | List of all open editor tabs with their paths.        |
| `get_selection`          | Current selection `{ path, range, text }` or `null`.  |
| `get_diagnostics`        | Full diagnostic list with severity, source, code.     |
| `get_workspace_folders`  | Workspace roots currently open in the editor.         |
| `get_visible_range`      | `{ start_line, end_line }` of the active viewport.    |
| `get_file_content(path)` | Full buffer text + metadata for one open file.        |

### M-IDE-1b — Edit tool surface ✅

Mutating callbacks on `EditorChannel` + the corresponding MCP tools.
Complete as of commit `287ac2dd`.

Every callback defaults to JSON-RPC `-32601 method_not_found` with the
adapter label echoed in the message, so `NullEditorChannel` and any
future adapter that hasn't implemented a given callback refuses
loudly rather than silently succeeding.

| Tool            | Callback                               | Args                       |
| --------------- | -------------------------------------- | -------------------------- |
| `apply_edit`    | `apply_edit(ApplyEditRequest)`         | `{ path, text_edits }`     |
| `open_file`     | `reveal_file(path)`                    | `{ path }`                 |
| `set_selection` | `set_selection(path, range)`           | `{ path, range }`          |
| `save_file`     | `save(Some(path))`                     | `{ path }`                 |
| `save_all`      | `save(None)`                           | _(empty)_                  |
| `run_task`      | `run_task(name)`                       | `{ name }`                 |
| `run_terminal`  | `run_terminal(command)`                | `{ command }`              |
| `start_debug`   | `debug_control(Start { config })`      | `{ config }`               |
| `stop_debug`    | `debug_control(Stop)`                  | _(empty)_                  |

**Design note — collapsed callbacks.** `save(Option<String>)` handles
single-file and save-all with one callback, and `debug_control(DebugAction)`
handles start/stop with one callback. Adapters only override the
callbacks they actually support instead of duplicating
`method_not_found` across trivial variants.

### M-IDE-1c — Adapter transport protocol ✅

Newline-delimited JSON over TCP loopback (`127.0.0.1:<ephemeral>`).
Complete as of commit `035648ef`.

**Wire protocol** (`src/protocol.rs`):

| Direction          | Message                 | Purpose                                      |
| ------------------ | ----------------------- | -------------------------------------------- |
| adapter → server   | `Hello { label, version }` | Identify adapter; initiate session.        |
| server  → adapter  | `HelloAck { version }`  | Acknowledge; adapter may start sending state.|
| adapter → server   | `StateUpdate(snapshot)` | Full editor-state push (open files, active file, selection, diagnostics, workspace folders, visible range). |
| server  → adapter  | `CallbackRequest { id, op }` | MCP tool asks adapter to perform an editor operation. |
| adapter → server   | `CallbackResponse { id, result }` | Adapter's `Ok` / `Err` reply.    |

**Transport pieces**:

| Component              | File                        | Role                                                      |
| ---------------------- | --------------------------- | --------------------------------------------------------- |
| `TcpSink`              | `src/transport.rs`          | `MessageSink` over `OwnedWriteHalf`; newline-delimited.   |
| `ChannelSlot`          | `src/transport.rs`          | `Arc<RwLock<Arc<dyn EditorChannel>>>` — swapped on connect/disconnect. |
| `run_accept_loop`      | `src/transport.rs`          | Binds port, writes discovery file, accepts one adapter at a time. |
| `ProtocolEditorChannel`| `src/adapter_channel.rs`    | `EditorChannel` impl: sends `CallbackRequest`, awaits `CallbackResponse` via correlation-id map. |
| `IdeMcpServer::with_channel_slot` | `src/server.rs` | Server variant that reads the live channel from `ChannelSlot` on every tool call. |

**Discovery file** written at `~/.cade/ide/<pid>.json`:

```json
{ "pid": 12345, "addr": "127.0.0.1:54321" }
```

The VS Code extension reads this file to find which port to connect on.
File is removed on clean binary exit.

### M-IDE-2 — VS Code extension

TypeScript extension that spawns `cade-ide-mcp`, implements
`EditorChannel` over the cycle-1c protocol, and registers with VS Code
events (`onDidChangeActiveTextEditor`, `onDidChangeTextDocument`,
`onDidChangeDiagnostics`, …).

### M-IDE-3 — JetBrains plugin

Kotlin plugin mirroring the VS Code extension's feature set against the
IntelliJ Platform APIs.

---

## Operational notes

* **Logging is always stderr-only.** The MCP protocol owns stdout; any
  write to stdout corrupts the framing. `tracing_subscriber` is
  configured with `with_writer(std::io::stderr)` in the binary.
* **No filesystem fallback.** `get_file_content` errors when a path is
  not open. The editor adapter owns buffer state; agents must not bypass
  that.
* **`NullEditorChannel`** is the default channel before a real adapter
  attaches. It reports `is_connected() == false`; mutating tools added
  in M-IDE-1b will refuse with a structured MCP error until a real
  channel is installed.
* **TCP loopback transport** (M-IDE-1c): the binary binds an ephemeral
  port on `127.0.0.1` and writes `~/.cade/ide/<pid>.json` so the editor
  extension knows which port to connect on. The port is chosen by the OS
  (`:0` bind); connections are handled one at a time. On disconnect the
  server reverts to `NullEditorChannel`.

---

## How to register `cade-ide-mcp` as an MCP server today

Build the binary:

```sh
cargo build --release -p cade-ide-mcp --bin cade-ide-mcp
```

Add to `~/.cade/settings.json`:

```json
{
  "mcpServers": {
    "cade-ide": {
      "command": "/path/to/cade/target/release/cade-ide-mcp"
    }
  }
}
```

Launch CADE. `/mcp` will show `cade-ide` connected; `/tools` will list
the seven read tools prefixed with `cade-ide__`. They return empty or
`null` responses until a real editor adapter attaches and populates the
state.
