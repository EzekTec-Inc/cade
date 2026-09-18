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

CADE uses a native, declarative **TOML specification** powered by an authoritative `ThemeResolver` and Opaline engine. Themes can be placed in:
1. **Project-local**: `.cade/themes/<name>.toml` (highest precedence, shared with team members in a git repository)
2. **User-global**: `~/.cade/themes/<name>.toml` (user-specific custom themes)
3. **Built-in themes**: Built directly into CADE binary (`dark`, `light`, `tokyonight`, `catppuccin`, `gruvbox`, etc.)

### Quick Start: Scaffold a Starter Theme

You can generate a starter template directly inside your project:
```
/theme init my-brand
```
This generates a fully-specified `.cade/themes/my-brand.toml` containing all core roles, palette declarations, syntax tokens, and fallback mappings.

### Validating and Inspecting Themes

CADE provides built-in validation to ensure custom themes are fully functional, have sufficient WCAG contrast, and correctly map all recommended roles:
```
/theme validate my-brand
/theme validate path/to/theme.toml
```

To see exactly how tokens resolve (exact match vs. derived from fallback):
```
/theme inspect my-brand
```

---

### Canonical TOML Theme Schema

A theme file is structured into three clean sections:
- `[meta]`: Metadata about the theme (name, variant, description, author).
- `[palette]`: Raw hex color definitions (`#rrggbb`).
- `[tokens]`: Semantic role mappings binding UI and syntax tokens to palette colors.

Example:
```toml
[meta]
name = "my-brand"
variant = "dark"
description = "High-contrast branded dark theme"
author = "Your Name"

[palette]
bg_dark         = "#1e1e2e"
bg_panel        = "#252538"
bg_elevated     = "#2f2f45"
fg_primary      = "#cdd6f4"
fg_muted        = "#a6adc8"
accent_primary  = "#89b4fa"
status_success  = "#a6e3a1"
status_warning  = "#f9e2af"
status_error    = "#f38ba8"
border_base     = "#313244"
border_active   = "#89b4fa"

[tokens]
"bg.base"          = "bg_dark"
"bg.panel"         = "bg_panel"
"bg.elevated"      = "bg_elevated"
"text.primary"     = "fg_primary"
"text.muted"       = "fg_muted"
"accent.primary"   = "accent_primary"
"success"          = "status_success"
"warning"          = "status_warning"
"error"            = "status_error"
"border.unfocused" = "border_base"
"border.focused"   = "border_active"
```

A complete, production-grade reference theme is available in `.cade/themes/reference.toml`.

---

### Fallback Semantics & Intentional Gray Preservation

CADE does not require you to specify every single token. When an optional token is absent, `ThemeResolver` will automatically check documented semantic fallbacks before using built-in defaults.

Crucially, **intentional neutral gray RGB `(128, 128, 128)` (`#808080`) is fully preserved**. Older heuristic color sentinel checks have been replaced with explicit presence checks (`try_color`), meaning dark/low-saturation design systems reproduce accurately.

---

### Legacy JSON and Migration Notes

Earlier versions supported a JSON-based schema. While JSON files can be converted easily to the canonical TOML format, all active and new themes should use `.toml`. If you have a legacy JSON theme:

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

#### Full Token Reference

Below is the complete list of JSON keys accepted in the `"colors"` object. All tokens are optional — omitted tokens are auto-derived from the core palette or fall back to built-in defaults.

**Core UI**

| Token | Description |
|-------|-------------|
| `accent` | Primary accent color (buttons, links, highlights) |
| `border` | Default border color |
| `borderAccent` | Focused / active border color |
| `borderMuted` | De-emphasised border color |
| `success` | Success indicators (green) |
| `error` | Error indicators (red) |
| `warning` | Warning indicators (amber) |
| `muted` | Muted text (comments, secondary labels) |
| `dim` | Dim text (timestamps, metadata) |
| `text` | Primary body text color |
| `thinkingText` | Text color inside thinking/reasoning blocks |

**Backgrounds & content**

