# CADE-nvim Option B — Direct HTTP Inline Completions Plan

> Generated: 2026-04-13
> Supersedes: `CADE-nvim-completions-plan.md` (Option A / MCP-driven approach)

---

## Overview

Option B wires Neovim directly to the existing `POST /v1/agents/:id/complete` SSE endpoint,
mirroring the VS Code extension's architecture. No MCP round-trip. No changes to `server.py`.
Neovim Lua owns the trigger, HTTP fetch, ghost-text rendering, and acceptance keymaps entirely.

```
User types
    │
    ▼ TextChangedI (debounced 300 ms)
cade/trigger.lua
    │  build prefix/suffix from buffer + cursor
    ▼
cade/http.lua
    │  vim.system { "curl", "--no-buffer", "-N",
    │               POST /v1/agents/:id/complete }
    │  stdout callback → SSE line parser
    ▼
cade/ghost.lua
    │  accumulate tokens → nvim_buf_set_extmark
    │    virt_text  (inline, first line)
    │    virt_lines (below,  remaining lines)
    ▼
User sees ghost text
    │
    ├─ <Tab>   → ghost.accept()      (insert all)
    ├─ <C-]>   → ghost.accept_line() (insert first line)
    ├─ <M-]>   → ghost.accept_word() (insert next word)
    └─ <C-e>   → ghost.dismiss()     (clear)
```

---

## Repository Layout After Implementation

```
~/.local/share/nvim/lazy/CADE-nvim/
├── plugin/
│   └── cade.lua          ← extend: load cade module, register autocmds + keymaps
└── lua/
    └── cade/
        ├── init.lua      ← NEW: public API (setup, accept, accept_line, accept_word, dismiss)
        ├── config.lua    ← NEW: defaults + user config merge
        ├── ghost.lua     ← NEW: extmark ghost-text renderer
        ├── http.lua      ← NEW: async curl SSE client
        └── trigger.lua   ← NEW: debounced TextChangedI handler
```

`mcp-server/server.py` — **untouched**.

---

## Phase 1 — Core Infrastructure (ghost text + HTTP client)

### 1A. `lua/cade/config.lua`

Holds defaults and the merged user config. No side effects on `require`.

```lua
local M = {}

M.defaults = {
  enabled       = true,
  server_port   = 8284,
  agent_id      = vim.env.CADE_AGENT_ID or "",
  api_key       = vim.env.CADE_API_KEY  or "",
  lines_before  = 50,      -- prefix context lines
  lines_after   = 20,      -- suffix context lines
  debounce_ms   = 300,     -- ms to wait after last keystroke
  min_prefix    = 3,       -- skip if prefix shorter than this
  max_tokens    = 512,     -- forwarded to server
  model         = "",      -- optional model override (empty = agent default)
  filetypes     = {},      -- allowlist; empty = all filetypes
  hl_group      = "Comment", -- ghost-text highlight group
}

M.current = vim.deepcopy(M.defaults)

function M.setup(opts)
  M.current = vim.tbl_deep_extend("force", M.defaults, opts or {})
end

function M.get() return M.current end

return M
```

---

### 1B. `lua/cade/ghost.lua`

Pure extmark renderer. No I/O, no timers. Called by `http.lua` and keymaps.

