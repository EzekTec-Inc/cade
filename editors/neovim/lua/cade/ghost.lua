-- cade/ghost.lua — Extmark ghost-text renderer
-- Pure renderer: no I/O, no timers. Called by http.lua callbacks and keymaps.

local M = {}
local api = vim.api
local ns  = api.nvim_create_namespace("cade_ghost")

-- ── State ────────────────────────────────────────────────────────────────────

M._pending  = nil   -- full accumulated completion string, or nil
M._buf      = nil   -- buffer handle where ghost text lives
M._mark_ids = {}    -- list of extmark IDs created by show()

-- ── Render ───────────────────────────────────────────────────────────────────

--- Show ghost text at the current cursor position.
--- Called incrementally as SSE tokens arrive (replaces previous ghost text).
---@param text string  Full accumulated completion text so far
function M.show(text)
  if not text or text == "" then return end

  local buf    = api.nvim_get_current_buf()
  local cursor = api.nvim_win_get_cursor(0) -- {row, col}, 1-based row
  local row    = cursor[1] - 1              -- 0-based for extmark API
  local col    = cursor[2]                  -- 0-based byte offset

  M.clear() -- remove previous extmarks before re-drawing

  M._pending  = text
  M._buf      = buf
  M._mark_ids = {}

  local hl    = M._hl()
  local lines = vim.split(text, "\n", { plain = true })

  -- First line: inline virtual text at cursor column
  if #lines >= 1 then
    local id = api.nvim_buf_set_extmark(buf, ns, row, col, {
      virt_text     = { { lines[1], hl } },
      virt_text_pos = "inline",
      undo_restore  = false,
      invalidate    = true,
    })
    table.insert(M._mark_ids, id)
  end

  -- Remaining lines: virtual lines below the cursor row
  if #lines > 1 then
    local virt_lines = {}
    for i = 2, #lines do
      table.insert(virt_lines, { { lines[i], hl } })
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

-- ── Acceptance ───────────────────────────────────────────────────────────────

--- Accept the full pending completion.
---@return boolean  true if a completion was accepted
function M.accept()
  if not M._pending then return false end
  local text = M._pending
  M.clear()
  local lines = vim.split(text, "\n", { plain = true })
  api.nvim_put(lines, "c", true, true)
  return true
end

--- Accept only the first line of the pending completion.
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

--- Accept the next word of the pending completion.
---@return boolean
function M.accept_word()
  if not M._pending then return false end
  -- Match optional leading whitespace + non-whitespace
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
---@return boolean
function M.is_visible()
  return M._pending ~= nil
end

-- ── Internal ─────────────────────────────────────────────────────────────────

function M._hl()
  return require("cade.config").get().hl_group
end

return M
