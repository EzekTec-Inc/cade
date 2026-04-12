-- cade/trigger.lua — Debounced TextChangedI handler
-- Manages timer, cancellation of in-flight requests, and context building.

local M = {}

local ghost = require("cade.ghost")
local http  = require("cade.http")

local _timer  = nil -- vim.uv timer handle (reused)
local _cancel = nil -- cancel() from the last http.fetch call

-- ── Internal helpers ─────────────────────────────────────────────────────────

local function cancel_inflight()
  if _cancel then
    _cancel()
    _cancel = nil
  end
end

--- Build prefix/suffix context from the current buffer and cursor position.
---@return string prefix, string suffix, string language
local function build_context()
  local cfg = require("cade.config").get()
  local buf = vim.api.nvim_get_current_buf()
  local cursor = vim.api.nvim_win_get_cursor(0) -- {row, col} 1-based row
  local row = cursor[1]                          -- 1-based
  local col = cursor[2]                          -- 0-based byte offset

  local total_lines = vim.api.nvim_buf_line_count(buf)

  -- Prefix: lines_before lines up to and including cursor column
  local prefix_start = math.max(1, row - cfg.lines_before)
  local prefix_lines = vim.api.nvim_buf_get_lines(buf, prefix_start - 1, row, false)
  if #prefix_lines > 0 then
    prefix_lines[#prefix_lines] = prefix_lines[#prefix_lines]:sub(1, col)
  end
  local prefix = table.concat(prefix_lines, "\n")

  -- Suffix: rest of cursor line + lines_after lines
  local suffix_end = math.min(total_lines, row + cfg.lines_after)
  local suffix_lines = vim.api.nvim_buf_get_lines(buf, row - 1, suffix_end, false)
  if #suffix_lines > 0 then
    suffix_lines[1] = suffix_lines[1]:sub(col + 1)
  end
  local suffix = table.concat(suffix_lines, "\n")

  local language = vim.bo[buf].filetype or "text"

  return prefix, suffix, language
end

--- Fires after the debounce timer elapses. Builds context and starts a fetch.
local function fire()
  local cfg = require("cade.config").get()
  if not cfg.enabled then return end

  -- Filetype filtering
  if #cfg.filetypes > 0 then
    local ft = vim.bo.filetype or ""
    local allowed = false
    for _, v in ipairs(cfg.filetypes) do
      if v == ft then allowed = true; break end
    end
    if not allowed then return end
  end

  local prefix, suffix, language = build_context()

  -- Skip if there is barely any context
  if #prefix < cfg.min_prefix then return end

  cancel_inflight()

  _cancel = http.fetch(
    prefix, suffix, language,
    function(accumulated) -- on_token: update ghost text incrementally
      ghost.show(accumulated)
    end,
    function() -- on_done: nothing extra needed
    end,
    function(--[[msg]]) -- on_error: silent (server may not be running)
    end
  )
end

-- ── Public API (called from autocmds in plugin/cade.lua) ─────────────────────

--- Called on TextChangedI — resets the debounce timer.
function M.on_text_changed()
  -- Clear existing ghost text immediately on new keystrokes
  if ghost.is_visible() then
    ghost.clear()
  end
  cancel_inflight()

  if _timer then
    _timer:stop()
  else
    _timer = vim.uv.new_timer()
  end

  local cfg = require("cade.config").get()
  _timer:start(cfg.debounce_ms, 0, vim.schedule_wrap(fire))
end

--- Called on CursorMovedI — cancel and clear (cursor moved without typing).
function M.on_cursor_moved()
  cancel_inflight()
  if _timer then _timer:stop() end
  if ghost.is_visible() then ghost.clear() end
end

--- Called on InsertLeave — full cleanup.
function M.on_insert_leave()
  cancel_inflight()
  if _timer then _timer:stop() end
  ghost.clear()
end

return M