```lua
local M = {}
local api  = vim.api
local ns   = api.nvim_create_namespace("cade_ghost")

-- State
M._pending  = nil   -- full accumulated string or nil
M._buf      = nil   -- buffer where ghost text lives
M._mark_ids = {}    -- list of extmark IDs (for targeted clearing)

-- ── Render ──────────────────────────────────────────────────────────────────

--- Show ghost text at the current cursor position.
--- Called incrementally as SSE tokens arrive (replaces previous ghost text).
---@param text string  Full accumulated completion text so far
function M.show(text)
  if not text or text == "" then return end
  local buf    = api.nvim_get_current_buf()
  local cursor = api.nvim_win_get_cursor(0)  -- {row, col}, 1-based row
  local row    = cursor[1] - 1               -- 0-based
  local col    = cursor[2]

  M.clear()   -- remove previous extmarks before re-drawing

  M._pending = text
  M._buf     = buf
  M._mark_ids = {}

  local lines = vim.split(text, "\n", { plain = true })

  -- First line: inline virt_text overlaid at cursor column
  if #lines >= 1 then
    local id = api.nvim_buf_set_extmark(buf, ns, row, col, {
      virt_text          = { { lines[1], M._hl() } },
      virt_text_pos      = "inline",
      undo_restore       = false,
      invalidate         = true,
    })
    table.insert(M._mark_ids, id)
  end

  -- Remaining lines: virt_lines below the cursor row
  if #lines > 1 then
    local virt_lines = {}
    for i = 2, #lines do
      table.insert(virt_lines, { { lines[i], M._hl() } })
    end
    local id = api.nvim_buf_set_extmark(buf, ns, row, 0, {
      virt_lines       = virt_lines,
      virt_lines_above = false,
      undo_restore     = false,
      invalidate       = true,
    })
    table.insert(M._mark_ids, id)
  end
end

--- Clear all ghost text from whichever buffer it lives in.
function M.clear()
  if M._buf and api.nvim_buf_is_valid(M._buf) then
    api.nvim_buf_clear_namespace(M._buf, ns, 0, -1)
  end
  M._pending  = nil
  M._buf      = nil
  M._mark_ids = {}
end

-- ── Acceptance ──────────────────────────────────────────────────────────────

--- Accept the full completion.
---@return boolean  true if a completion was accepted
function M.accept()
  if not M._pending then return false end
  local text = M._pending
  M.clear()
  local lines = vim.split(text, "\n", { plain = true })
  api.nvim_put(lines, "c", true, true)
  return true
end

--- Accept only the first line of the completion.
---@return boolean
function M.accept_line()
  if not M._pending then return false end
  local lines = vim.split(M._pending, "\n", { plain = true })
  local first = lines[1]
  if #lines > 1 then
    M._pending = table.concat(lines, "\n", 2)
    M.show(M._pending)
  else
    M.clear()
  end
  api.nvim_put({ first }, "c", true, true)
  return true
end

--- Accept the next word of the completion.
---@return boolean
function M.accept_word()
  if not M._pending then return false end
  -- Match leading whitespace + word characters
  local word, rest = M._pending:match("^(%s*%S+)(.*)")
  if not word then return false end
  if rest == "" or rest:match("^%s*$") then
    M.clear()
  else
    M._pending = rest
    M.show(M._pending)
  end
  api.nvim_put({ word }, "c", true, true)
  return true
end

--- Dismiss ghost text without accepting.
function M.dismiss()
  M.clear()
end

--- True when ghost text is currently visible.
function M.is_visible()
  return M._pending ~= nil
end

-- ── Internal ─────────────────────────────────────────────────────────────────

function M._hl()
  return require("cade.config").get().hl_group
end

return M
```

---

### 1C. `lua/cade/http.lua`

Async curl SSE client. Calls `on_token(text)` for each accumulated chunk,
`on_done()` on `[DONE]`, `on_error(msg)` on failure. Returns a `cancel()` function.

```lua
local M = {}

---@param prefix   string
---@param suffix   string
---@param language string
---@param on_token fun(accumulated: string)
---@param on_done  fun()
---@param on_error fun(msg: string)
---@return fun()  cancel  Call to abort the in-flight request
function M.fetch(prefix, suffix, language, on_token, on_done, on_error)
  local cfg = require("cade.config").get()

  if cfg.agent_id == "" then
    on_error("cade.nvim: agent_id not configured")
    return function() end
  end

  local url = string.format(
    "http://127.0.0.1:%d/v1/agents/%s/complete",
    cfg.server_port,
    cfg.agent_id
  )

  local body = vim.json.encode({
    prefix     = prefix,
    suffix     = suffix,
    language   = language,
    max_tokens = cfg.max_tokens,
    model      = cfg.model ~= "" and cfg.model or nil,
  })

  local headers = {
    "-H", "Content-Type: application/json",
    "-H", "Accept: text/event-stream",
  }
  if cfg.api_key ~= "" then
    vim.list_extend(headers, { "-H", "Authorization: Bearer " .. cfg.api_key })
  end

  local cmd = vim.list_extend(
    { "curl", "--silent", "--no-buffer", "-N", "-X", "POST",
      "-d", body },
    headers
  )
  table.insert(cmd, url)

  local accumulated = ""
  local buffer      = ""   -- partial SSE line accumulator
  local done        = false

  local handle = vim.system(cmd, {
    text   = true,
    stdout = function(err, chunk)
      if done then return end
      if err then
        vim.schedule(function() on_error(err) end)
        return
      end
      if not chunk then return end  -- stream closed

      buffer = buffer .. chunk
      -- Split on newlines; keep trailing partial line in buffer
      local lines = vim.split(buffer, "\n", { plain = true })
      buffer = table.remove(lines) or ""

      for _, line in ipairs(lines) do
        line = vim.trim(line)
        if line:sub(1, 6) == "data: " then
          local payload = line:sub(7)
          if payload == "[DONE]" then
            done = true
            vim.schedule(on_done)
            return
          end
          local ok, obj = pcall(vim.json.decode, payload)
          if ok and obj then
            if obj.message_type == "stream_delta" and obj.content then
              accumulated = accumulated .. obj.content
              local snap = accumulated
              vim.schedule(function() on_token(snap) end)
            elseif obj.error then
              done = true
              vim.schedule(function() on_error(obj.error) end)
              return
            end
          end
        end
      end
    end,
  }, function(result)
    -- on_exit: curl process ended
    if not done then
      if result.code ~= 0 then
        vim.schedule(function()
          on_error("cade.nvim: curl exited with code " .. result.code)
        end)
      else
        vim.schedule(on_done)
      end
    end
  end)

  return function()
    done = true
    pcall(function() handle:kill(9) end)
  end
end

return M
```

