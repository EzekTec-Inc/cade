# cade.nvim

A lightweight Neovim plugin that exports your active colorscheme to [CADE](https://github.com/ezektec/cade) as a dynamic JSON theme.

## Features

- Extracts true RGB hex colors from active highlight groups.
- Maps standard Neovim highlight groups to CADE's semantic theme tokens.
- Supports `auto_export` to sync colors instantly when you change your Neovim colorscheme.
- Automatically handles light/dark transitions and fallback highlight groups.

## Installation

Using [lazy.nvim](https://github.com/folke/lazy.nvim):

```lua
{
  "EzekTec-Inc/cade.nvim",
  config = function()
    require("cade").setup({
      auto_export = true, -- Automatically export theme on ColorScheme change
      theme_name = "nvim-exported" -- The name of the exported CADE theme
    })
  end
}
```

Using [packer.nvim](https://github.com/wbthomason/packer.nvim):

```lua
use {
  "EzekTec-Inc/cade.nvim",
  config = function()
    require("cade").setup({ auto_export = true })
  end
}
```

## Usage

Once installed, the plugin can automatically export your active colorscheme to `~/.cade/themes/nvim-exported.json` whenever you run `:colorscheme`.

You can also manually trigger an export with the user command:
```vim
:CadeExportTheme
```

### In CADE
To use the exported theme in CADE, open the CADE TUI and run:
```
/theme nvim-exported
```
To make it persistent across sessions, update your `~/.cade/settings.json`:
```json
{
  "theme": "nvim-exported"
}
```

## How It Works

`cade.nvim` reads properties from `vim.api.nvim_get_hl` to map highlight groups like `Statement`, `FloatBorder`, and `DiagnosticOk` to CADE's `accent`, `border`, and `success` tokens respectively. 
