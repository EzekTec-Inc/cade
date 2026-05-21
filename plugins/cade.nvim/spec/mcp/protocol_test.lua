-- test/protocol_test.lua
-- Tests for lua/cade_ide/protocol.lua
-- Runs headless: nvim --headless --noplugin -u test/runner.lua

local proto = require("cade.mcp.protocol")

-- Inject a minimal JSON shim so tests don't depend on vim.json quirks.
-- vim.json IS available in headless Neovim, so this just makes intent explicit.
proto._json = vim.json

local function eq(a, b, msg)
  if a ~= b then
    error((msg or "assertion failed") .. ("\n  expected: %s\n  got:      %s"):format(
      tostring(b), tostring(a)), 2)
  end
end

local function has(s, substr, msg)
  if not s:find(substr, 1, true) then
    error((msg or "substring not found") .. ": " .. substr .. "\n  in: " .. s, 2)
  end
end

local T = {}

-- ── encode ──────────────────────────────────────────────────────────────────

function T.test_encode_ends_with_newline()
  local line = proto.encode(proto.hello("neovim-test"))
  eq(line:sub(-1), "\n", "encoded line must end with newline")
end

function T.test_encode_no_embedded_newlines()
  local line = proto.encode(proto.hello("neovim-test"))
  local body = line:sub(1, -2)   -- strip trailing \n
  eq(body:find("\n"), nil, "no embedded newlines")
end

function T.test_hello_type_tag()
  local line = proto.encode(proto.hello("nv"))
  has(line, '"type"', "has type key")
  has(line, '"hello"', "has hello value")
  has(line, '"nv"', "has label")
  has(line, '"protocol_version"', "has protocol_version")
end

-- ── round-trip AdapterMessage variants ──────────────────────────────────────

function T.test_hello_round_trips()
  local msg = proto.hello("neovim-0.11")
  local decoded = vim.json.decode(proto.encode(msg):match("(.-)%s*$"))
  eq(decoded.type, "hello")
  eq(decoded.label, "neovim-0.11")
  eq(decoded.protocol_version, 1)
end

