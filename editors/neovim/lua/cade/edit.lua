-- cade/edit.lua — interactive edits with explicit review and guarded application.
local M = {}
local http = require("cade.http")
local sessions = require("cade.edit_session")

function M.fetch_edit(prefix, selected_text, suffix, instruction, language, on_token, on_done, on_error)
  local cfg = require("cade.config").get()
  local cancelled, terminal = false, false
  local function deliver(callback, value)
    vim.schedule(function()
      if not cancelled then callback(value) end
    end)
  end
  local function finish(callback, value)
    if cancelled or terminal then return end
    terminal = true
    deliver(callback, value)
  end
  local handle
  local function cancel()
    cancelled = true
    if handle then pcall(function() handle:kill(15) end) end
  end
  if cfg.agent_id == "" then
    finish(on_error, "cade.nvim: agent_id not configured")
    return cancel
  end
  local body = vim.json.encode({
    prefix = prefix, selected_text = selected_text, suffix = suffix,
    instruction = instruction, language = language, max_tokens = 4096,
    model = cfg.model ~= "" and cfg.model or vim.NIL,
  })
  local cmd = { "curl", "--silent", "--fail-with-body", "--show-error", "--no-buffer", "-N", "-X", "POST",
    "-d", body, "-H", "Content-Type: application/json", "-H", "Accept: text/event-stream" }
  if cfg.api_key ~= "" then vim.list_extend(cmd, { "-H", "Authorization: Bearer " .. cfg.api_key }) end
  table.insert(cmd, string.format("http://127.0.0.1:%d/v1/agents/%s/edit", cfg.server_port, cfg.agent_id))
  local accumulated, pending, raw = "", "", ""
  local function line_received(line)
    if cancelled or terminal then return end
    local parsed = http._parse_sse_line(line)
    if not parsed then return end
    if parsed.type == "done" then
      finish(on_done)
    elseif parsed.type == "error" then
      finish(on_error, parsed.message)
    elseif parsed.type == "delta" then
      accumulated = accumulated .. parsed.content
      deliver(on_token, accumulated)
    end
  end
  local ok, result = pcall(vim.system, cmd, {
    text = true,
    stdout = function(err, chunk)
      if cancelled or terminal then return end
      if err then finish(on_error, err); return end
      if not chunk then return end
      raw = (raw .. chunk):sub(-4096)
      pending = pending .. chunk
      local lines = vim.split(pending, "\n", { plain = true })
      pending = table.remove(lines) or ""
      for _, line in ipairs(lines) do line_received(line) end
    end,
  }, function(exit)
    if cancelled or terminal then return end
    if exit.code ~= 0 then
      local err = "cade.nvim: curl exited with code " .. exit.code
      if raw:find("Unauthorized") or raw:find("invalid API key") then
        err = "CADE server returned 401 Unauthorized. Check CADE_API_KEY on server and client."
      elseif raw ~= "" then
        err = err .. "\nServer response: " .. vim.trim(raw)
      elseif exit.stderr and exit.stderr ~= "" then
        err = err .. "\nError: " .. vim.trim(exit.stderr)
      end
      finish(on_error, err)
    else
      -- Process a final unterminated SSE line, but EOF alone is never success.
      if pending ~= "" then line_received(pending) end
      finish(on_error, "CADE edit stream ended without a completion event")
    end
  end)
  if ok then handle = result else finish(on_error, tostring(result)) end
  return cancel
end

