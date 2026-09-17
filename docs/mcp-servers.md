# MCP Servers

**MCP** (Model Context Protocol) is a stdio-based protocol that lets
external processes expose tools to LLM agents. CADE supports MCP both as
a **client** (consuming external servers) and as a **server**
(`cade-ide-mcp` exposes CADE's editor state to other MCP-capable agents).

## Configuring servers

Edit `~/.cade/settings.json` (global) or `.cade/settings.json` (project).
Both keys have the literal name `mcpServers` (camelCase, matches the
upstream spec):

```json
{
  "mcpServers": {
    "git": {
      "command": "/path/to/git-mcp-server"
    },
    "openviking": {
      "command": "/path/to/.venv/bin/python",
      "args": ["/path/to/openviking_mcp.py"]
    },
    "cade-ide": {
      "command": "/path/to/cade/target/release/cade-ide-mcp"
    },
    "remote-thing": {
      "url": "https://example.com/mcp",
      "headers": { "Authorization": "Bearer ..." }
    }
  }
}
```

Server entry fields:

| Field | Type | Purpose |
|---|---|---|
| `command` | string | Executable for stdio transport |
| `args` | string[] | Args passed to `command` |
| `env` | map | Extra env vars |
| `url` | string | HTTP transport (instead of stdio) |
| `headers` | map | Extra headers for HTTP transport |
| `write_tools` | string[] | Tools that mutate state (require permission) |
| `disabled` | bool | Skip on startup without removing the entry |
| `core_server` | bool | Mark server's tools as "core" — never pruned by ITS |

Project servers override global ones with the same key.

## Tool naming and dynamic auto-discovery

External tools are exposed with a `{server}__` prefix. So if the `git`
server exposes a `commit` tool, the LLM sees it as `git__commit`. This
prevents collisions and makes tool provenance unambiguous.

Rather than relying on hardcoded lists of third-party servers (like `desktop-commander` or `developer`) or prefix-specific rules, CADE employs a fully dynamic, prefix-agnostic auto-discovery mechanism:
1. **CapabilityMesh Dynamic Injection**: Any connected MCP server's tools are loaded dynamically at runtime through the `CapabilityMesh` seam (ADR-0020) and injected directly into the active LLM context and function-calling schemas.
2. **Tag-Driven Intelligent Tool Selection**: Servers with `"core_server": true` receive the `core_mcp` tag, ensuring they are permanently retained in the prompt schema and exempt from adaptive token pruning across long sessions.
3. **Stream Health & Invalidation**: If an active server process drops its pipe or fails to respond, `McpManager` performs exponential backoff reconnections (`MAX_RECONNECT_ATTEMPTS = 3`). If persistent failure occurs, the server is marked disabled, its tool schemas are dynamically invalidated, and any cached session permissions are purged immediately via `remove_session_allows_for_prefix()`.
4. **First-occurrence Prefix Stripping**: At the dispatch level, CADE prefix-strips tool names by finding the first occurrence of `__`, extracting the base tool name dynamically.
5. **Canonical Mapping**: CADE maps aliases (such as `RunShellCommand` -> `bash`, `ReadFileGemini` -> `read_file`, `Replace` -> `edit_file`, `SearchFileContent` -> `grep`, `GlobGemini` -> `glob`) dynamically to canonicalize them before routing to native actions.
6. **Provider Tool Capping & Priority Retention**: For model providers that enforce strict limits on total declared functions (such as OpenAI's 128-tool limit), CADE dynamically prioritizes tools carrying `x-cade: { core_server: true }` metadata or `core_mcp` database tags. All core MCP servers (e.g. `serena`, `headroom`, `cade-rag-mcp`) are dynamically preserved without any hardcoded tool prefixes or server names in the provider codebase.
7. **Provider Schema Adaptation**: MCP tool schemas are normalized again at each provider adapter seam before request dispatch. The OpenAI adapter guarantees that every function schema has top-level `type: "object"`, removes unsupported top-level schema combinators (`oneOf`, `anyOf`, `allOf`, `enum`, `const`, `not`), preserves real parameter names that collide with JSON Schema metadata keys (such as an issue field named `title`), converts OpenAPI-style `nullable: true` to JSON Schema nullable types, and prunes `required` entries that do not exist in `properties`.

## Inspecting

```bash
/mcp                      # list connected servers + their tools
/mcp-save <name>          # persist a runtime-connected server to settings.json
/connect <name>           # re-attach a saved server
/disconnect <name>        # stop and detach
```

Hot reload — `/hooks` reloads MCP, hooks, and permissions in one pass
without restarting the session.

## MCP UI Overlays

CADE supports native TUI rendering of interactive UI components returned by MCP tools via the `_meta.ui.resourceUri` field. When a tool call returns an output along with a UI resource URI, the CADE `mcp_ui_host` Lua plugin will intercept the response, fetch the associated resource payload (e.g. HTML or a structured JSON schema), and dynamically transpile it into `LuaWidget` components natively displayed as popups in the terminal or inside your IDE integrations.

This allows MCP servers to trigger rich interactive flows (such as form inputs, confirmation dialogs, or dynamic data tables) directly on the host machine without having to stream text continuously.

## Built-in MCP servers (in-tree)

| Server | Crate | Purpose |
|---|---|---|
| `cade-ide-mcp` | `crates/cade-ide-mcp/` | Bridge editor state to CADE |

`cade-ide-mcp` runs as a separate binary that:

1. Boots and binds an ephemeral TCP loopback port
2. Writes a discovery file at `~/.cade/ide/<pid>.json`
3. Speaks stdio MCP to the CADE agent
4. Speaks TCP loopback to an editor adapter (Neovim plugin, VS Code
   extension, JetBrains plugin)
5. Forwards `state_update` (open buffers, selection, diagnostics) and
   `callback_request` frames in both directions

Editor adapters live in `plugins/` (or external repos):

- `editors/neovim/` — Neovim plugin

## Recommended public servers

Compatible servers (configured in CADE's own dev environment):

| Server | Use for |
|---|---|
| `cade-rag` | Workspace semantic search (primary code-lookup tool) |
| `desktop-commander` | OS-level file / process operations |
| `lsp-mcp` | Language Server Protocol (`get_definition`, `get_references`) |
| `context7` | Library documentation lookup |
| `git-mcp-server` | Git operations |
| `github-mcp-server` | GitHub API |
| `structurizr-mcp` | Structurizr DSL validation, parsing, inspections, and view export to Mermaid/PlantUML |

## Structurizr MCP

Structurizr publishes an official MCP server at `https://docs.structurizr.com/ai/mcp`.
CADE can connect to the hosted read-only endpoint directly over HTTP:

```json
{
  "mcpServers": {
    "structurizr-mcp": {
      "url": "https://mcp.structurizr.com/mcp",
      "write_tools": [],
      "disabled": false
    }
  }
}
```

The hosted endpoint enables the DSL, Mermaid, and PlantUML tools. Server-side workspace create/read/update/delete tools are not enabled by default here, because they require a self-hosted Structurizr MCP server plus Structurizr server credentials.

To self-host all or selected Structurizr tools, run the official Docker image or Java WAR from Structurizr and point CADE at the local `/mcp` URL. For example:

```json
{
  "mcpServers": {
    "structurizr-mcp-local": {
      "url": "http://localhost:3000/mcp",
      "write_tools": [
        "delete_workspace_from_a_structurizr_server",
        "update_a_workspace_on_a_structurizr_server",
        "create_workspace_on_a_structurizr_server"
      ],
      "disabled": false
    }
  }
}
```

After editing MCP settings, run `/mcp reload` in CADE to hot-load the server.

## Authoring an MCP server

A minimal Rust MCP server using `rmcp`:

```rust
// see crates/cade-mcp/ for the integration helpers
```

For a from-scratch MCP server in a new repo, use the
`rust-mcp-server-generator` skill or follow the `mcp-builder` skill.

## Server-Managed (Centralized) MCP Architecture

In multi-session environments (such as concurrently using the CLI and an IDE extension), CADE avoids spinning up redundant, independent process trees for each stdio-based MCP server. 

Instead, when running alongside the centralized background daemon (`cade-server`):
1. **Central Management**: `cade-server` starts, monitors, and manages the lifecycle of all configured MCP server processes centrally.
2. **Tool Sharing**: The background daemon registers and exposes all MCP-derived tools through its central API gateway.
3. **Session Auto-Discovery**: When a CLI or REPL session connects to the server (`bg_server_connected`), it bypasses local `McpManager::start` execution to prevent port/process conflicts. It then automatically queries the server's central tool registry, conventions-maps, and attaches the active MCP tools dynamically to the active session.

## Headroom (Context Optimization & Compression)

CADE integrates with Headroom for context compression, token conservation, and prompt/response optimization.

### 1. On-Demand MCP Tools
Configure Headroom as an MCP server in `~/.cade/settings.json` or `.cade/settings.json`:

```json
{
  "mcpServers": {
    "headroom": {
      "command": "headroom",
      "args": ["mcp", "serve"],
      "core_server": true,
      "disabled": false
    }
  }
}
```

This exposes native tools:
- `headroom__headroom_compress`: Offloads verbose tool outputs, logs, or file content out-of-context and returns a hash reference.
- `headroom__headroom_retrieve`: Restores original uncompressed text on demand using the hash key.
- `headroom__headroom_stats`: Returns real-time compression metrics, tokens saved, and proxy status.

### 2. Transparent Full Coverage Proxy (`cade-headroom`)
For transparent network-level compression on all LLM requests (OpenAI/Anthropic):
- Use the launcher script at `scripts/cade-headroom` (or install to `~/.local/bin/cade-headroom`).
- It checks proxy health (`/readyz`), automatically launches `headroom proxy --port 8787` if not already running, and exports `OPENAI_BASE_URL="http://127.0.0.1:8787/v1"` and `ANTHROPIC_BASE_URL="http://127.0.0.1:8787"`.
- CADE's core AI layer automatically resolves these standard environment variables dynamically without requiring database mutations or hardcoding.
- When CADE exits, the launcher cleanly terminates the proxy (unless `HEADROOM_PERSIST=1` is set).
- Usage: `cade-headroom [args...]`

### 3. Health & Diagnostic Reporting in `/mcp`
When you open the MCP manager (`/mcp`), every configured server displays its real-time operational state:
- **Connected (`[ready · N tools]`):** Handshake succeeded, tools registered and active.
- **Failed (`[failed (error)]`):** Displays the exact startup or execution error (e.g. command not found, binary execution format error).
- **Timeout (`[timeout]`):** Connection timed out after 10 seconds.
- **Disconnected (`[disabled]`):** Server is disabled in configuration.

This centralized model eliminates process starvation, port collisions, and cold-start latency across concurrent terminals and editor buffers.

4. **Live Hot-Reload Settings Watcher**: To keep the centralized processes strictly synchronized with user actions, `cade-server` runs its own background file-system watcher. When a user toggles a server (via `Space` inside the `/mcp` overlay modal) or manually edits `.cade/settings.json`, the background daemon instantly detects the write, hot-reloads the settings, and starts or stops the running MCP subprocesses centrally. This ensures the central daemon always reflects your active settings without requiring any server or session restarts.

## Troubleshooting Common Loading Errors

When dynamically loading third-party MCP servers, extensions, or packages (using loaders like `jiti`), you may encounter path resolution errors.

### 1. `Cannot find module ... pi-ai/dist/index.js/compat`
- **Symptom:** The console throws an error claiming it cannot locate `/compat` inside `@earendil-works/pi-ai/dist/index.js`.
- **Cause:** This occurs on older versions of `@earendil-works/pi-ai` (specifically `< v0.80.0` like `v0.74.2`), where the compatibility subpath does not exist because all fallback functions (e.g. `complete`, `StringEnum`) are exported directly from the root module, but the environment resolver (such as `jiti`) tries to append `/compat` directly to the `main` file.
- **Resolution:**
  1. Add subpath aliases for `@earendil-works/pi-ai/compat` and `@mariozechner/pi-ai/compat` mapping directly to the root `piAiEntry` in `VIRTUAL_MODULES` and `_aliases` sections of `loader.js` (inside `@earendil-works/pi-coding-agent`).
  2. Map `"./compat"` inside `@earendil-works/pi-ai`'s `package.json` under the `"exports"` property.
  3. Create physical fallback `compat.js` and `compat.d.ts` proxy files at the package root and `dist` directory of `pi-ai` that re-export from the root entry point.

---

## Security notes

- MCP servers run with **the agent's privileges**. Trust the binaries
  you configure.
- Tools listed in `write_tools` get gated by the permission system
  exactly like native tools — `/yolo` bypasses prompts but path
  protection still applies.
- HTTP transport adds network attack surface; prefer stdio for local
  servers.
- The `cade-rag-guard` PreToolUse hook in this repo refuses raw
  `grep`/`read_file` calls until `cade-rag__index_workspace` has run —
  pattern-match if you want to enforce similar discipline elsewhere.
