-- test/callback_handler_test.lua
-- Tests for lua/cade_ide/callback_handler.lua
-- Uses real Neovim buffers; no LSP/DAP required for core ops.

local handler = require("cade_ide.callback_handler")

local function eq(a, b, msg)
  if a ~= b then
    error((msg or "eq") .. ("\n  expected: %s\n  got: %s"):format(
      tostring(b), tostring(a)), 2)
  end
end

local T = {}

-- ── apply_edit ────────────────────────────────────────────────────────────────

function T.test_apply_edit_inserts_text()
  local buf  = vim.api.nvim_create_buf(false, true)
  local path = "/tmp/cade_apply_" .. buf .. ".lua"
  vim.api.nvim_buf_set_name(buf, path)
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, { "hello world" })

  local result = handler.handle({
    op   = "apply_edit",
    path = path,
    text_edits = {
      { range = { start={line=0,character=0}, ["end"]={line=0,character=5} },
        new_text = "goodbye" }
    },
  })

  eq(result.ok, true)
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  eq(lines[1], "goodbye world")
  vim.api.nvim_buf_delete(buf, { force=true })
end

function T.test_apply_edit_multiple_edits_applied_in_reverse()
  local buf  = vim.api.nvim_create_buf(false, true)
  local path = "/tmp/cade_multi_" .. buf .. ".lua"
  vim.api.nvim_buf_set_name(buf, path)
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, { "foo bar baz" })

  local result = handler.handle({
    op   = "apply_edit",
    path = path,
    text_edits = {
      { range = { start={line=0,character=8}, ["end"]={line=0,character=11} }, new_text="qux" },
      { range = { start={line=0,character=0}, ["end"]={line=0,character=3} }, new_text="abc" },
    },
  })

  eq(result.ok, true)
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  eq(lines[1], "abc bar qux")
  vim.api.nvim_buf_delete(buf, { force=true })
end

-- ── reveal_file ───────────────────────────────────────────────────────────────

function T.test_reveal_file_opens_buffer()
  local tmp = vim.fn.tempname() .. ".lua"
  local f = io.open(tmp, "w"); f:write("-- reveal test\n"); f:close()

  local result = handler.handle({ op="reveal_file", path=tmp })
  eq(result.ok, true)

  local cur = vim.api.nvim_buf_get_name(0)
  eq(cur, tmp)
  os.remove(tmp)
end

-- ── save ─────────────────────────────────────────────────────────────────────

function T.test_save_single_buffer()
  local tmp = vim.fn.tempname() .. ".lua"
  -- Create and open the file.
  local buf = vim.fn.bufadd(tmp)
  vim.fn.bufload(buf)
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, { "-- saved content" })

  local result = handler.handle({ op="save", path=tmp })
  eq(result.ok, true)

  -- Verify file exists and has content.
  local f = io.open(tmp, "r")
  assert(f, "file was not written")
  local content = f:read("*a"); f:close()
  assert(content:find("saved content"), "unexpected content: " .. content)

  os.remove(tmp)
  vim.api.nvim_buf_delete(buf, { force=true })
end

-- ── run_terminal ─────────────────────────────────────────────────────────────

function T.test_run_terminal_opens_terminal_buffer()
  local before = #vim.api.nvim_list_bufs()
  local result = handler.handle({ op="run_terminal", command="echo cade-test" })
  eq(result.ok, true)
  local after = #vim.api.nvim_list_bufs()
  assert(after > before, "expected a new terminal buffer")
  -- Cleanup: close the terminal window
  pcall(vim.cmd, "bdelete!")
end

-- ── unknown op ───────────────────────────────────────────────────────────────

function T.test_unknown_op_returns_err()
  local result = handler.handle({ op="totally_unknown_op" })
  assert(result.err, "expected err result")
  assert(result.err:find("unknown"), "error should mention 'unknown': " .. result.err)
end

-- ── debug_control without dap ────────────────────────────────────────────────

function T.test_debug_control_without_dap_returns_err()
  -- nvim-dap is not installed in the test environment.
  local result = handler.handle({ op="debug_control", action="start" })
  -- Should return err (nvim-dap not installed), not panic.
  assert(result.err ~= nil, "expected err when dap not installed")
end

-- ── run_task fallback ────────────────────────────────────────────────────────

function T.test_run_task_fallback_does_not_panic()
  -- Without overseer, falls back to :make. In headless :make may error
  -- but handle() should catch it and return { err = ... }, not crash.
  local result = handler.handle({ op="run_task", name="nonexistent-target" })
  -- Either ok (make happened to succeed) or err — both are fine; just no panic.
  assert(result.ok ~= nil or result.err ~= nil)
end

return T
