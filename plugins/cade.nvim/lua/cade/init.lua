local M = {}

M.config = {
    auto_export = false,
    theme_name = "nvim-exported"
}

-- helper to convert int color to #RRGGBB
local function int_to_hex(int_color)
    if not int_color then return nil end
    return string.format("#%06x", int_color)
end

-- helper to get a highlight property (fg or bg)
local function get_color(group_name, prop)
    local hl = vim.api.nvim_get_hl(0, { name = group_name, link = false })
    -- Neovim 0.9+ nvim_get_hl returns a table that might contain .fg or .bg
    -- If link=false, it resolves the link natively in newer nvim versions, 
    -- but for safety we might want to manually walk the link if it's there.
    if hl and hl.link then
        hl = vim.api.nvim_get_hl(0, { name = hl.link, link = false })
    end
    if hl and hl[prop] then
        return int_to_hex(hl[prop])
    end
    return nil
end

local function resolve_color(primary, fallback_1, fallback_2, prop)
    return get_color(primary, prop) or get_color(fallback_1, prop) or (fallback_2 and get_color(fallback_2, prop))
end

function M.export_theme()
    local function c_fg(primary, f1, f2) return resolve_color(primary, f1, f2, "fg") end
    local function c_bg(primary, f1, f2) return resolve_color(primary, f1, f2, "bg") end

    local text_color = c_fg("Normal", "Normal") or "#FFFFFF"
    local normal_bg = c_bg("Normal", "Normal") or ""
    
    local accent = c_fg("Statement", "Function", "Keyword") or "#000000"
    local dim = c_fg("NonText", "Conceal") or "#888888"

    local colors = {
        accent = accent,
        border = c_fg("FloatBorder", "WinSeparator", "LineNr"),
        borderAccent = c_fg("TelescopeBorder", "TelescopeBorder") or accent,
        borderMuted = c_fg("Comment", "NonText"),
        success = c_fg("DiagnosticOk", "String"),
        error = c_fg("DiagnosticError", "ErrorMsg"),
        warning = c_fg("DiagnosticWarn", "WarningMsg"),
        muted = c_fg("Comment", "LineNr"),
        dim = dim,
        text = text_color,
        thinkingText = c_fg("Comment", "Comment") or dim,
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

    -- Remove nil values and fallback to empty string for safety, though CADE 
    -- expects hex strings for all colors. If a background is empty, it means transparent.
    for k, v in pairs(colors) do
        if not v then
            colors[k] = ""
        end
    end

    local theme = {
        name = M.config.theme_name,
        author = "cade.nvim",
        colors = colors
    }

    local json_str = vim.fn.json_encode(theme)
    
    local home = os.getenv("HOME")
    if not home then return end
    
    local theme_dir = home .. "/.cade/themes"
    vim.fn.mkdir(theme_dir, "p")
    
    local filepath = theme_dir .. "/" .. M.config.theme_name .. ".json"
    local file = io.open(filepath, "w")
    if file then
        file:write(json_str)
        file:close()
    else
        vim.notify("CADE: Failed to write theme file to " .. filepath, vim.log.levels.ERROR)
    end
end

function M.setup(opts)
    if opts then
        M.config = vim.tbl_deep_extend("force", M.config, opts)
    end
    
    if M.config.auto_export then
        vim.api.nvim_create_autocmd("ColorScheme", {
            pattern = "*",
            callback = function()
                M.export_theme()
            end,
        })
        -- Export immediately on setup
        M.export_theme()
    end
    
    vim.api.nvim_create_user_command("CadeExportTheme", function()
        M.export_theme()
        vim.notify("CADE theme exported to ~/.cade/themes/" .. M.config.theme_name .. ".json", vim.log.levels.INFO)
    end, {})
end

return M
