-- cade/edit.lua
local M = {}
local http = require("cade.http")

function M.fetch_edit(prefix, selected_text, suffix, instruction, language, on_token, on_done, on_error)
  local cfg = require("cade.config").get()

  if cfg.agent_id == "" then
    on_error("cade.nvim: agent_id not configured")
    return function() end
  end

  local url = string.format("http://127.0.0.1:%d/v1/agents/%s/edit", cfg.server_port, cfg.agent_id)

  local body = vim.json.encode({
    prefix        = prefix,
    selected_text = selected_text,
    suffix        = suffix,
    instruction   = instruction,
    language      = language,
    max_tokens    = 4096,
    model         = cfg.model ~= "" and cfg.model or vim.NIL,
  })

  local headers = { "-H", "Content-Type: application/json", "-H", "Accept: text/event-stream" }
  if cfg.api_key ~= "" then
    vim.list_extend(headers, { "-H", "Authorization: Bearer " .. cfg.api_key })
  end

  local cmd = vim.list_extend({ "curl", "--silent", "--fail-with-body", "--show-error", "--no-buffer", "-N", "-X", "POST", "-d", body }, headers)
  table.insert(cmd, url)

  local accumulated = ""
  local sse_buffer = ""
  local done = false

  local handle = vim.system(cmd, {
    text = true,
    stdout = function(err, chunk)
      if done then return end
      if err then
        vim.schedule(function() on_error(err) end)
        return
      end
      if not chunk then return end

      sse_buffer = sse_buffer .. chunk
      local lines = vim.split(sse_buffer, "\n", { plain = true })
      sse_buffer = table.remove(lines) or ""

      for _, line in ipairs(lines) do
        local parsed = http._parse_sse_line(line)
        if parsed then
          if parsed.type == "done" then
            done = true
            vim.schedule(on_done)
            return
          elseif parsed.type == "delta" then
            accumulated = accumulated .. parsed.content
            local snap = accumulated
            vim.schedule(function() on_token(snap) end)
          elseif parsed.type == "error" then
            done = true
            vim.schedule(function() on_error(parsed.message) end)
            return
          end
        end
      end
    end,
  }, function(result)
    if not done then
      if result.code ~= 0 then
        vim.schedule(function() on_error("cade.nvim: curl exited with code " .. result.code) end)
      else
        vim.schedule(on_done)
      end
    end
  end)

  return function()
    done = true
    pcall(function() handle:kill(9) end)
  end
end

local local _, start_row, start_col, _ = unpack(vim.fn.getpos("'<"))\n  local _, end_row, end_col, _ = unpack(vim.fn.getpos("'>"))\n  start_row = start_row - 1\n  end_row = end_row - 1\n  start_col = start_col - 1\n  -- end_col from getpos is 1-indexed inclusive, which perfectly matches 0-indexed exclusive\n\n  if start_row > end_row or (start_row == end_row and start_col > end_col) then\n    start_row, end_row = end_row, start_row\n    start_col, end_col = end_col, start_col\n  end\n  \n  -- For line mode ('V'), end_col is usually v:maxcol, we need to clamp it\n  local line_len = string.len(vim.api.nvim_buf_get_lines(0, end_row, end_row+1, true)[1] or "")\n  if end_col > line_len then\n    end_col = line_len\n  end\n\n  local lines = vim.api.nvim_buf_get_text(0, start_row, start_col, end_row, end_col, {})\n  return table.concat(lines, "\\n"), start_row, start_col, end_row, end_col

local function replace_text(buf, start_row, start_col, end_row, end_col, new_text)
  local new_lines = vim.split(new_text, "\n", { plain = true })
  vim.api.nvim_buf_set_text(buf, start_row, start_col, end_row, end_col, new_lines)
end

local hint_ns = vim.api.nvim_create_namespace("cade_edit_hint")

