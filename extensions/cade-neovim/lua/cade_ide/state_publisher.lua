-- lua/cade_ide/state_publisher.lua
-- Subscribes to Neovim autocmd events and pushes debounced StateUpdate
-- frames to CadeConnection.
--
-- Pure Neovim API — no external deps beyond connection.lua / protocol.lua.

local proto = require("cade_ide.protocol")

local DEBOUNCE_MS = 50

local M = {}

--- Create and return a StatePublisher.
--- @param conn table   CadeConnection instance
--- @param opts table|nil  { debounce_ms = number }
function M.new(conn, opts)
  opts = opts or {}
  local debounce_ms = opts.debounce_ms or DEBOUNCE_MS

  local self = {
    _conn       = conn,
    _timer      = nil,
    _augroup    = nil,
    _disposed   = false,
  }

  -- ── public API ─────────────────────────────────────────────────────────────

  --- Register autocmds and fire the first publish.
  function self.start()
    if self._disposed then return end
    local ag = vim.api.nvim_create_augroup("CadeIdeStatePublisher", { clear = true })
    self._augroup = ag

    local events = {
      "BufEnter", "BufLeave", "BufWritePost",
      "TextChanged", "TextChangedI",
      "CursorMoved", "CursorMovedI",
      "DiagnosticChanged",
      "ModeChanged",
      "VimResized",
    }
    vim.api.nvim_create_autocmd(events, {
      group    = ag,
      callback = function() self._schedule() end,
    })

    self._schedule()   -- initial snapshot
  end

  --- Stop publishing and remove autocmds.
  function self.dispose()
    self._disposed = true
    self:_cancel_timer()
    if self._augroup then
      pcall(vim.api.nvim_del_augroup_by_id, self._augroup)
      self._augroup = nil
    end
  end

  -- ── private ─────────────────────────────────────────────────────────────────

  function self._schedule()
    if self._disposed then return end
    self:_cancel_timer()
    local t = vim.loop.new_timer()
    self._timer = t
    t:start(debounce_ms, 0, function()
      t:close(); self._timer = nil
      vim.schedule(function()
        if not self._disposed then
          self._conn.send_state_update(self:_snapshot())
        end
      end)
    end)
  end

  function self:_cancel_timer()
    if self._timer then
      pcall(function() self._timer:stop(); self._timer:close() end)
      self._timer = nil
    end
  end

  --- Build a StateSnapshot from the current Neovim state.
  function self:_snapshot()
    local open_files = self:_open_files()
    local active_file = vim.api.nvim_buf_get_name(0)
    if active_file == "" then active_file = nil end

    return {
      open_files       = open_files,
      active_file      = active_file,
      selection        = self:_selection(),
      diagnostics      = self:_diagnostics(),
      workspace_folders = self:_workspace_folders(),
      visible_range    = self:_visible_range(),
    }
  end

  function self:_open_files()
    local result = {}
    for _, buf in ipairs(vim.api.nvim_list_bufs()) do
      if not vim.api.nvim_buf_is_loaded(buf) then goto continue end
      local name = vim.api.nvim_buf_get_name(buf)
      if name == "" then goto continue end
      local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
      local text  = table.concat(lines, "\n")
      local ft    = vim.api.nvim_get_option_value("filetype", { buf = buf })
      local mod   = vim.api.nvim_get_option_value("modified", { buf = buf })
      table.insert(result, {
        path        = name,
        text        = text,
        language_id = ft ~= "" and ft or "plaintext",
        version     = vim.api.nvim_buf_get_changedtick(buf),
        is_dirty    = mod,
      })
      ::continue::
    end
    return result
  end

  function self:_selection()
    local mode = vim.fn.mode()
    local in_visual = mode == "v" or mode == "V" or mode == "\22"
    local buf  = vim.api.nvim_get_current_buf()
    local name = vim.api.nvim_buf_get_name(buf)
    if name == "" then return vim.NIL end

    -- getpos returns {bufnum, line (1-based), col (1-based), offset}.
    -- When currently in visual mode, '< and '> hold the PREVIOUS selection
    -- (or zeros), so use 'v' (visual anchor) and '.' (cursor) instead.
    -- When not in visual mode, fall back to '<'/'>' for the last selection —
    -- but only if they're non-zero.
    local s_mark, e_mark
    if in_visual then
      s_mark, e_mark = "v", "."
    else
      s_mark, e_mark = "'<", "'>"
    end
    local s = vim.fn.getpos(s_mark)
    local e = vim.fn.getpos(e_mark)
    if s[2] == 0 or e[2] == 0 then return vim.NIL end

    -- Normalize so start <= end regardless of selection direction.
    if s[2] > e[2] or (s[2] == e[2] and s[3] > e[3]) then
      s, e = e, s
    end

    local sr, sc = s[2] - 1, s[3] - 1
    local er, ec = e[2] - 1, e[3]         -- end_col exclusive
    if mode == "V" then
      -- Line-wise: extend to full lines.
      sc = 0
      ec = #(vim.api.nvim_buf_get_lines(buf, er, er + 1, false)[1] or "")
    end
    local lines = vim.api.nvim_buf_get_lines(buf, sr, er + 1, false)
    local text  = table.concat(lines, "\n")

    return {
      path  = name,
      range = proto.range(proto.position(sr, sc), proto.position(er, ec)),
      text  = text,
    }
  end

  function self:_diagnostics()
    local result = {}
    local sev_map = { [1]="error", [2]="warning", [3]="info", [4]="hint" }
    for _, buf in ipairs(vim.api.nvim_list_bufs()) do
      if not vim.api.nvim_buf_is_loaded(buf) then goto continue end
      local name = vim.api.nvim_buf_get_name(buf)
      if name == "" then goto continue end
      for _, d in ipairs(vim.diagnostic.get(buf)) do
        table.insert(result, {
          path     = name,
          range    = proto.range(
            proto.position(d.lnum,     d.col),
            proto.position(d.end_lnum or d.lnum, d.end_col or d.col)
          ),
          severity = sev_map[d.severity] or "hint",
          message  = d.message,
          source   = d.source or vim.NIL,
          code     = d.code and tostring(d.code) or vim.NIL,
        })
      end
      ::continue::
    end
    return result
  end

  function self:_workspace_folders()
    -- Use the LSP workspace root if available, else fall back to cwd.
    local roots = {}
    local seen  = {}
    for _, client in ipairs(vim.lsp.get_clients()) do
      local root = client.root_dir
      if root and not seen[root] then
        seen[root] = true
        local name = vim.fn.fnamemodify(root, ":t")
        table.insert(roots, { path = root, name = name })
      end
    end
    if #roots == 0 then
      local cwd = vim.fn.getcwd()
      table.insert(roots, { path = cwd, name = vim.fn.fnamemodify(cwd, ":t") })
    end
    return roots
  end

  function self:_visible_range()
    local first = vim.fn.line("w0")
    local last  = vim.fn.line("w$")
    if first == 0 then return vim.NIL end
    return { first - 1, last - 1 }   -- convert to 0-based
  end

  return self
end

return M