function T.test_state_update_empty_round_trips()
  local snap = {
    open_files = {}, active_file = vim.NIL, selection = vim.NIL,
    diagnostics = {}, workspace_folders = {}, visible_range = vim.NIL,
  }
  local msg = proto.state_update(snap)
  local decoded = vim.json.decode(proto.encode(msg):match("(.-)%s*$"))
  eq(decoded.type, "state_update")
  eq(#decoded.open_files, 0)
end

function T.test_state_update_full_round_trips()
  local snap = {
    open_files = { { path = "/tmp/a.lua", text = "print(1)", language_id = "lua",
                     version = 2, is_dirty = true } },
    active_file = "/tmp/a.lua",
    selection = {
      path = "/tmp/a.lua",
      range = proto.range(proto.position(0,0), proto.position(0,5)),
      text = "print",
    },
    diagnostics = { {
      path = "/tmp/a.lua",
      range = proto.range(proto.position(0,0), proto.position(0,5)),
      severity = "warning", message = "unused", source = "lua", code = "W001",
    } },
    workspace_folders = { { path = "/tmp", name = "tmp" } },
    visible_range = { 0, 40 },
  }
  local decoded = vim.json.decode(proto.encode(proto.state_update(snap)):match("(.-)%s*$"))
  eq(decoded.type, "state_update")
  eq(decoded.active_file, "/tmp/a.lua")
  eq(#decoded.open_files, 1)
  eq(decoded.open_files[1].path, "/tmp/a.lua")
  eq(decoded.open_files[1].is_dirty, true)
  eq(#decoded.diagnostics, 1)
  eq(decoded.diagnostics[1].severity, "warning")
end

function T.test_callback_response_ok_round_trips()
  local msg = proto.callback_response(42, { ok = true })
  local decoded = vim.json.decode(proto.encode(msg):match("(.-)%s*$"))
  eq(decoded.type, "callback_response")
  eq(decoded.id, 42)
  eq(decoded.result.ok, true)
end

function T.test_callback_response_err_round_trips()
  local msg = proto.callback_response(7, { err = "file not open" })
  local decoded = vim.json.decode(proto.encode(msg):match("(.-)%s*$"))
  eq(decoded.type, "callback_response")
  eq(decoded.id, 7)
  eq(decoded.result.err, "file not open")
end

-- ── decode_server ────────────────────────────────────────────────────────────

function T.test_decode_server_hello_ack()
  local line = '{"type":"hello_ack","protocol_version":1}'
  local msg = proto.decode_server(line)
  eq(msg.type, "hello_ack")
  eq(msg.protocol_version, 1)
end

function T.test_decode_server_callback_request_apply_edit()
  local line = vim.json.encode({
    type = "callback_request", id = 1,
    op = "apply_edit", path = "/tmp/a.lua",
    text_edits = { { range = { start={line=0,character=0}, ["end"]={line=0,character=0} },
                     new_text = "-- hi\n" } },
  })
  local msg = proto.decode_server(line)
  eq(msg.type, "callback_request")
  eq(msg.id, 1)
  eq(msg.op, "apply_edit")
  eq(msg.path, "/tmp/a.lua")
  eq(#msg.text_edits, 1)
end

function T.test_decode_server_callback_request_reveal_file()
  local msg = proto.decode_server(vim.json.encode({
    type="callback_request", id=2, op="reveal_file", path="/tmp/b.lua"
  }))
  eq(msg.op, "reveal_file")
  eq(msg.path, "/tmp/b.lua")
end

function T.test_decode_server_callback_request_set_selection()
  local msg = proto.decode_server(vim.json.encode({
    type="callback_request", id=3, op="set_selection", path="/tmp/c.lua",
    range={ start={line=5,character=2}, ["end"]={line=5,character=10} }
  }))
  eq(msg.op, "set_selection")
  eq(msg.range.start.line, 5)
end

function T.test_decode_server_callback_request_save_single()
  local msg = proto.decode_server(vim.json.encode({
    type="callback_request", id=4, op="save", path="/tmp/d.lua"
  }))
  eq(msg.op, "save"); eq(msg.path, "/tmp/d.lua")
end

function T.test_decode_server_callback_request_save_all()
  local msg = proto.decode_server(vim.json.encode({
    type="callback_request", id=5, op="save", path=vim.NIL
  }))
  eq(msg.op, "save")
  eq(msg.path, vim.NIL)
end

function T.test_decode_server_callback_request_run_task()
  local msg = proto.decode_server(vim.json.encode({
    type="callback_request", id=6, op="run_task", name="make"
  }))
  eq(msg.op, "run_task"); eq(msg.name, "make")
end

function T.test_decode_server_callback_request_run_terminal()
  local msg = proto.decode_server(vim.json.encode({
    type="callback_request", id=7, op="run_terminal", command="cargo test"
  }))
  eq(msg.op, "run_terminal"); eq(msg.command, "cargo test")
end

function T.test_decode_server_callback_request_debug_start()
  local msg = proto.decode_server(vim.json.encode({
    type="callback_request", id=8, op="debug_control",
    action="start", config="unit-tests"
  }))
  eq(msg.op, "debug_control"); eq(msg.action, "start"); eq(msg.config, "unit-tests")
end

function T.test_decode_server_callback_request_debug_stop()
  local msg = proto.decode_server(vim.json.encode({
    type="callback_request", id=9, op="debug_control", action="stop"
  }))
  eq(msg.action, "stop")
end

function T.test_decode_server_throws_on_missing_type()
  local ok, err = pcall(proto.decode_server, '{"id":1}')
  eq(ok, false)
  has(tostring(err), "type", "error mentions 'type'")
end

function T.test_decode_server_throws_on_invalid_json()
  local ok, _ = pcall(proto.decode_server, "not-json")
  eq(ok, false)
end

function T.test_decode_server_throws_on_empty_line()
  local ok, _ = pcall(proto.decode_server, "   ")
  eq(ok, false)
end

-- ── position / range helpers ─────────────────────────────────────────────────

function T.test_position_constructor()
  local p = proto.position(3, 7)
  eq(p.line, 3); eq(p.character, 7)
end

function T.test_range_constructor()
  local r = proto.range(proto.position(0,0), proto.position(1,5))
  eq(r.start.line, 0); eq(r["end"].character, 5)
end

return T