local mode = vim.fn.mode()\n  if mode ~= "v" and mode ~= "V" and mode ~= "\\22" then\n    vim.api.nvim_buf_clear_namespace(0, hint_ns, 0, -1)\n    return\n  end\n  \n  local _, s_row, _, _ = unpack(vim.fn.getpos("'<"))\n  local _, e_row, _, _ = unpack(vim.fn.getpos("'>"))\n  \n  local row = math.max(s_row, e_row) - 1\n  if row < 0 then row = 0 end\n  \n  vim.api.nvim_buf_clear_namespace(0, hint_ns, 0, -1)\n  \n  local cfg = require("cade.config").get()\n  local key = (cfg.keymaps and cfg.keymaps.edit) or "<leader>ce"\n  \n  pcall(vim.api.nvim_buf_set_extmark, 0, hint_ns, row, 0, {\n    virt_text = { { " [" .. key .. ": ask cade]", "DiagnosticInfo" } },\n    virt_text_pos = "eol",\n    hl_mode = "combine",\n  })

function M.setup_hints()
  local group = vim.api.nvim_create_augroup("CadeEditHints", { clear = true })
  vim.api.nvim_create_autocmd({ "CursorMoved", "ModeChanged" }, {
    group = group,
    callback = M.update_visual_hint,
  })
end

