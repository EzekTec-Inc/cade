-- CADE-nvim Initialization
-- This file is automatically loaded by Neovim when the plugin is installed.

if vim.g.loaded_cade_nvim == 1 then
  return
end
vim.g.loaded_cade_nvim = 1

local socket_path = "/tmp/nvim.pipe"

-- Automatically set the environment variable globally for Neovim and all child processes (only if not headless)
if #vim.api.nvim_list_uis() > 0 then
  if vim.env.NVIM_LISTEN_ADDRESS == nil or vim.env.NVIM_LISTEN_ADDRESS == "" then
    vim.env.NVIM_LISTEN_ADDRESS = socket_path
  end
end

-- Safely start the internal server on the socket (only if not headless)
if #vim.api.nvim_list_uis() > 0 then
  pcall(function()
    vim.fn.serverstart(socket_path)
  end)
end

-- ── CADE Inline Completions (Option B) ──────────────────────────────────────

local ok, cade = pcall(require, "cade")
if not ok then return end

-- Default setup (user can call require("cade").setup({}) in their config to override)
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

-- Keymaps — driven by config; set keymaps=false in setup() to disable all
local cfg = require("cade.config").get()
if cfg.keymaps ~= false then
  local km = cfg.keymaps

  -- Insert-mode bindings: fall through when no ghost text is visible
  local insert_bindings = {
    { km.accept,      cade.accept,      "CADE: accept full completion" },
    { km.accept_line, cade.accept_line, "CADE: accept one line"        },
    { km.accept_word, cade.accept_word, "CADE: accept next word"       },
    { km.dismiss,     cade.dismiss,     "CADE: dismiss completion"     },
  }
  for _, b in ipairs(insert_bindings) do
    local lhs, fn, desc = b[1], b[2], b[3]
    if lhs then
      vim.keymap.set("i", lhs, function()
        if cade.is_visible() then fn(); return "" end
        return lhs
      end, { expr = true, noremap = true, desc = desc })
    end
  end

  -- Normal-mode toggle
  if km.toggle then
    vim.keymap.set("n", km.toggle, cade.toggle, { desc = "CADE: toggle completions" })
  end

  -- Visual-mode edit
  if km.edit then
    vim.keymap.set("v", km.edit, cade.hover_edit, { desc = "CADE: hover edit" })
  end
end

-- User commands
vim.api.nvim_create_user_command("CadeStatus", function()
  cade.status()
end, { desc = "CADE: show completion status and server reachability" })
