-- cade/config.lua — Defaults and user configuration merge
-- No side effects on require.

local M = {}

--- Default path to the CADE settings file.
local DEFAULT_SETTINGS_PATH = vim.fn.expand("~/.cade/settings.json")

--- Resolve agent_id: $CADE_AGENT_ID → settings.json last_agent → "".
---@param settings_path string|nil  Override path for testing
---@return string
local function resolve_agent_id(settings_path)
  -- 1. Environment variable wins
  local env = vim.env.CADE_AGENT_ID
  if env and env ~= "" then
    return env
  end

  -- 2. Fall back to ~/.cade/settings.json → last_agent
  local path = settings_path or DEFAULT_SETTINGS_PATH
  local ok, lines = pcall(vim.fn.readfile, path)
  if ok and lines and #lines > 0 then
    local json_ok, data = pcall(vim.fn.json_decode, table.concat(lines, "\n"))
    if json_ok and type(data) == "table" and type(data.last_agent) == "string" and data.last_agent ~= "" then
      return data.last_agent
    end
  end

  -- 3. Nothing found
  return ""
end

M.defaults = {
  enabled       = true,
  server_port   = 8284,
  agent_id      = resolve_agent_id(),
  api_key       = vim.env.CADE_API_KEY  or "",
  lines_before  = 50,       -- prefix context lines
  lines_after   = 20,       -- suffix context lines
  debounce_ms   = 300,      -- ms to wait after last keystroke
  min_prefix    = 3,        -- skip if prefix shorter than this
  max_tokens    = 512,      -- forwarded to server
  model         = "",       -- optional model override (empty = agent default)
  filetypes     = {},       -- allowlist; empty = all filetypes
  hl_group      = "Comment", -- ghost-text highlight group
  keymaps       = {         -- set to false to disable all bindings
    accept      = "<Tab>",
    accept_line = "<C-]>",
    accept_word = "<M-]>",
    dismiss     = "<C-e>",
    toggle      = "<leader>ct",
    edit        = "<leader>ce",
  },
}

M.current = vim.deepcopy(M.defaults)

--- Merge user options into defaults.
--- Pass opts._settings_path to override the settings.json location (testing only).
---@param opts table|nil
function M.setup(opts)
  opts = opts or {}

  -- Re-resolve agent_id if caller hasn't explicitly set one
  local settings_path = opts._settings_path
  if settings_path then
    opts._settings_path = nil -- strip internal key before merge
    if not opts.agent_id then
      opts.agent_id = resolve_agent_id(settings_path)
    end
  end

  M.current = vim.tbl_deep_extend("force", vim.deepcopy(M.defaults), opts)
end

--- Return the active config table.
---@return table
function M.get()
  return M.current
end

return M
