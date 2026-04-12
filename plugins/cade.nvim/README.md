# cade.nvim

Neovim plugin for [CADE](https://github.com/EzekTec-Inc/CADE) providing **inline AI code completions** (ghost text) powered by the CADE agent server.

## Features

- **Ghost-text completions** — suggestions appear inline at the cursor, rendered via `nvim_buf_set_extmark`.
- **Streaming SSE** — tokens render incrementally as the CADE server produces them, giving instant feedback.
- **Debounced trigger** — fires only after a configurable idle period so it never blocks typing.
- **Partial acceptance** — accept the full completion, one line, or one word at a time.
- **Zero dependencies** — uses only `vim.system` (Neovim ≥ 0.10) and `vim.uv` timers; no external Lua libraries.
- **Filetype filter** — optionally restrict completions to specific languages.
- **Toggle** — enable/disable at runtime with a single keymap.

## Requirements

| Requirement | Version |
|---|---|
| Neovim | ≥ 0.10 |
| CADE server | running on `localhost:8284` (or configured port) |
| `curl` | any modern version |

## Installation

### lazy.nvim (recommended)

```lua
-- ~/.config/nvim/lua/plugins/cade.lua
return {
  {
    "EzekTec-Inc/cade",
    -- Point lazy at the subdirectory that contains the plugin
    main = "cade",
    subdir = "plugins/cade.nvim",
    lazy = false,
    config = function()
      require("cade").setup({
        -- All values below are the defaults:
        server_port = 8284,
        -- agent_id = "",  -- override or set $CADE_AGENT_ID
        -- api_key  = "",  -- override or set $CADE_API_KEY
        debounce_ms = 300,
        lines_before = 50,
        lines_after  = 20,
        min_prefix   = 3,
        max_tokens   = 512,
        filetypes    = {},        -- empty = all filetypes
        hl_group     = "Comment", -- ghost-text highlight group
      })
    end,
  },
}
```

### Direct / local path

If you have the CADE repo checked out locally:

```lua
return {
  {
    dir  = "/path/to/CADE/plugins/cade.nvim",
    lazy = false,
    config = function()
      require("cade").setup({})
    end,
  },
}
```

## Configuration

`require("cade").setup(opts)` accepts:

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | `boolean` | `true` | Enable completions on startup |
| `server_port` | `number` | `8284` | CADE server port |
| `agent_id` | `string` | `$CADE_AGENT_ID` | Agent ID for `/v1/agents/:id/complete` |
| `api_key` | `string` | `$CADE_API_KEY` | Bearer token (optional) |
| `debounce_ms` | `number` | `300` | Milliseconds to wait after last keystroke |
| `lines_before` | `number` | `50` | Lines of prefix context sent to the server |
| `lines_after` | `number` | `20` | Lines of suffix context sent to the server |
| `min_prefix` | `number` | `3` | Skip completion when prefix is shorter than this |
| `max_tokens` | `number` | `512` | `max_tokens` forwarded to the CADE server |
| `model` | `string` | `""` | Optional model override (empty = agent default) |
| `filetypes` | `string[]` | `{}` | Allowlist of filetypes (empty = all) |
| `hl_group` | `string` | `"Comment"` | Highlight group for ghost text |

## Default keymaps

Set automatically by `plugin/cade.lua`:

| Insert-mode key | Action |
|---|---|
| `<Tab>` | Accept full completion |
| `<C-]>` | Accept one line |
| `<M-]>` | Accept next word |
| `<C-e>` | Dismiss completion |

| Normal-mode key | Action |
|---|---|
| `<leader>ct` | Toggle completions on/off |

`<Tab>` falls through to its normal behaviour when no ghost text is visible.

## How It Works

```
TextChangedI
    │  debounce (300 ms)
    ▼
trigger.lua ──► http.lua ──── curl POST /v1/agents/:id/complete ──► CADE server
                    │  SSE stream (text/event-stream)
                    ▼
             ghost.lua ──► nvim_buf_set_extmark  (inline virtual text)
```

1. `trigger.lua` listens to `TextChangedI`, resets a `vim.uv` timer on each keystroke.
2. After `debounce_ms` idle, `http.lua` opens a `curl` child process streaming SSE from the CADE `/v1/agents/:id/complete` endpoint.
3. Each `stream_delta` token is appended and `ghost.lua` re-renders the extmark at the current cursor position.
4. Accepting (`<Tab>`) calls `nvim_put` and clears the extmark; dismissing (`<C-e>`) just clears it.

## Module layout

```
plugins/cade.nvim/
├── lua/
│   └── cade/
│       ├── init.lua      ← public API (setup / accept / dismiss / toggle)
│       ├── config.lua    ← defaults and user-option merge
│       ├── ghost.lua     ← extmark renderer
│       ├── http.lua      ← async curl SSE client
│       └── trigger.lua   ← debounced TextChangedI handler
└── plugin/
    └── cade.lua          ← autocmds and keymaps (auto-loaded by Neovim)
```

## Environment variables

| Variable | Description |
|---|---|
| `CADE_AGENT_ID` | Agent ID used when `agent_id` is not set in `setup()` |
| `CADE_API_KEY` | Bearer token used when `api_key` is not set in `setup()` |

---

> **Theme export (legacy):** The old colorscheme-export functionality has been superseded by native `.tmTheme` support in the CADE TUI. Drop any `.tmTheme` file into `~/.cade/themes/` and activate it with `/theme <name>` inside CADE.
