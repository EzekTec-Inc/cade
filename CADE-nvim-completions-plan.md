# CADE-nvim Code Completion Implementation Plan

> Saved: 2026-04-09

## Overview

Add AI code completion/suggestions (like Avante/Copilot) to the existing `CADE-nvim` MCP server.

## Current Architecture

```
~/.local/share/nvim/lazy/CADE-nvim/
├── plugin/cade.lua          # Auto-loaded: sets up the socket server at /tmp/nvim.pipe
└── mcp-server/
    ├── server.py            # FastMCP server with ide_* tools (Python + pynvim)
    └── requirements.txt     # pynvim, mcp
```

**Existing tools in server.py:**
- `ide_read_buffer` — Read full buffer with path and cursor
- `ide_read_selection` — Read visual selection (v, V, <C-v>)
- `ide_get_cursor_context` — Read lines around cursor
- `ide_propose_edit` — Find/replace in buffer (unsaved)
- `ide_apply_patch` — Apply unified diff patch (unsaved)

## Proposed New Tools

### 1. `ide_request_completion`

```python
@app.tool()
def ide_request_completion(
    max_context_lines: int = 100,
    trigger: str = "auto",  # "auto" | "manual"
) -> str:
    """Request a code completion at the current cursor position.
    
    Gathers context around the cursor (file content, cursor position,
    language) and returns it as a structured prompt for the LLM to
    generate completions.
    
    Returns JSON with:
    - file_path: str
    - language: str (filetype)
    - cursor_line: int
    - cursor_col: int
    - prefix: str (text before cursor on current line)
    - suffix: str (text after cursor on current line)
    - context_before: list[str] (lines before cursor)
    - context_after: list[str] (lines after cursor)
    """
```

### 2. `ide_insert_completion`

```python
@app.tool()
def ide_insert_completion(text: str, mode: str = "inline") -> str:
    """Insert completion text at the cursor position.
    
    Modes:
    - inline: Insert at cursor position
    - replace_line: Replace current line
    - replace_selection: Replace visual selection
    """
```

### 3. `ide_show_ghost_text`

```python
@app.tool()
def ide_show_ghost_text(text: str) -> str:
    """Display completion suggestion as ghost text (virtual text).
    
    The ghost text appears dimmed at the cursor position. User can:
    - Accept with <Tab>
    - Accept line with <C-]>
    - Accept word with <M-]>
    - Dismiss with <C-e> or <Esc>
    - Continue typing (auto-dismiss)
    """
```

### 4. `ide_clear_ghost_text`

```python
@app.tool()
def ide_clear_ghost_text() -> str:
    """Clear any displayed ghost text."""
```

## Lua-side Additions (plugin/cade.lua)

```lua
-- CADE Completion State
local M = {}
M.ns_id = vim.api.nvim_create_namespace("cade_completion")
M.pending_completion = nil

-- Show ghost text at cursor
function M.show_ghost(text)
  M.clear_ghost()
  M.pending_completion = text
  
  local cursor = vim.api.nvim_win_get_cursor(0)
  local row = cursor[1] - 1
  local col = cursor[2]
  
  local lines = vim.split(text, "\n")
  
  -- First line: inline virtual text
  if #lines >= 1 then
    vim.api.nvim_buf_set_extmark(0, M.ns_id, row, col, {
      virt_text = {{ lines[1], "Comment" }},
      virt_text_pos = "overlay",
    })
  end
  
  -- Remaining lines: virtual lines below
  if #lines > 1 then
    local virt_lines = {}
    for i = 2, #lines do
      table.insert(virt_lines, {{ lines[i], "Comment" }})
    end
    vim.api.nvim_buf_set_extmark(0, M.ns_id, row, 0, {
      virt_lines = virt_lines,
    })
  end
end

-- Clear ghost text
function M.clear_ghost()
  vim.api.nvim_buf_clear_namespace(0, M.ns_id, 0, -1)
  M.pending_completion = nil
end

-- Accept full completion
function M.accept()
  if not M.pending_completion then
    return false
  end
  
  local lines = vim.split(M.pending_completion, "\n")
  vim.api.nvim_put(lines, "c", true, true)
  M.clear_ghost()
  return true
end

-- Accept just the first line
function M.accept_line()
  if not M.pending_completion then
    return false
  end
  
  local lines = vim.split(M.pending_completion, "\n")
  vim.api.nvim_put({ lines[1] }, "c", true, true)
  
  -- Update pending to remaining lines
  if #lines > 1 then
    M.pending_completion = table.concat(vim.list_slice(lines, 2), "\n")
    M.show_ghost(M.pending_completion)
  else
    M.clear_ghost()
  end
  return true
end

-- Accept next word
function M.accept_word()
  if not M.pending_completion then
    return false
  end
  
  local word = M.pending_completion:match("^(%S+)")
  if word then
    vim.api.nvim_put({ word }, "c", true, true)
    M.pending_completion = M.pending_completion:sub(#word + 1)
    if M.pending_completion:match("^%s*$") then
      M.clear_ghost()
    else
      M.show_ghost(M.pending_completion)
    end
  end
  return true
end

-- Expose globally for MCP server to call
_G.cade_completion = M

-- Keymaps
vim.keymap.set("i", "<Tab>", function()
  if M.pending_completion then
    M.accept()
    return ""
  end
  return "<Tab>"
end, { expr = true, noremap = true })

vim.keymap.set("i", "<C-]>", function()
  if M.pending_completion then
    M.accept_line()
    return ""
  end
  return "<C-]>"
end, { expr = true, noremap = true })

vim.keymap.set("i", "<M-]>", function()
  if M.pending_completion then
    M.accept_word()
    return ""
  end
  return "<M-]>"
end, { expr = true, noremap = true })

vim.keymap.set("i", "<C-e>", function()
  if M.pending_completion then
    M.clear_ghost()
    return ""
  end
  return "<C-e>"
end, { expr = true, noremap = true })

-- Auto-clear on cursor move or text change
vim.api.nvim_create_autocmd({"CursorMovedI", "InsertLeave"}, {
  callback = function()
    M.clear_ghost()
  end,
})
```

