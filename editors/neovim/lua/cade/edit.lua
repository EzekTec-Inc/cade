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
  local raw_stdout = ""
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

      raw_stdout = raw_stdout .. chunk
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
        vim.schedule(function()
          local err_msg = "cade.nvim: curl exited with code " .. result.code
          if raw_stdout ~= "" then
            local clean_body = vim.trim(raw_stdout)
            if clean_body:find("Unauthorized") or clean_body:find("invalid API key") then
              err_msg = "CADE server returned 401 Unauthorized. Please check that CADE_API_KEY is configured correctly on both server and client."
            else
              err_msg = err_msg .. "\nServer response: " .. clean_body
            end
          elseif result.stderr and result.stderr ~= "" then
            err_msg = err_msg .. "\nError: " .. vim.trim(result.stderr)
          end
          on_error(err_msg)
        end)
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

local function get_visual_selection()
  local s_pos = vim.fn.getpos("'<")
  local e_pos = vim.fn.getpos("'>")
  local mode = vim.fn.visualmode()

  -- Use robust getregion (Neovim >= 0.10 native, zero column-math crashes)
  local region_lines = {}
  local ok, err_reg = pcall(function()
    region_lines = vim.fn.getregion(s_pos, e_pos, { type = mode })
  end)
  if not ok or not region_lines or #region_lines == 0 then
    -- Fallback to standard line collection
    region_lines = vim.api.nvim_buf_get_lines(0, s_pos[2] - 1, e_pos[2], false)
  end
  local selected_text = table.concat(region_lines, "\n")

  local start_row = s_pos[2] - 1
  local start_col = s_pos[3] - 1
  local end_row = e_pos[2] - 1
  local end_col = e_pos[3]

  local last_line = vim.api.nvim_buf_get_lines(0, end_row, end_row + 1, true)[1] or ""
  local line_len = string.len(last_line)

  if mode == "V" then
    start_col = 0
    end_col = line_len
  else
    -- Clamp end_col safely
    if end_col > line_len or end_col < 0 then
      end_col = line_len
    end
    if start_col < 0 then
      start_col = 0
    end
  end

  return selected_text, start_row, start_col, end_row, end_col, mode
end

local function replace_text(buf, start_row, start_col, end_row, end_col, new_text)
  local new_lines = vim.split(new_text, "\n", { plain = true })
  vim.api.nvim_buf_set_text(buf, start_row, start_col, end_row, end_col, new_lines)
end

local hint_ns = vim.api.nvim_create_namespace("cade_edit_hint")

function M.update_visual_hint()
  local mode = vim.fn.mode()
  if mode ~= "v" and mode ~= "V" and mode ~= "\22" then
    vim.api.nvim_buf_clear_namespace(0, hint_ns, 0, -1)
    return
  end
  
  local v_pos = vim.fn.getpos("v")
  local cur_pos = vim.fn.getpos(".")
  local row = math.max(v_pos[2], cur_pos[2]) - 1
  
  vim.api.nvim_buf_clear_namespace(0, hint_ns, 0, -1)
  
  local cfg = require("cade.config").get()
  local key = (cfg.keymaps and cfg.keymaps.edit) or "<leader>ce"
  
  local ok, err = pcall(vim.api.nvim_buf_set_extmark, 0, hint_ns, row, 0, {
    virt_text = { { " [" .. key .. ": ask cade]", "DiagnosticInfo" } },
    virt_text_pos = "eol",
    hl_mode = "combine",
  })
  if not ok then
    vim.notify("Hint error: " .. tostring(err), vim.log.levels.WARN)
  end
end

function M.setup_hints()
  local group = vim.api.nvim_create_augroup("CadeEditHints", { clear = true })
  vim.api.nvim_create_autocmd({ "CursorMoved", "ModeChanged" }, {
    group = group,
    pattern = "*",
    callback = M.update_visual_hint,
  })
end