| Token | Description |
|-------|-------------|
| `selectedBg` | Selection highlight background |
| `userMessageBg` | User message card background |
| `userMessageText` | User message text color |
| `customMessageBg` | System/custom message background |
| `customMessageText` | System/custom message text |
| `customMessageLabel` | Label color for custom messages |
| `toolPendingBg` | Background while a tool call is running |
| `toolSuccessBg` | Background for successful tool results |
| `toolErrorBg` | Background for errored tool results |
| `toolTitle` | Tool call header text |
| `toolOutput` | Tool output body text |

**Markdown rendering**

| Token | Description |
|-------|-------------|
| `mdHeading` | Heading text (`# H1`, `## H2`, …) |
| `mdLink` | Link label text |
| `mdLinkUrl` | Link URL text |
| `mdCode` | Inline code spans |
| `mdCodeBlock` | Code block body text |
| `mdCodeBlockBorder` | Code block border (`┌──` / `└──`) |
| `mdQuote` | Block-quote text |
| `mdQuoteBorder` | Block-quote left border |
| `mdHr` | Horizontal rule |
| `mdListBullet` | List bullet / number |

**Diffs**

| Token | Description |
|-------|-------------|
| `toolDiffAdded` | Added line color |
| `toolDiffRemoved` | Removed line color |
| `toolDiffContext` | Context line color |

**Syntax highlighting** (code blocks)

| Token | Description |
|-------|-------------|
| `syntaxComment` | Comments |
| `syntaxKeyword` | Keywords (`fn`, `let`, `if`, …) |
| `syntaxFunction` | Function names |
| `syntaxVariable` | Variables |
| `syntaxString` | String literals |
| `syntaxNumber` | Numeric literals |
| `syntaxType` | Type names |
| `syntaxOperator` | Operators |
| `syntaxPunctuation` | Punctuation |

**Thinking level indicators**

| Token | Description |
|-------|-------------|
| `thinkingOff` | Thinking disabled indicator |
| `thinkingMinimal` | Minimal thinking indicator |
| `thinkingLow` | Low thinking indicator |
| `thinkingMedium` | Medium thinking indicator |
| `thinkingHigh` | High thinking indicator |
| `thinkingXhigh` | Extra-high thinking indicator (falls back to `error` if omitted) |

**Bash mode**

| Token | Description |
|-------|-------------|
| `bashMode` | Bash mode border indicator (falls back to `warning` if omitted) |

**Extended tokens** *(optional — auto-derived when absent)*

These tokens give fine-grained control over the context bar, spinner, and border style. When omitted they are automatically derived from the core palette.

| Token | Description | Auto-derived from |
|-------|-------------|-------------------|
| `borderStyle` | Border character style: `"rounded"`, `"thick"`, `"plain"`, or `"double"` | Default: `"rounded"` |
| `spinnerAccent` | Base color for the animated spinner gradient (4 steps are generated automatically) | `accent` |
| `ctxBarSystem` | Context bar: system prompt segment | `muted` (dimmed) |
| `ctxBarNativeTools` | Context bar: native tools segment | `accent` |
| `ctxBarMcpTools` | Context bar: MCP tools segment | `accent` (brightened) |
| `ctxBarMemory` | Context bar: memory segment | `warning` |
| `ctxBarSkills` | Context bar: skills segment | `warning` (brightened) |
| `ctxBarMessages` | Context bar: messages segment | `syntaxKeyword` |
| `ctxBarFree` | Context bar: free/unused segment | `dim` |
| `ctxBarBuffer` | Context bar: autocompact buffer segment | `border` |
```

## Neovim Integration

CADE natively parses `.tmTheme` files, so external Lua plugins are no longer necessary for syncing themes. You can download the `.tmTheme` artifact of any Neovim colorscheme (TokyoNight, Catppuccin, RosePine) and put it into `~/.cade/themes/` directly.

However, if you still want to dynamically force CADE to synchronize automatically whenever you run `:colorscheme` inside a live Neovim instance, you can use the **Legacy Export Approach**.

### Standalone Plugin (Legacy)

The `cade.nvim` plugin is a minimal Lua script that extracts colors directly from Neovim highlight groups and exports a `.json` theme format to CADE. Since it is hosted in the main CADE monorepo, point your package manager to the `editors/neovim` directory.

Using **lazy.nvim**:
```lua
{
  "EzekTec-Inc/cade",
  config = function(plugin)
    vim.opt.rtp:append(plugin.dir .. "/editors/neovim")
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