function M.hover_edit()\n  local mode = vim.fn.mode()\n  if mode ~= "v" and mode ~= "V" and mode ~= "\\22" then\n    vim.notify("CADE edit requires a visual selection", vim.log.levels.WARN)\n    return\n  end\n  \n  -- Escape to normal mode to set '< and '> marks\n  vim.api.nvim_feedkeys(vim.api.nvim_replace_termcodes("<Esc>", true, false, true), "x", false)\n\n  vim.schedule(function()\n    local selected_text, s_row, s_col, e_row, e_col = get_visual_selection()\n    local buf = vim.api.nvim_get_current_buf()\n    \n    local sel_ns = vim.api.nvim_create_namespace("cade_edit_sel")\n    local sel_extmark = vim.api.nvim_buf_set_extmark(buf, sel_ns, s_row, s_col, {\n      end_row = e_row,\n      end_col = e_col,\n      hl_group = "IncSearch",\n    })\n    \n    local prefix_lines = vim.api.nvim_buf_get_lines(buf, math.max(0, s_row - 50), s_row, false)\n    if #prefix_lines > 0 then\n      local partial_start = vim.api.nvim_buf_get_text(buf, s_row, 0, s_row, s_col, {})[1] or ""\n      table.insert(prefix_lines, partial_start)\n    end\n    local prefix = table.concat(prefix_lines, "\\n")\n    \n    local suffix_lines = vim.api.nvim_buf_get_lines(buf, e_row + 1, e_row + 20, false)\n    local partial_end_lines = vim.api.nvim_buf_get_text(buf, e_row, e_col, e_row, -1, {})\n    local partial_end = partial_end_lines[1] or ""\n    table.insert(suffix_lines, 1, partial_end)\n    local suffix = table.concat(suffix_lines, "\\n")\n    \n    local language = vim.bo[buf].filetype\n    \n    local prompt_buf = vim.api.nvim_create_buf(false, true)\n    vim.bo[prompt_buf].filetype = "markdown"\n    vim.api.nvim_buf_set_option(prompt_buf, "bufhidden", "wipe")\n    \n    local win_opts = {\n      relative = "cursor",\n      row = 1,\n      col = 0,\n      width = math.min(80, vim.o.columns - 4),\n      height = 1,\n      style = "minimal",\n      border = "rounded",\n      title = " ✨ CADE Edit (Ctrl+S to Submit) ",\n      title_pos = "center"\n    }\n    \n    local prompt_win = vim.api.nvim_open_win(prompt_buf, true, win_opts)\n    \n    vim.api.nvim_set_option_value("wrap", true, { win = prompt_win })\n    vim.api.nvim_set_option_value("linebreak", true, { win = prompt_win })\n    vim.api.nvim_set_option_value("breakindent", true, { win = prompt_win })\n    vim.api.nvim_set_option_value("winhl", "Normal:NormalFloat,FloatBorder:FloatBorder,FloatTitle:Title", { win = prompt_win })\n    \n    local max_h = math.max(20, math.floor(vim.o.lines * 0.8))\n    \n    vim.api.nvim_create_autocmd({ "TextChanged", "TextChangedI" }, {\n      buffer = prompt_buf,\n      callback = function()\n        local h = vim.api.nvim_win_text_height(prompt_win, {}).all\n        if h > 0 then\n          vim.api.nvim_win_set_config(prompt_win, { height = math.min(h, max_h) })\n        end\n      end\n    })\n    \n    vim.cmd("startinsert")\n    \n    local cancel = nil\n    \n    local function close_all()\n      if cancel then cancel() end\n      pcall(vim.api.nvim_win_close, prompt_win, true)\n      pcall(vim.api.nvim_buf_del_extmark, buf, sel_ns, sel_extmark)\n    end\n    \n    local function submit()\n      local instruction = table.concat(vim.api.nvim_buf_get_lines(prompt_buf, 0, -1, false), "\\n")\n      if instruction == vim.trim("") then return end\n      \n      vim.cmd("stopinsert")\n      \n      vim.api.nvim_buf_set_lines(prompt_buf, -1, -1, false, { "", "---", "", "```" .. language, "```" })\n      local response_start_row = vim.api.nvim_buf_line_count(prompt_buf) - 1\n      \n      vim.api.nvim_win_set_config(prompt_win, { title = " ✨ CADE Edit (Streaming...) ", title_pos = "center" })\n      \n      local accumulated_response = ""\n      \n      local function apply_and_close()\n        if not cancel then\n          local start_pos = vim.api.nvim_buf_get_extmark_by_id(buf, sel_ns, sel_extmark, {details=true})\n          if #start_pos > 0 and accumulated_response ~= "" then\n            local cur_s_row, cur_s_col, details = start_pos[1], start_pos[2], start_pos[3]\n            local cur_e_row, cur_e_col = details.end_row, details.end_col\n            replace_text(buf, cur_s_row, cur_s_col, cur_e_row, cur_e_col, accumulated_response)\n          end\n          close_all()\n        end\n      end\n      \n      vim.keymap.set("n", "<CR>", apply_and_close, { buffer = prompt_buf })\n      vim.keymap.set("n", "<C-s>", apply_and_close, { buffer = prompt_buf })\n      vim.keymap.set("n", "<Esc>", close_all, { buffer = prompt_buf })\n      \n      cancel = M.fetch_edit(prefix, selected_text, suffix, instruction, language, \n        function(snap)\n          accumulated_response = snap\n          local lines = vim.split(snap, "\\n", {plain=true})\n          table.insert(lines, "```")\n          vim.api.nvim_buf_set_lines(prompt_buf, response_start_row, -1, false, lines)\n          \n          local new_height = vim.api.nvim_win_text_height(prompt_win, {}).all\n          if new_height > 0 then\n            vim.api.nvim_win_set_config(prompt_win, { height = math.min(max_h, new_height) })\n          end\n          \n          local line_count = vim.api.nvim_buf_line_count(prompt_buf)\n          pcall(vim.api.nvim_win_set_cursor, prompt_win, {line_count, 0})\n        end,\n        function()\n          cancel = nil\n          vim.api.nvim_win_set_config(prompt_win, { title = " ✨ Press Ctrl+S to Apply, Esc to Cancel ", title_pos = "center" })\n        end,\n        function(err)\n          cancel = nil\n          vim.notify("CADE Edit error: " .. err, vim.log.levels.ERROR)\n          vim.api.nvim_win_set_config(prompt_win, { title = " ✨ Error ", title_pos = "center" })\n        end\n      )\n    end\n    \n    vim.keymap.set("i", "<C-s>", submit, { buffer = prompt_buf })\n    vim.keymap.set("n", "<C-s>", submit, { buffer = prompt_buf })\n    \n    vim.keymap.set("n", "<Esc>", close_all, { buffer = prompt_buf })\n    vim.keymap.set("i", "<Esc>", function()\n      vim.cmd("stopinsert")\n      close_all()\n    end, { buffer = prompt_buf })\n  end)\nend

return M