function M.hover_edit()
  local mode = vim.fn.mode()
  if mode ~= "v" and mode ~= "V" and mode ~= "\22" then
    vim.notify("CADE edit requires a visual selection", vim.log.levels.WARN)
    return
  end
  
  -- Escape to normal mode to set '< and '> marks
  vim.api.nvim_feedkeys(vim.api.nvim_replace_termcodes("<Esc>", true, false, true), "x", false)

  vim.schedule(function()
    local selected_text, s_row, s_col, e_row, e_col, mode = get_visual_selection()
    local buf = vim.api.nvim_get_current_buf()
    
    local sel_ns = vim.api.nvim_create_namespace("cade_edit_sel")
    local sel_opts = {
      end_row = e_row,
      end_col = e_col,
      hl_group = "Visual",
      priority = 10000,
    }
    if mode == "V" then
      sel_opts.hl_eol = true
    end
    local sel_extmark = vim.api.nvim_buf_set_extmark(buf, sel_ns, s_row, s_col, sel_opts)
    
    local prefix_lines = vim.api.nvim_buf_get_lines(buf, math.max(0, s_row - 50), s_row, false)
    if #prefix_lines > 0 then
      local partial_start = ""
      pcall(function()
        partial_start = vim.api.nvim_buf_get_text(buf, s_row, 0, s_row, s_col, {})[1] or ""
      end)
      table.insert(prefix_lines, partial_start)
    end
    local prefix = table.concat(prefix_lines, "\n")
    
    local suffix_lines = vim.api.nvim_buf_get_lines(buf, e_row + 1, e_row + 20, false)
    local partial_end = ""
    pcall(function()
      local partial_end_lines = vim.api.nvim_buf_get_text(buf, e_row, e_col, e_row, -1, {})
      partial_end = partial_end_lines[1] or ""
    end)
    table.insert(suffix_lines, 1, partial_end)
    local suffix = table.concat(suffix_lines, "\n")
    
    local language = vim.bo[buf].filetype
    
    local prompt_buf = vim.api.nvim_create_buf(false, true)
    vim.bo[prompt_buf].filetype = "markdown"
    vim.api.nvim_buf_set_option(prompt_buf, "bufhidden", "wipe")
    
    local win_opts = {
      relative = "cursor",
      row = 1,
      col = 0,
      width = math.min(80, vim.o.columns - 4),
      height = 1,
      style = "minimal",
      border = "rounded",
      title = " ✨ CADE Edit ",
      title_pos = "center"
    }
    
    local prompt_win = vim.api.nvim_open_win(prompt_buf, true, win_opts)
    
    -- Modern UI styling and wrapping
    vim.api.nvim_set_option_value("wrap", true, { win = prompt_win })
    vim.api.nvim_set_option_value("linebreak", true, { win = prompt_win })
    vim.api.nvim_set_option_value("breakindent", true, { win = prompt_win })
    vim.api.nvim_set_option_value("winhl", "Normal:NormalFloat,FloatBorder:FloatBorder,FloatTitle:Title", { win = prompt_win })
    
    local max_h = math.max(20, math.floor(vim.o.lines * 0.8))
    
    -- Dynamic resizing while typing
    vim.api.nvim_create_autocmd({ "TextChanged", "TextChangedI" }, {
      buffer = prompt_buf,
      callback = function()
        local h = vim.api.nvim_win_text_height(prompt_win, {}).all
        if h > 0 then
          vim.api.nvim_win_set_config(prompt_win, { height = math.min(h, max_h) })
        end
      end
    })
    
    vim.cmd("startinsert")
    
    local cancel = nil
    local accumulated_response = ""
    local error_occurred = false
    local response_start_row = 0
    local is_streaming = false
    
    local function close_all()
      if cancel then cancel() end
      pcall(vim.api.nvim_win_close, prompt_win, true)
      pcall(vim.api.nvim_buf_del_extmark, buf, sel_ns, sel_extmark)
    end
    
    local function submit_or_apply(is_ctrl_s)
      if is_streaming then
        vim.notify("Wait for the edit to finish streaming, or press Esc to cancel.", vim.log.levels.INFO)
        return
      end
      
      if error_occurred then
        error_occurred = false
      end
      
      -- If we already have a response, apply it (only via <C-s>)
      if accumulated_response ~= "" then
        if not is_ctrl_s then
          -- Do not apply on Enter, only on <C-s>!
          return
        end
        local start_pos = vim.api.nvim_buf_get_extmark_by_id(buf, sel_ns, sel_extmark, {details=true})
        if #start_pos > 0 then
          local cur_s_row, cur_s_col, details = start_pos[1], start_pos[2], start_pos[3]
          local cur_e_row, cur_e_col = details.end_row, details.end_col
          replace_text(buf, cur_s_row, cur_s_col, cur_e_row, cur_e_col, accumulated_response)
        end
        close_all()
        return
      end
      
      -- Otherwise, submit instruction
      local lines = vim.api.nvim_buf_get_lines(prompt_buf, 0, -1, false)
      
      -- If there is a separator from a previous failed run, truncate it to retry cleanly
      local separator_idx = nil
      for i, line in ipairs(lines) do
        if line == "---" then
          separator_idx = i
          break
        end
      end
      
      if separator_idx then
        local new_lines = {}
        for i = 1, separator_idx - 1 do
          table.insert(new_lines, lines[i])
        end
        -- Remove the separator and everything after it
        vim.api.nvim_buf_set_lines(prompt_buf, separator_idx - 1, -1, false, {})
        lines = new_lines
      end

      local instruction = vim.trim(table.concat(lines, "\n"))
      if instruction == "" then return end
      
      vim.cmd("stopinsert")
      
      vim.api.nvim_buf_set_lines(prompt_buf, -1, -1, false, { "", "---", "", "```" .. language, "```" })
      response_start_row = vim.api.nvim_buf_line_count(prompt_buf) - 1
      
      vim.api.nvim_win_set_config(prompt_win, { title = " ✨ CADE Edit (Streaming...) ", title_pos = "center" })
      
      is_streaming = true
      
      cancel = M.fetch_edit(prefix, selected_text, suffix, instruction, language, 
        function(snap)
          accumulated_response = snap
          local rsp_lines = vim.split(snap, "\n", {plain=true})
          table.insert(rsp_lines, "```")
          local ok = pcall(vim.api.nvim_buf_set_lines, prompt_buf, response_start_row, -1, false, rsp_lines)
          if not ok then return end
          
          local ok_h, new_height = pcall(function() return vim.api.nvim_win_text_height(prompt_win, {}).all end)
          if ok_h and new_height > 0 then
            pcall(vim.api.nvim_win_set_config, prompt_win, { height = math.min(max_h, new_height) })
          end
          
          -- Auto-scroll to the bottom as new lines stream in
          local ok_c, line_count = pcall(vim.api.nvim_buf_line_count, prompt_buf)
          if ok_c then
            pcall(vim.api.nvim_win_set_cursor, prompt_win, {line_count, 0})
          end
        end,
        function()
          is_streaming = false
          cancel = nil
          pcall(vim.api.nvim_win_set_config, prompt_win, { title = " ✨ Press <C-s> to Apply, Esc to Cancel ", title_pos = "center" })
        end,
        function(err)
          is_streaming = false
          cancel = nil
          error_occurred = true
          vim.notify("CADE Edit error: " .. err, vim.log.levels.ERROR)
          pcall(vim.api.nvim_win_set_config, prompt_win, { title = " ✨ Error ", title_pos = "center" })
        end
      )
    end
    
    vim.keymap.set("n", "<Esc>", close_all, { buffer = prompt_buf })
    vim.keymap.set("i", "<Esc>", function()
      vim.cmd("stopinsert")
    end, { buffer = prompt_buf })
    
    -- Use <C-s> or <CR> to submit or apply
    vim.keymap.set("n", "<C-s>", function() submit_or_apply(true) end, { buffer = prompt_buf })
    vim.keymap.set("i", "<C-s>", function() submit_or_apply(true) end, { buffer = prompt_buf })
    
    vim.keymap.set("n", "<CR>", function() submit_or_apply(false) end, { buffer = prompt_buf })
    vim.keymap.set("i", "<CR>", function() submit_or_apply(false) end, { buffer = prompt_buf })
  end)
end

M._get_visual_selection = get_visual_selection
M._replace_text = replace_text

return M