---

## Phase 2 — Trigger & Debounce

### 2A. `lua/cade/trigger.lua`

Watches `TextChangedI` / `CursorMovedI`. Debounces with `vim.uv.new_timer()`.
Cancels any in-flight request before starting a new one.

```lua
local M = {}

local ghost   = require("cade.ghost")
local http    = require("cade.http")

local _timer  = nil   -- vim.uv timer handle
local _cancel = nil   -- cancel() from last http.fetch call

-- ── Internal helpers ─────────────────────────────────────────────────────────

local function cancel_inflight()
  if _cancel then
    _cancel()
    _cancel = nil
  end
  ghost.clear()
end

local function build_context()
  local cfg    = require("cade.config").get()
  local buf    = vim.api.nvim_get_current_buf()
  local cursor = vim.api.nvim_win_get_cursor(0)  -- {row, col} 1-based row
  local row    = cursor[1]                        -- 1-based
  local col    = cursor[2]                        -- 0-based byte offset

  local total_lines = vim.api.nvim_buf_line_count(buf)

  -- Prefix: lines_before lines up to (and including) cursor column
  local prefix_start = math.max(1, row - cfg.lines_before)
  local prefix_lines = vim.api.nvim_buf_get_lines(buf, prefix_start - 1, row, false)
  -- Trim the last element to cursor column
  if #prefix_lines > 0 then
    prefix_lines[#prefix_lines] = prefix_lines[#prefix_lines]:sub(1, col)
  end
  local prefix = table.concat(prefix_lines, "\n")

  -- Suffix: rest of cursor line + lines_after lines
  local suffix_end  = math.min(total_lines, row + cfg.lines_after)
  local suffix_lines = vim.api.nvim_buf_get_lines(buf, row - 1, suffix_end, false)
  -- Trim the first element from cursor column onward
  if #suffix_lines > 0 then
    suffix_lines[1] = suffix_lines[1]:sub(col + 1)
  end
  local suffix = table.concat(suffix_lines, "\n")

  local language = vim.bo[buf].filetype or "text"

  return prefix, suffix, language
end

local function fire()
  local cfg = require("cade.config").get()
  if not cfg.enabled then return end

  local prefix, suffix, language = build_context()

  -- Skip if there is barely any context
  if #prefix < cfg.min_prefix then return end

  cancel_inflight()

  _cancel = http.fetch(
    prefix, suffix, language,
    function(accumulated)       -- on_token: update ghost text incrementally
      ghost.show(accumulated)
    end,
    function()                  -- on_done: nothing extra needed
    end,
    function(--[[msg]])         -- on_error: silent (server may not be running)
    end
  )
end

-- ── Public API ───────────────────────────────────────────────────────────────

function M.on_text_changed()
  local cfg = require("cade.config").get()
  -- If ghost text is showing, clear it immediately on new keystrokes
  if ghost.is_visible() then
    ghost.clear()
    cancel_inflight()
  end

  if _timer then
    _timer:stop()
  else
    _timer = vim.uv.new_timer()
  end

  _timer:start(cfg.debounce_ms, 0, vim.schedule_wrap(fire))
end

function M.on_cursor_moved()
  -- Cursor moved in insert mode (e.g. arrow keys) — clear and stop
  cancel_inflight()
  if _timer then _timer:stop() end
end

function M.on_insert_leave()
  cancel_inflight()
  if _timer then _timer:stop() end
  ghost.clear()
end

return M
```

