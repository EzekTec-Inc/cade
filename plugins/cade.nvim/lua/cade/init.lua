-- cade/init.lua — Public API for CADE inline completions

local M = {}

--- Setup CADE completions with user options.
---@param opts table|nil  See cade.config for available options.
function M.setup(opts)
  require("cade.config").setup(opts)
end

-- ── Completion state passthrough ─────────────────────────────────────────────

function M.accept()
  return require("cade.ghost").accept()
end

function M.accept_line()
  return require("cade.ghost").accept_line()
end

function M.accept_word()
  return require("cade.ghost").accept_word()
end

function M.dismiss()
  return require("cade.ghost").dismiss()
end

function M.is_visible()
  return require("cade.ghost").is_visible()
end

--- Toggle completions on/off.
function M.toggle()
  local cfg = require("cade.config")
  cfg.current.enabled = not cfg.current.enabled
  if not cfg.current.enabled then
    require("cade.ghost").clear()
  end
  vim.notify("CADE completions " .. (cfg.current.enabled and "enabled" or "disabled"))
end

return M
