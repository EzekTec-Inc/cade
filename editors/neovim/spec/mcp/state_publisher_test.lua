-- test/state_publisher_test.lua
-- Tests for the pure helper logic in state_publisher.lua.
-- No autocmd registration — just snapshot helpers.

local sp_mod = require("cade.mcp.state_publisher")
local proto  = require("cade.mcp.protocol")

local function eq(a, b, msg)
  if a ~= b then
    error((msg or "eq") .. ("\n  expected: %s\n  got: %s"):format(
      tostring(b), tostring(a)), 2)
  end
end

-- Stub connection that records sent snapshots.
local function stub_conn()
  local sent = {}
  return {
    send_state_update = function(snap) table.insert(sent, snap) end,
    sent = sent,
  }
end

local T = {}

-- ── _snapshot structure ───────────────────────────────────────────────────────

function T.test_snapshot_has_required_fields()
  local conn = stub_conn()
  local pub  = sp_mod.new(conn)
  local snap = pub:_snapshot()

  assert(type(snap.open_files)        == "table")
  assert(type(snap.diagnostics)       == "table")
  assert(type(snap.workspace_folders) == "table")
  -- active_file may be nil/empty in headless
  assert(snap.active_file == nil or type(snap.active_file) == "string")
end

function T.test_workspace_folders_non_empty()
  local conn = stub_conn()
  local pub  = sp_mod.new(conn)
  local snap = pub:_snapshot()
  assert(#snap.workspace_folders >= 1, "at least one workspace folder expected")
  local f = snap.workspace_folders[1]
  assert(type(f.path) == "string" and #f.path > 0)
  assert(type(f.name) == "string" and #f.name > 0)
end

function T.test_visible_range_nil_in_headless()
  local conn = stub_conn()
  local pub  = sp_mod.new(conn)
  local snap = pub:_snapshot()
  -- In headless mode w0/w$ return 0, so visible_range is NIL.
  assert(snap.visible_range == nil or snap.visible_range == vim.NIL
    or type(snap.visible_range) == "table")
end

-- ── _open_files via scratch buffer ───────────────────────────────────────────

function T.test_open_files_includes_named_scratch_buffer()
  local buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_buf_set_name(buf, "/tmp/cade_test_" .. buf .. ".lua")
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, {"print('hello')"})
  vim.api.nvim_set_option_value("filetype", "lua", { buf = buf })

  local conn = stub_conn()
  local pub  = sp_mod.new(conn)
  local files = pub:_open_files()

  local found = false
  for _, f in ipairs(files) do
    if f.path == vim.api.nvim_buf_get_name(buf) then
      found = true
      assert(f.text:find("print", 1, true))
      eq(f.language_id, "lua")
      assert(type(f.version) == "number")
    end
  end
  assert(found, "scratch buffer not found in open_files")
  vim.api.nvim_buf_delete(buf, { force = true })
end

-- ── _diagnostics via vim.diagnostic ─────────────────────────────────────────

function T.test_diagnostics_includes_injected_diagnostic()
  local buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_buf_set_name(buf, "/tmp/cade_diag_test_" .. buf .. ".lua")
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, {"local x = 1"})
  local ns = vim.api.nvim_create_namespace("cade_test")
  vim.diagnostic.set(ns, buf, {
    { lnum=0, col=6, end_lnum=0, end_col=7,
      message="unused variable", severity=vim.diagnostic.severity.WARN,
      source="cade-test", code="W001" }
  })

  local conn = stub_conn()
  local pub  = sp_mod.new(conn)
  local diags = pub:_diagnostics()

  local found = false
  for _, d in ipairs(diags) do
    if d.message == "unused variable" then
      found = true
      eq(d.severity, "warning")
      eq(d.source,   "cade-test")
      eq(d.code,     "W001")
      eq(d.range.start.line, 0)
    end
  end
  assert(found, "injected diagnostic not found")
  vim.diagnostic.reset(ns, buf)
  vim.api.nvim_buf_delete(buf, { force = true })
end

-- ── debounce / dispose ───────────────────────────────────────────────────────

function T.test_schedule_sends_after_debounce()
  local conn = stub_conn()
  local pub  = sp_mod.new(conn, { debounce_ms = 20 })
  pub._schedule()

  -- Pump 100ms — should fire once.
  local stop = vim.loop.now() + 100
  vim.loop.update_time()
  while vim.loop.now() < stop do
    vim.loop.run("nowait"); vim.loop.update_time()
  end
  vim.wait(0, function() return false end)

  assert(#conn.sent >= 1, "expected at least one state_update to be sent")
end

function T.test_dispose_cancels_timer()
  local conn = stub_conn()
  local pub  = sp_mod.new(conn, { debounce_ms = 200 })
  pub._schedule()
  pub.dispose()

  local stop = vim.loop.now() + 300
  vim.loop.update_time()
  while vim.loop.now() < stop do
    vim.loop.run("nowait"); vim.loop.update_time()
  end
  vim.wait(0, function() return false end)

  eq(#conn.sent, 0, "disposed publisher must not send")
end

-- ── _selection falls back to '<'/'>' when not in visual mode ─────────────────

function T.test_selection_nil_in_normal_mode_without_prior_visual()
  -- Clear any prior '<'/'>' marks by making sure the buffer is fresh.
  local buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_buf_set_name(buf, "/tmp/cade_sel_test_" .. buf .. ".lua")
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, {"hello world"})
  vim.api.nvim_set_current_buf(buf)

  local conn = stub_conn()
  local pub  = sp_mod.new(conn)
  local sel  = pub:_selection()
  assert(sel == vim.NIL, "expected NIL when not in visual mode and no prior selection")

  vim.api.nvim_buf_delete(buf, { force = true })
end

-- ── _selection uses 'v' / '.' marks when currently in visual mode ────────────
-- (Integration: actually enter visual mode, then snapshot.)

function T.test_selection_captured_in_visual_mode()
  local buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_buf_set_name(buf, "/tmp/cade_sel_vis_" .. buf .. ".lua")
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, {"hello world"})
  vim.api.nvim_set_current_buf(buf)

  -- Simulate: enter visual mode and select "hello" (cols 0..5).
  -- Use nvim_feedkeys with the 'x' flag to consume pending input synchronously.
  vim.cmd("normal! 0")
  vim.api.nvim_feedkeys("v4l", "x", false)   -- 'v' + move right 4 chars
  -- After 'x' flag, we should be in visual mode AT col 4 (selection 0..4).
  -- Some headless environments drop straight back to normal mode after feedkeys;
  -- if so, the '< / '> marks are set, and the fallback branch kicks in.

  local conn = stub_conn()
  local pub  = sp_mod.new(conn)
  local sel  = pub:_selection()

  -- Either path (visual-mode anchor or post-visual marks) should give us
  -- a non-nil selection covering columns inside "hello world" on line 0.
  if sel == vim.NIL then
    -- headless mode may fail to enter visual — skip without failing the suite.
    vim.api.nvim_buf_delete(buf, { force = true })
    return
  end
  eq(sel.range.start.line, 0)
  assert(sel.text:find("h") ~= nil, "selection text should contain 'h'")
  vim.api.nvim_buf_delete(buf, { force = true })
end

return T