---

## Phase 3 — Public API & Setup

### 3A. `lua/cade/init.lua`

```lua
local M = {}

function M.setup(opts)
  require("cade.config").setup(opts)
end

-- Completion state passthrough
function M.accept()      return require("cade.ghost").accept()      end
function M.accept_line() return require("cade.ghost").accept_line() end
function M.accept_word() return require("cade.ghost").accept_word() end
function M.dismiss()     return require("cade.ghost").dismiss()     end
function M.is_visible()  return require("cade.ghost").is_visible()  end

function M.toggle()
  local cfg = require("cade.config")
  cfg.current.enabled = not cfg.current.enabled
  if not cfg.current.enabled then
    require("cade.ghost").clear()
  end
  vim.notify("CADE completions " .. (cfg.current.enabled and "enabled" or "disabled"))
end

return M
```

---

## Phase 4 — Plugin Wiring

### 4A. `plugin/cade.lua` additions

Append to the existing file (after the socket setup block):

```lua
-- ── CADE Inline Completions (Option B) ──────────────────────────────────────

local ok, cade = pcall(require, "cade")
if not ok then return end

-- Default setup (user can call require("cade").setup({}) to override)
cade.setup({})

local trigger = require("cade.trigger")

-- Autocmds
local group = vim.api.nvim_create_augroup("CadeCompletions", { clear = true })

vim.api.nvim_create_autocmd("TextChangedI", {
  group    = group,
  callback = trigger.on_text_changed,
})

vim.api.nvim_create_autocmd("CursorMovedI", {
  group    = group,
  callback = trigger.on_cursor_moved,
})

vim.api.nvim_create_autocmd("InsertLeave", {
  group    = group,
  callback = trigger.on_insert_leave,
})

-- Keymaps (insert mode)
local function imap(lhs, fn, desc)
  vim.keymap.set("i", lhs, function()
    if cade.is_visible() then
      fn()
      return ""
    end
    return lhs
  end, { expr = true, noremap = true, desc = desc })
end

imap("<Tab>",  cade.accept,      "CADE: accept full completion")
imap("<C-]>",  cade.accept_line, "CADE: accept one line")
imap("<M-]>",  cade.accept_word, "CADE: accept next word")
imap("<C-e>",  cade.dismiss,     "CADE: dismiss completion")

-- Normal-mode toggle
vim.keymap.set("n", "<leader>ct", cade.toggle, { desc = "CADE: toggle completions" })
```

---

## Configuration Reference

Users call `require("cade").setup({})` in their Neovim config (e.g. `lazy.nvim`):

```lua
{
  "EzekTec-Inc/CADE-nvim",
  config = function()
    require("cade").setup({
      enabled      = true,
      server_port  = 8284,
      agent_id     = "my-agent-id",   -- or set $CADE_AGENT_ID
      api_key      = "",              -- or set $CADE_API_KEY
      lines_before = 50,
      lines_after  = 20,
      debounce_ms  = 300,
      min_prefix   = 3,
      max_tokens   = 512,
      model        = "",              -- empty = agent's default model
      hl_group     = "Comment",       -- ghost-text highlight
      filetypes    = {},              -- empty = all; e.g. {"lua","rust","python"}
    })
  end
}
```

---

## Keymap Reference

| Key | Mode | Action |
|-----|------|--------|
| `<Tab>` | Insert | Accept full completion |
| `<C-]>` | Insert | Accept current line only |
| `<M-]>` | Insert | Accept next word |
| `<C-e>` | Insert | Dismiss ghost text |
| `<leader>ct` | Normal | Toggle completions on/off |

---

## Data Flow (Detailed)

```
TextChangedI fires
  └─ trigger.on_text_changed()
       ├─ ghost.clear()          [if showing]
       ├─ _timer:stop()          [reset debounce]
       └─ _timer:start(300ms)
              └─ fire()  [on vim main thread via schedule_wrap]
                   ├─ build_context() → prefix, suffix, language
                   ├─ cancel_inflight()
                   └─ http.fetch(prefix, suffix, language,
                        on_token  → ghost.show(accumulated)   [incremental]
                        on_done   → noop
                        on_error  → silent
                      )
                        └─ vim.system(["curl", ...])
                             stdout callback (libuv thread)
                               └─ parse SSE line
                                    └─ vim.schedule → on_token(snap)
                                         └─ ghost.show(snap)
                                              ├─ ghost.clear()   [remove old extmarks]
                                              └─ nvim_buf_set_extmark × 2
                                                   virt_text  (line 1, inline)
                                                   virt_lines (lines 2-N, below)

User presses <Tab>
  └─ ghost.accept()
       ├─ text = M._pending
       ├─ ghost.clear()
       └─ nvim_put(lines, "c", true, true)
```