## Design Approach

### Option A: MCP-driven (Recommended for v1)

```
User types → (pause) → CADE polls or user triggers manually
                            ↓
                    ide_request_completion
                            ↓
                    CADE generates completion via LLM
                            ↓
                    ide_show_ghost_text
                            ↓
                    User sees ghost text, accepts/rejects
```

**Pros:**
- Simple architecture — all intelligence in CADE
- Leverages existing MCP infrastructure
- Ghost text rendering is fast (local Lua)

**Cons:**
- Latency depends on LLM response time
- Requires CADE to have a polling/trigger mechanism

### Option B: Nvim-triggered (Future enhancement)

Neovim Lua code triggers completions directly via HTTP to a local endpoint, bypassing MCP round-trip.

## Keybindings

| Key | Action |
|-----|--------|
| `<Tab>` | Accept full suggestion |
| `<C-]>` | Accept current line only |
| `<M-]>` | Accept next word |
| `<C-e>` | Dismiss suggestion |
| `<M-\>` | Manually trigger completion (future) |

## Implementation Steps

### Phase 1: Server-side tools
1. Add `ide_request_completion` to `server.py`
2. Add `ide_show_ghost_text` to `server.py`
3. Add `ide_clear_ghost_text` to `server.py`
4. Add `ide_insert_completion` to `server.py`

### Phase 2: Lua-side rendering
1. Add ghost text module to `plugin/cade.lua`
2. Add keymaps for accept/dismiss
3. Add autocmds for auto-clear

### Phase 3: CADE integration
1. Add trigger mechanism (slash command? `/complete`)
2. Add background completion polling (optional)
3. Add completion caching

### Phase 4: Polish
1. Streaming completions (show character-by-character)
2. Multi-suggestion cycling (`<M-n>` / `<M-p>`)
3. Configurable debounce timing
4. Local model fallback for low-latency

## Files to Modify

1. `~/.local/share/nvim/lazy/CADE-nvim/mcp-server/server.py`
2. `~/.local/share/nvim/lazy/CADE-nvim/plugin/cade.lua`
3. CADE core (for trigger mechanism)

## Open Questions

1. **Trigger mechanism:** How should CADE know when to request a completion?
   - User-triggered via `/complete` command?
   - Automatic polling when Neovim is connected?
   - Neovim sends a notification on pause?

2. **Model selection:** Should completions use:
   - Same model as CADE chat?
   - Dedicated fast model (e.g., Claude Haiku, Qwen2.5-Coder)?
   - User-configurable?

3. **Context window:** How much context to include?
   - Full file? (may be large)
   - N lines before/after cursor?
   - Include other open buffers?

4. **Streaming:** Should we stream completions character-by-character for perceived speed?
