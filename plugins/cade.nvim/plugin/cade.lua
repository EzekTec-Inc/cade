-- CADE-nvim Initialization
-- This file is automatically loaded by Neovim when the plugin is installed.

if vim.g.loaded_cade_nvim == 1 then
  return
end
vim.g.loaded_cade_nvim = 1

local socket_path = "/tmp/nvim.pipe"

-- Automatically set the environment variable globally for Neovim and all child processes
if vim.env.NVIM_LISTEN_ADDRESS == nil or vim.env.NVIM_LISTEN_ADDRESS == "" then
  vim.env.NVIM_LISTEN_ADDRESS = socket_path
end

-- Safely start the internal server on the socket
pcall(function()
  vim.fn.serverstart(socket_path)
end)

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
