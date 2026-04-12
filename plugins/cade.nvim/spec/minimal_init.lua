-- spec/minimal_init.lua — Minimal Neovim init for plenary test runner
-- Adds the plugin's own lua/ directory to the package path so
-- require("cade.*") works without a full Neovim config.
-- Does NOT load plugin/ to avoid the serverstart() conflict.

-- Prevent the plugin/cade.lua serverstart from conflicting
vim.env.NVIM_LISTEN_ADDRESS = ""
vim.g.loaded_cade_nvim = 1 -- skip plugin/cade.lua entirely

local plugin_root = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":h:h")

-- Add only lua/ to rtp, not the whole plugin (avoids plugin/ auto-load)
vim.opt.rtp:prepend(plugin_root)

-- Add plenary to rtp if available via lazy
local plenary_path = vim.fn.stdpath("data") .. "/lazy/plenary.nvim"
if vim.fn.isdirectory(plenary_path) == 1 then
  vim.opt.rtp:prepend(plenary_path)
end
