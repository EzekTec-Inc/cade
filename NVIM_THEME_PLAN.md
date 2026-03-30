# Neovim Theme Integration Plan (Export Plugin Approach)

This document outlines the implementation plan for enabling Neovim colorscheme synchronization in CADE using the **Export Plugin Approach**. 

This approach keeps CADE's core zero-bloat by shifting the extraction logic to a lightweight Neovim Lua script. Neovim will export its active colorscheme to a CADE-compatible JSON file, and CADE will pick it up instantly via its existing hot-reloading mechanism.

---

## Phase 1: The Neovim Lua Exporter (`cade.nvim`)

**Goal:** Create a Lua script that runs inside Neovim, extracts colors from active highlight groups, maps them to CADE's semantic tokens, and writes a valid CADE theme JSON file.

### 1. Highlight Group Extraction
Use `vim.api.nvim_get_hl(0, { name = "GroupName", link = false })` to extract exact RGB hex values from Neovim's current colorscheme. 
We need a helper function to convert the integer color values returned by the API into `#RRGGBB` hex strings.

### 2. Semantic Mapping Table
Map Neovim's standard highlight groups (and popular plugin groups as fallbacks) to CADE's `ThemeTokens`:

| CADE Token | Neovim Highlight Group (Primary) | Fallback |
| :--- | :--- | :--- |
| `accent` | `Statement` or `Function` | `Keyword` |
| `border` | `FloatBorder` | `WinSeparator` or `LineNr` |
| `borderAccent` | `TelescopeBorder` (if exists) | `accent` |
| `borderMuted` | `Comment` | `NonText` |
| `success` | `DiagnosticOk` | `String` |
| `error` | `DiagnosticError` | `ErrorMsg` |
| `warning` | `DiagnosticWarn` | `WarningMsg` |
| `muted` | `Comment` | `LineNr` |
| `dim` | `NonText` | `Conceal` |
| `text` | `Normal.fg` | `#FFFFFF` |
| `thinkingText` | `Comment` | `dim` |
| `selectedBg` | `Visual` | `CursorLine` |
| `userMessageBg` | `NormalFloat.bg` | `CursorLine` |
| `userMessageText` | `NormalFloat.fg` | `Normal.fg` |
| `customMessageBg` | `NormalFloat.bg` | `Normal.bg` |
| `customMessageText` | `NormalFloat.fg` | `Normal.fg` |
| `toolPendingBg` | `CursorLine` | `ColorColumn` |
| `toolSuccessBg` | `DiffAdd` | `Normal.bg` |
| `toolErrorBg` | `DiffDelete` | `ErrorMsg.bg` |
| `toolTitle` | `Title` | `Function` |
| `toolOutput` | `Normal.fg` | `text` |

*(Note: The Lua script must handle `NONE` or transparent backgrounds gracefully, perhaps falling back to terminal defaults `""` or a dark gray).*

### 3. JSON Generation
Construct a Lua table matching CADE's schema:
```lua
{
  name = "nvim-exported",
  author = "cade.nvim",
  colors = {
    accent = "#...",
    border = "#...",
    -- ... mapped tokens
  }
}
```
Serialize this table to JSON (using `vim.fn.json_encode`).

### 4. File I/O
Write the JSON string to the standard CADE global themes directory:
`~/.cade/themes/nvim-exported.json`

### 5. Automation (Autocmd)
Set up a Neovim `autocmd` to trigger the export function automatically whenever the colorscheme changes:
```lua
vim.api.nvim_create_autocmd("ColorScheme", {
    pattern = "*",
    callback = function()
        require("cade").export_theme()
    end,
})
```

---

## Phase 2: CADE Configuration

**Goal:** Instruct CADE to use the exported theme.

Since CADE already supports hot-reloading and dynamic theme switching (implemented previously), the user only needs to do one of two things:

1. **Interactive:** Run `/theme nvim-exported` in the CADE REPL.
2. **Persistent:** Update their `~/.cade/settings.json`:
   ```json
   {
     "theme": "nvim-exported"
   }
   ```

Because CADE watches the settings and (implicitly) the active theme, updates from Neovim will instantly reflect in the CADE TUI.

---

## Phase 3: Documentation & Distribution

**Goal:** Make it trivial for users to adopt.

1. **Documentation Snippet:** 
   Add a new section in `docs/themes.md` (or a new `docs/neovim.md`) providing a copy-pasteable Lua snippet for users who want to drop it directly into their `init.lua`.
2. **Standalone Plugin:**
   Create a dedicated GitHub repository (e.g., `EzekTec-Inc/cade.nvim`) or host this Lua code inside the monorepo at `plugins/cade.nvim`. This allows users to install it via Lazy/Packer pointing to the subdirectory:
   ```lua
   {
     "EzekTec-Inc/cade",
     config = function(plugin)
       vim.opt.rtp:append(plugin.dir .. "/plugins/cade.nvim")
       require("cade").setup({ auto_export = true })
     end
   }
   ```
3. **Demo:** Create a GIF showing a user changing their Neovim colorscheme (e.g., `:colorscheme tokyonight` then `:colorscheme gruvbox`) and the CADE terminal window instantly updating its colors to match.