local function get_visual_selection()
  local first, last = vim.fn.getpos("'<"), vim.fn.getpos("'>")
  local mode = vim.fn.visualmode()
  if mode == "\22" then error("CADE edits do not support blockwise selections") end
  local sr, er = first[2] - 1, last[2] - 1
  local start_line = vim.api.nvim_buf_get_lines(0, sr, sr + 1, true)[1] or ""
  local end_line = vim.api.nvim_buf_get_lines(0, er, er + 1, true)[1] or ""
  local sc, ec = math.min(math.max(first[3] - 1, 0), #start_line), math.min(last[3] - 1, #end_line)
  if mode == "V" then
    sc, ec = 0, #end_line
  elseif vim.o.selection ~= "exclusive" and ec < #end_line then
    -- Marks contain byte columns; include the entire final UTF-8 character.
    local char = vim.fn.strcharpart(end_line:sub(ec + 1), 0, 1)
    ec = ec + #char
  end
  local selected = table.concat(vim.api.nvim_buf_get_text(0, sr, sc, er, ec, {}), "\n")
  return selected, sr, sc, er, ec, mode
end

local function replace_text(buf, sr, sc, er, ec, new_text)
  vim.api.nvim_buf_set_text(buf, sr, sc, er, ec, vim.split(new_text, "\n", { plain = true }))
end

local hint_ns = vim.api.nvim_create_namespace("cade_edit_hint")
function M.update_visual_hint()
  local mode = vim.fn.mode()
  vim.api.nvim_buf_clear_namespace(0, hint_ns, 0, -1)
  if mode ~= "v" and mode ~= "V" and mode ~= "\22" then return end
  local row = math.max(vim.fn.getpos("v")[2], vim.fn.getpos(".")[2]) - 1
  local cfg = require("cade.config").get()
  local key = (cfg.keymaps and cfg.keymaps.edit) or "<leader>ce"
  local ok, err = pcall(vim.api.nvim_buf_set_extmark, 0, hint_ns, row, 0, {
    virt_text = { { " [" .. key .. ": ask cade]", "DiagnosticInfo" } },
    virt_text_pos = "eol", hl_mode = "combine",
  })
  if not ok then vim.notify("Hint error: " .. tostring(err), vim.log.levels.WARN) end
end

function M.setup_hints()
  vim.api.nvim_create_autocmd({ "CursorMoved", "ModeChanged" }, {
    group = vim.api.nvim_create_augroup("CadeEditHints", { clear = true }),
    pattern = "*", callback = M.update_visual_hint,
  })
end

function M.hover_edit()
  local cfg = require("cade.config").get()
  if cfg.edit and cfg.edit.enabled == false then
    vim.notify("CADE interactive edits are disabled", vim.log.levels.INFO)
    return
  end
  local mode = vim.fn.mode()
  if mode == "\22" then
    vim.notify("CADE edits do not support blockwise selections; use characterwise or linewise selection.", vim.log.levels.WARN)
    return
  end
  if mode ~= "n" and mode ~= "v" and mode ~= "V" then
    vim.notify("CADE edit requires normal or visual mode", vim.log.levels.WARN)
    return
  end
  local buf = vim.api.nvim_get_current_buf()
  if not vim.bo[buf].modifiable or vim.bo[buf].readonly then
    vim.notify("Target buffer is not writable", vim.log.levels.WARN)
    return
  end
  if mode ~= "n" then
    vim.api.nvim_feedkeys(vim.api.nvim_replace_termcodes("<Esc>", true, false, true), "nx", false)
  end
  -- Capture the source before scheduling/UI focus changes.
  local sr, sc, er, ec
  if mode == "n" then
    sr = vim.api.nvim_win_get_cursor(0)[1] - 1
    sc, er, ec = 0, sr, #vim.api.nvim_get_current_line()
  else
    local _, a, b, c, d = get_visual_selection()
    sr, sc, er, ec = a, b, c, d
  end
  local prefix_lines = vim.api.nvim_buf_get_lines(buf, math.max(0, sr - 50), sr, false)
  table.insert(prefix_lines, vim.api.nvim_buf_get_text(buf, sr, 0, sr, sc, {})[1])
  local end_line = vim.api.nvim_buf_get_lines(buf, er, er + 1, false)[1]
  local suffix_lines = vim.api.nvim_buf_get_lines(buf, er + 1, er + 21, false)
  table.insert(suffix_lines, 1, end_line:sub(ec + 1))
  local context = { prefix = table.concat(prefix_lines, "\n"), suffix = table.concat(suffix_lines, "\n"), language = vim.bo[buf].filetype }
  local prompt_buf = vim.api.nvim_create_buf(false, true)
  local review_buf = vim.api.nvim_create_buf(false, true)
  vim.bo[prompt_buf].bufhidden, vim.bo[review_buf].bufhidden = "wipe", "wipe"
  vim.bo[prompt_buf].filetype, vim.bo[review_buf].filetype = "markdown", "diff"
  local width = math.max(1, math.min(80, vim.o.columns - 4))
  local height = math.max(1, math.min(12, vim.o.lines - 9))
  local prompt_win = vim.api.nvim_open_win(prompt_buf, true, {
    relative = "editor", row = 1, col = 1, width = width, height = 3,
    style = "minimal", border = "rounded", title = " CADE instruction: Enter submits; Esc cancels ",
  })
  local review_win = vim.api.nvim_open_win(review_buf, false, {
    relative = "editor", row = 6, col = 1, width = width, height = height,
    style = "minimal", border = "rounded", title = " CADE review ",
  })
  local closed, session = false, nil
  local autocmds = {}
  local function close_all()
    if closed then return end
    closed = true
    for _, id in ipairs(autocmds) do pcall(vim.api.nvim_del_autocmd, id) end
    if session then session:cancel() end
    for _, win in ipairs({ prompt_win, review_win }) do
      if vim.api.nvim_win_is_valid(win) then pcall(vim.api.nvim_win_close, win, true) end
    end
    for _, scratch in ipairs({ prompt_buf, review_buf }) do
      if vim.api.nvim_buf_is_valid(scratch) then pcall(vim.api.nvim_buf_delete, scratch, { force = true }) end
    end
  end
  local function render(s)
    if closed then return end
    if s.state == "cancelled" or s.state == "applied" then close_all(); return end
    if not vim.api.nvim_buf_is_valid(review_buf) then close_all(); return end
    local lines = { "Enter: generate/retry | Ctrl-s: apply completed proposal | Esc: cancel" }
    if s.state == "failed" then
      vim.list_extend(lines, { "", "FAILED: " .. s.error:gsub("\n", " | "), "Instruction retained above. Edit it or press Enter to retry." })
    elseif s.state == "ready" then
      vim.list_extend(lines, { "", s.proposal == "" and "READY — empty replacement deletes selected text" or "READY — review before applying", "--- original", "+++ proposal" })
      local diff = vim.diff(s.original .. "\n", s.proposal .. "\n", { result_type = "unified" })
      vim.list_extend(lines, vim.split(diff == "" and "(No text changes)" or diff, "\n", { plain = true }))
    elseif s.state == "streaming" then
      vim.list_extend(lines, { "", "STREAMING — cannot apply", "" })
      vim.list_extend(lines, vim.split(s.proposal or "", "\n", { plain = true }))
    end
    vim.bo[review_buf].modifiable = true
    vim.api.nvim_buf_set_lines(review_buf, 0, -1, false, lines)
    vim.bo[review_buf].modifiable = false
  end
  session = sessions.new(buf, { sr, sc, er, ec }, render)
  render(session)
  for _, scratch in ipairs({ prompt_buf, review_buf }) do
    table.insert(autocmds, vim.api.nvim_create_autocmd({ "BufWipeout", "BufUnload" }, { buffer = scratch, callback = close_all }))
  end
  for _, win in ipairs({ prompt_win, review_win }) do
    table.insert(autocmds, vim.api.nvim_create_autocmd("WinClosed", { pattern = tostring(win), callback = close_all }))
  end
  local function submit_or_apply(apply)
    if closed then return end
    vim.cmd("stopinsert")
    if session.state == "streaming" then
      vim.notify("Wait for streaming to finish, or press Esc to cancel.", vim.log.levels.INFO)
      return
    end
    if apply and session.state == "ready" then
      local ok, err = session:apply()
      if not ok then vim.notify(err, vim.log.levels.WARN) end
    else
      local instruction = table.concat(vim.api.nvim_buf_get_lines(prompt_buf, 0, -1, false), "\n")
      session:start(instruction, M.fetch_edit, context)
    end
  end
  for _, scratch in ipairs({ prompt_buf, review_buf }) do
    vim.keymap.set({ "n", "i" }, "<Esc>", function() vim.cmd("stopinsert"); close_all() end, { buffer = scratch })
    vim.keymap.set({ "n", "i" }, "<C-s>", function() submit_or_apply(true) end, { buffer = scratch })
    vim.keymap.set({ "n", "i" }, "<CR>", function() submit_or_apply(false) end, { buffer = scratch })
  end
  vim.cmd("startinsert")
end

M._get_visual_selection = get_visual_selection
M._replace_text = replace_text
return M
