-- cade/config.lua — Defaults and user configuration merge
-- No side effects on require.

local M = {}

M.defaults = {
  enabled       = true,
  server_port   = 8284,
  agent_id      = vim.env.CADE_AGENT_ID or "",
  api_key       = vim.env.CADE_API_KEY  or "",
  lines_before  = 50,       -- prefix context lines
  lines_after   = 20,       -- suffix context lines
  debounce_ms   = 300,      -- ms to wait after last keystroke
  min_prefix    = 3,        -- skip if prefix shorter than this
  max_tokens    = 512,      -- forwarded to server
  model         = "",       -- optional model override (empty = agent default)
  filetypes     = {},       -- allowlist; empty = all filetypes
  hl_group      = "Comment", -- ghost-text highlight group
}

M.current = vim.deepcopy(M.defaults)

--- Merge user options into defaults.
---@param opts table|nil
function M.setup(opts)
  M.current = vim.tbl_deep_extend("force", vim.deepcopy(M.defaults), opts or {})
end

--- Return the active config table.
---@return table
function M.get()
  return M.current
end

return M
