-- lua/cade_ide/callback_handler.lua
-- Receives a CallbackOp (from a ServerMessage.CallbackRequest) and
-- executes the corresponding Neovim operation.
-- Returns { ok = true } or { err = "message" }.

local M = {}

--- Dispatch a CallbackOp and return a CallbackResult.
--- @param op table  The decoded op object (has op.op field for the operation name)
--- @return table    { ok=true } | { err="..." }
function M.handle(op)
  local ok, err = pcall(M._dispatch, op)
  if ok then
    return { ok = true }
  else
    return { err = tostring(err) }
  end
end

function M._dispatch(op)
  local name = op.op or op.type
  if     name == "apply_edit"    then M._apply_edit(op)
  elseif name == "reveal_file"   then M._reveal_file(op)
  elseif name == "set_selection" then M._set_selection(op)
  elseif name == "save"          then M._save(op)
  elseif name == "run_task"      then M._run_task(op)
  elseif name == "run_terminal"  then M._run_terminal(op)
  elseif name == "debug_control" then M._debug_control(op)
  else   error("unknown op: " .. tostring(name))
  end
end

-- ── apply_edit ────────────────────────────────────────────────────────────────

function M._apply_edit(op)
  local buf = M._buf_for_path(op.path)
  -- Sort edits in reverse order so earlier offsets stay valid.
  local edits = vim.deepcopy(op.text_edits)
  table.sort(edits, function(a, b)
    if a.range.start.line ~= b.range.start.line then
      return a.range.start.line > b.range.start.line
    end
    return a.range.start.character > b.range.start.character
  end)
  for _, edit in ipairs(edits) do
    local r = edit.range
    -- nvim_buf_set_text: rows 0-based, cols 0-based, end_row/end_col exclusive
    local lines = vim.split(edit.new_text, "\n", { plain = true })
    vim.api.nvim_buf_set_text(
      buf,
      r.start.line, r.start.character,
      r["end"].line, r["end"].character,
      lines
    )
  end
end

-- ── reveal_file ───────────────────────────────────────────────────────────────

function M._reveal_file(op)
  vim.cmd("edit " .. vim.fn.fnameescape(op.path))
end

-- ── set_selection ─────────────────────────────────────────────────────────────

function M._set_selection(op)
  -- Open the file if not already current.
  local cur = vim.api.nvim_buf_get_name(0)
  if cur ~= op.path then
    vim.cmd("edit " .. vim.fn.fnameescape(op.path))
  end
  local r = op.range
  -- Move cursor to start, enter visual mode, move to end.
  vim.api.nvim_win_set_cursor(0, { r.start.line + 1, r.start.character })
  vim.cmd("normal! v")
  vim.api.nvim_win_set_cursor(0, { r["end"].line + 1,
    math.max(0, r["end"].character - 1) })
end

-- ── save ─────────────────────────────────────────────────────────────────────

function M._save(op)
  if op.path == nil or op.path == vim.NIL then
    vim.cmd("wa")
  else
    local buf = M._buf_for_path(op.path)
    vim.api.nvim_buf_call(buf, function() vim.cmd("w") end)
  end
end

-- ── run_task ─────────────────────────────────────────────────────────────────

function M._run_task(op)
  -- Delegate to :make or a user-configured task runner.
  -- Supports nvim-task / overseer / plain :make.
  local ok, overseer = pcall(require, "overseer")
  if ok then
    overseer.run_template({ name = op.name })
  else
    -- Fall back to :make with the task name as the target.
    vim.cmd("make " .. vim.fn.shellescape(op.name))
  end
end

-- ── run_terminal ─────────────────────────────────────────────────────────────

function M._run_terminal(op)
  -- Open a new terminal split and run the command.
  vim.cmd("split | terminal " .. op.command)
end

-- ── debug_control ─────────────────────────────────────────────────────────────

function M._debug_control(op)
  local ok, dap = pcall(require, "dap")
  if not ok then error("nvim-dap not installed") end
  if op.action == "start" then
    if op.config then
      dap.run(dap.configurations[op.config] or error("config not found: " .. op.config))
    else
      dap.continue()
    end
  elseif op.action == "stop" then
    dap.terminate()
  elseif op.action == "step_over" then
    dap.step_over()
  elseif op.action == "step_in" then
    dap.step_into()
  elseif op.action == "step_out" then
    dap.step_out()
  else
    error("unknown debug action: " .. tostring(op.action))
  end
end

-- ── helpers ───────────────────────────────────────────────────────────────────

--- Return the buffer number for the given path, loading it if needed.
function M._buf_for_path(path)
  for _, buf in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_get_name(buf) == path then
      return buf
    end
  end
  -- Not open — load it.
  local buf = vim.fn.bufadd(path)
  vim.fn.bufload(buf)
  return buf
end

return M
