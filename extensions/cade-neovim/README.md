# cade-neovim

Neovim adapter for [cade-ide-mcp](../../crates/cade-ide-mcp/). Connects
Neovim to a running CADE agent and gives the agent live access to your
editor state — open buffers, selection, diagnostics, workspace root — and
lets it apply edits, open files, run terminals, and control nvim-dap.

---

## Requirements

| Requirement | Version |
|-------------|---------|
| Neovim | ≥ 0.10 |
| cade-ide-mcp binary | built from this repo |
| nvim-dap _(optional)_ | for debug_control ops |
| overseer.nvim _(optional)_ | for run_task ops |

---

## Installation

### 1 — Build the MCP server binary

```sh
cargo build --release -p cade-ide-mcp --bin cade-ide-mcp
```

### 2 — Register it in CADE settings

Add to `~/.cade/settings.json`:

```json
{
  "mcpServers": {
    "cade-ide": {
      "command": "/path/to/CADE/target/release/cade-ide-mcp"
    }
  }
}
```

### 3 — Install the plugin

**lazy.nvim:**

```lua
{
  dir = "/path/to/CADE/extensions/cade-neovim",
  config = function()
    require("cade_ide").setup()
  end,
}
```

**packer.nvim:**

```lua
use {
  "/path/to/CADE/extensions/cade-neovim",
  config = function()
    require("cade_ide").setup()
  end,
}
```

**Manual (symlink):**

```sh
make install          # symlinks into ~/.local/share/nvim/site/pack/cade/start/
```

Then add to your `init.lua`:

```lua
require("cade_ide").setup()
```

---

## How it works

```
CADE agent  ←── stdio MCP ──→  cade-ide-mcp
                                     │
                               TCP loopback
                              (~/.cade/ide/<pid>.json)
                                     │
                           cade-neovim plugin (this)
                                     │
                          Neovim API (buffers, LSP, dap)
```

1. `cade-ide-mcp` starts, binds an ephemeral port, writes a discovery
   file at `~/.cade/ide/<pid>.json`.
2. The plugin reads the discovery file, connects via TCP, and sends a
   `Hello` frame.
3. On every `BufEnter`, `TextChanged`, `CursorMoved`, `DiagnosticChanged`
   (debounced 50 ms) the plugin pushes a `state_update` frame containing
   open buffers, active file, selection, diagnostics, and workspace root.
4. When the agent calls a write tool (`apply_edit`, `reveal_file`, etc.),
   `cade-ide-mcp` sends a `callback_request` frame; the plugin dispatches
   it through `callback_handler.lua` and sends back a `callback_response`.
5. On disconnect the plugin auto-reconnects every 3 seconds.

---

## Configuration

```lua
require("cade_ide").setup({
  debounce_ms   = 50,    -- state-update debounce (default 50 ms)
  -- discovery_dir = "/custom/path",  -- override ~/.cade/ide (rarely needed)
  -- log = function(msg) print(msg) end,  -- custom logger
})
```

---

## Commands

| Command | Description |
|---------|-------------|
| `:CadeReconnect` | Manually reconnect to cade-ide-mcp |

---

## Supported callback ops

| Op | Effect |
|----|--------|
| `apply_edit` | `nvim_buf_set_text` — applies LSP-style text edits |
| `reveal_file` | `:edit <path>` |
| `set_selection` | Moves cursor + enters visual selection |
| `save` | `:w` (single file) or `:wa` (all) |
| `run_task` | overseer.nvim if installed, else `:make <name>` |
| `run_terminal` | `:split \| terminal <command>` |
| `debug_control` | nvim-dap `continue`, `terminate`, `step_over`, `step_into`, `step_out` |

---

## Running tests

```sh
cd extensions/cade-neovim
make test      # runs 45 headless Neovim tests
```