---

## Edge Cases & Mitigations

| Scenario | Mitigation |
|---|---|
| CADE server not running | `curl` exits non-zero → `on_error` is silent → no ghost text, no crash |
| User types faster than debounce | Each `TextChangedI` resets the timer; previous curl handle is killed |
| Slow LLM (> 2-3s) | Ghost text appears incrementally as tokens stream; cursor move clears |
| Empty or whitespace completion | `ghost.show("")` is a no-op; `[DONE]` received → nothing shown |
| Multi-line completion | Line 1 → `virt_text` inline; lines 2-N → `virt_lines` below cursor |
| `<Tab>` already mapped (nvim-cmp) | `expr = true` keymap only intercepts when `is_visible()` is true |
| Completion menu open (nvim-cmp) | Autocmd on `User BlinkCmpMenuOpen` / `cmp.event "menu_opened"` sets `vim.b.cade_hidden = true` to pause trigger |
| Buffer without a filetype | Falls back to `"text"` |
| `agent_id` not configured | `http.fetch` calls `on_error` immediately; silent, no curl invoked |
| Rapid `accept_word` calls | Each call re-renders remaining ghost text via `ghost.show()` |

---

## Testing Strategy

### Manual smoke tests (no test framework required)

1. Start `cade-server`, create/select an agent, set `$CADE_AGENT_ID`.
2. Open a Lua/Rust/Python file in Neovim. Type a function signature — ghost text should appear after 300 ms.
3. Press `<Tab>` — full completion inserted, ghost text cleared.
4. Type again, press `<C-]>` — only first line inserted.
5. Type again, press `<M-]>` — one word inserted, remainder re-shown.
6. Type again, press `<C-e>` — ghost text dismissed with no insertion.
7. Move cursor with arrow keys mid-suggestion — ghost text clears immediately.
8. Kill `cade-server` — typing continues with no error popups.

### Unit-testable pure functions

| Function | Input | Expected output |
|---|---|---|
| `ghost.accept_word()` with `"  foo bar"` | pending = `"  foo bar"` | inserts `"  foo"`, pending = `" bar"` |
| `ghost.accept_line()` with `"a\nb\nc"` | pending = `"a\nb\nc"` | inserts `"a"`, pending = `"b\nc"` |
| `build_context()` at row 5, col 3 | buffer with 20 lines | prefix ends at col 3, suffix starts at col 3 |

---

## Implementation Phases Summary

| Phase | Deliverable | Files |
|---|---|---|
| **P1** | Ghost renderer + HTTP client | `lua/cade/config.lua`, `lua/cade/ghost.lua`, `lua/cade/http.lua` |
| **P2** | Debounced trigger | `lua/cade/trigger.lua` |
| **P3** | Public API module | `lua/cade/init.lua` |
| **P4** | Plugin wiring | `plugin/cade.lua` (appended) |

Each phase is independently reviewable and testable before the next begins.

---

## What Does NOT Change

- `mcp-server/server.py` — untouched; MCP intercept tools remain unchanged
- `plugin/cade.lua` socket setup — untouched; append-only additions
- `crates/cade-server/src/server/api/complete.rs` — already complete, no changes
- VS Code extension — unaffected

---

## Key Design Decisions

| Decision | Rationale |
|---|---|
| `vim.system` + `curl` over `vim.uv` TCP | `curl` handles HTTP/1.1 keep-alive, redirects, and TLS automatically; raw `vim.uv.new_tcp()` would require manual HTTP framing |
| `vim.net` not used | Not available until Neovim nightly; Neovim 0.11.5 is the target |
| `inline` virt_text_pos (not `overlay`) | `inline` inserts visual space without overwriting real characters; `overlay` can obscure existing text |
| Incremental ghost-text updates | Tokens stream in from SSE — showing each partial completion improves perceived latency |
| Silent `on_error` | Server may be stopped between sessions; noisy errors would degrade the editing experience |
| Debounce 300 ms default | Balances responsiveness with LLM API cost; copilot.lua uses 75 ms but that targets a local LSP |
| `expr = true` keymaps | `<Tab>` passthrough when no completion is visible — avoids hijacking indentation |
