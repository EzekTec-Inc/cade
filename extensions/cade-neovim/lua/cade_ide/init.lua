-- lua/cade_ide/init.lua
-- Entry point for the cade-ide Neovim plugin.
--
-- Usage:
--   require("cade_ide").setup()          -- call automatically from plugin/cade_ide.lua
--   require("cade_ide").setup({ ... })   -- with options
--   require("cade_ide").reconnect()      -- manual reconnect

local connection     = require("cade_ide.connection")
local state_pub      = require("cade_ide.state_publisher")
local callback_handler = require("cade_ide.callback_handler")

local M = {}

--- @type table|nil  Active CadeConnection instance.
local _conn = nil
--- @type table|nil  Active StatePublisher instance.
local _pub  = nil

--- Setup the plugin.
--- @param opts table|nil
---   discovery_dir  string   Override the ~/.cade/ide discovery dir (for testing).
---   debounce_ms    number   State-update debounce delay (default 50).
---   log            function Custom log function (msg: string).
function M.setup(opts)
  opts = opts or {}

  -- Dispose any previous session.
  M._teardown()

  local conn = connection.new({
    log           = opts.log,
    discovery_dir = opts.discovery_dir,
  })

  local pub = state_pub.new(conn, {
    debounce_ms = opts.debounce_ms,
  })

  -- Dispatch incoming CallbackRequests.
  conn.on_callback(function(id, op)
    local result = callback_handler.handle(op)
    conn.send_response(id, result)
  end)

  pub.start()
  conn.connect()

  _conn = conn
  _pub  = pub

  -- Expose a :CadeReconnect user command.
  vim.api.nvim_create_user_command("CadeReconnect", function()
    M.reconnect()
  end, { desc = "Reconnect CADE IDE bridge to cade-ide-mcp" })
end

--- Manually reconnect (useful after restarting cade-ide-mcp).
function M.reconnect()
  if _conn then _conn.connect() end
end

--- Teardown — called on VimLeave or before re-setup.
function M._teardown()
  if _pub  then pcall(_pub.dispose);  _pub  = nil end
  if _conn then pcall(_conn.dispose); _conn = nil end
end

-- Auto-teardown on exit.
vim.api.nvim_create_autocmd("VimLeavePre", {
  group    = vim.api.nvim_create_augroup("CadeIdeCleanup", { clear = true }),
  callback = M._teardown,
})

return M
