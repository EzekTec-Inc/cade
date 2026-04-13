# Theming in CADE

CADE supports custom themes and dynamic colorscheme switching via the `/theme <name>` command.

## Applying a Theme

You can apply a theme in CADE dynamically:
```
/theme dark
/theme light
/theme nvim-exported
```

To set the default theme persistently, edit your `~/.cade/settings.json`:
```json
{
  "theme": "dark"
}
```

## Creating Custom Themes

Themes are either natively supported TextMate (`.tmTheme`) files or custom JSON files located in `~/.cade/themes/`. 

### The Modern TextMate (.tmTheme) Approach

CADE natively parses standard `.tmTheme` (TextMate / Sublime Text) files, the exact format powering VS Code and Neovim color schemes. This is the **recommended** way to theme CADE because it requires zero manual color mapping and applies exactly the same syntax-highlighting to markdown blocks as your editor.

To use an existing theme (e.g. Tokyonight, Catppuccin, Gruvbox):
1. Download its `.tmTheme` file (usually found in the `extras` or `bat` directory of your favorite Neovim/VSCode theme repository).
2. Place it in `~/.cade/themes/mytheme.tmTheme`.
3. Switch to it dynamically using `/theme mytheme`.

CADE automatically extracts UI colors (borders, backgrounds, prompts) by dynamically parsing semantic scopes like `keyword.control`, `string`, `invalid`, and the global `background`/`foreground` properties.

---

### Legacy JSON Theme Schema

If you prefer building a theme manually from scratch without a `.tmTheme` base, a typical theme file like `~/.cade/themes/my-theme.json` looks like this:

```json
{
  "name": "my-theme",
  "author": "Your Name",
  "colors": {
    "accent": "#ff8800",
    "border": "#444444",
    "borderAccent": "#ffaa00",
    "borderMuted": "#333333",
    "success": "#00ff00",
    "error": "#ff0000",
    "warning": "#ffff00",
    "muted": "#888888",
    "dim": "#555555",
    "text": "#ffffff",
    "thinkingText": "#aaaaaa",
    "selectedBg": "#333355",
    "userMessageBg": "#222222",
    "userMessageText": "#ffffff",
    "customMessageBg": "#111111",
    "customMessageText": "#eeeeee",
    "toolPendingBg": "#444422",
    "toolSuccessBg": "#224422",
    "toolErrorBg": "#442222",
    "toolTitle": "#ffccaa",
    "toolOutput": "#dddddd"
  }
}
```

## Neovim Integration

CADE natively parses `.tmTheme` files, so external Lua plugins are no longer necessary for syncing themes. You can download the `.tmTheme` artifact of any Neovim colorscheme (TokyoNight, Catppuccin, RosePine) and put it into `~/.cade/themes/` directly.

However, if you still want to dynamically force CADE to synchronize automatically whenever you run `:colorscheme` inside a live Neovim instance, you can use the **Legacy Export Approach**.

### Standalone Plugin (Legacy)

The `cade.nvim` plugin is a minimal Lua script that extracts colors directly from Neovim highlight groups and exports a `.json` theme format to CADE. Since it is hosted in the main CADE monorepo, point your package manager to the `plugins/cade.nvim` directory.

Using **lazy.nvim**:
```lua
{
  "EzekTec-Inc/cade",
  config = function(plugin)
    vim.opt.rtp:append(plugin.dir .. "/plugins/cade.nvim")
    require("cade").setup({
      auto_export = true,
      theme_name = "nvim-exported"
    })
  end
}
```
After installing the plugin, run `:CadeExportTheme` in Neovim, and switch to the theme in CADE with `/theme nvim-exported`.

### Copy-Paste Snippet

If you prefer a plugin-free experience, you can copy the following Lua function directly into your `init.lua` to enable immediate auto-export on `:colorscheme` change:

```lua
local function cade_export_theme()
    local function int_to_hex(color)
        return color and string.format("#%06x", color) or nil
    end
    
    local function get_hl_prop(name, prop)
        local hl = vim.api.nvim_get_hl(0, { name = name, link = false })
        if hl and hl.link then hl = vim.api.nvim_get_hl(0, { name = hl.link, link = false }) end
        return hl and hl[prop] and int_to_hex(hl[prop]) or nil
    end

    local function c_fg(...)
        for _, group in ipairs({...}) do
            local color = get_hl_prop(group, "fg")
            if color then return color end
        end
    end
    
    local function c_bg(...)
        for _, group in ipairs({...}) do
            local color = get_hl_prop(group, "bg")
            if color then return color end
        end
    end

    local text_color = c_fg("Normal") or "#FFFFFF"
    local normal_bg = c_bg("Normal") or ""
    local accent = c_fg("Statement", "Function", "Keyword") or "#000000"
    local dim = c_fg("NonText", "Conceal") or "#888888"

    local colors = {
        accent = accent,
        border = c_fg("FloatBorder", "WinSeparator", "LineNr"),
        borderAccent = c_fg("TelescopeBorder") or accent,
        borderMuted = c_fg("Comment", "NonText"),
        success = c_fg("DiagnosticOk", "String"),
        error = c_fg("DiagnosticError", "ErrorMsg"),
        warning = c_fg("DiagnosticWarn", "WarningMsg"),
        muted = c_fg("Comment", "LineNr"),
        dim = dim,
        text = text_color,
        thinkingText = c_fg("Comment") or dim,
        selectedBg = c_bg("Visual", "CursorLine"),
        userMessageBg = c_bg("NormalFloat", "CursorLine"),
        userMessageText = c_fg("NormalFloat", "Normal") or text_color,
        customMessageBg = c_bg("NormalFloat", "Normal") or normal_bg,
        customMessageText = c_fg("NormalFloat", "Normal") or text_color,
        toolPendingBg = c_bg("CursorLine", "ColorColumn"),
        toolSuccessBg = c_bg("DiffAdd", "Normal") or normal_bg,
        toolErrorBg = c_bg("DiffDelete", "ErrorMsg"),
        toolTitle = c_fg("Title", "Function"),
        toolOutput = text_color,
    }

    for k, v in pairs(colors) do if not v then colors[k] = "" end end

    local theme = { name = "nvim-exported", author = "cade.nvim", colors = colors }
    local theme_dir = os.getenv("HOME") .. "/.cade/themes"
    vim.fn.mkdir(theme_dir, "p")
    local file = io.open(theme_dir .. "/nvim-exported.json", "w")
    if file then
        file:write(vim.fn.json_encode(theme))
        file:close()
    end
end

-- Export on colorscheme change
vim.api.nvim_create_autocmd("ColorScheme", {
    pattern = "*",
    callback = cade_export_theme,
})
-- Run once on startup
cade_export_theme()
```
