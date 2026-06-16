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
1. **Dynamic Tool Discovery**: Any connected MCP server's tools are loaded dynamically at startup and registered directly into CADE's database.
2. **First-occurrence Prefix Stripping**: At the dispatch and evaluation level, CADE prefix-strips tool names by finding the first occurrence of the `__` namespace separator. This extracts the exact base tool name (e.g., `write_file` from `desktop-commander__write_file` or `shell` from `developer__shell`) dynamically.
3. **Canonical Mapping**: CADE maps aliases (such as `RunShellCommand` -> `bash`, `ReadFileGemini` -> `read_file`, `Replace` -> `edit_file`, `SearchFileContent` -> `grep`, `GlobGemini` -> `glob`, `edit_block` -> `edit_file`, `ide_propose_edit` -> `edit_file`, `ide_apply_patch` -> `apply_patch`, and `create_file` -> `write_file`) dynamically to canonicalize them before routing to native actions.

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

## Authoring an MCP server

A minimal Rust MCP server using `rmcp`:

```rust
// see crates/cade-mcp/ for the integration helpers
```

For a from-scratch MCP server in a new repo, use the
`rust-mcp-server-generator` skill or follow the `mcp-builder` skill.

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
