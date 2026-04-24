-- test/connection_test.lua
-- Tests for lua/cade_ide/connection.lua
-- Uses a real in-process TCP server via vim.loop.

local conn_mod = require("cade_ide.connection")
local proto    = require("cade_ide.protocol")

local function eq(a, b, msg)
  if a ~= b then
    error((msg or "eq failed") ..
      ("\n  expected: %s\n  got:      %s"):format(tostring(b), tostring(a)), 2)
  end
end

-- ── TCP test server ───────────────────────────────────────────────────────────

local function make_server()
  local server = vim.loop.new_tcp()
  server:bind("127.0.0.1", 0)
  local port = server:getsockname().port

  local pending_cb = nil
  server:listen(128, function()
    local client = vim.loop.new_tcp()
    server:accept(client)
    if pending_cb then
      local cb = pending_cb; pending_cb = nil; cb(client)
    end
  end)

  return {
    port   = port,
    -- Register handler for the next accepted client.
    on_client = function(self, cb) pending_cb = cb end,
    close     = function(self) pcall(function() server:close() end) end,
  }
end

-- Each test uses a unique temp directory so real cade-ide-mcp discovery
-- files never interfere.
local TEST_DISC_DIR = vim.fn.tempname()
vim.fn.mkdir(TEST_DISC_DIR, "p")

local DISC_PATTERN = "*.json"
local function write_discovery(port)
  -- Clean old files in this test dir.
  for _, f in ipairs(vim.fn.glob(TEST_DISC_DIR.."/"..DISC_PATTERN, false, true)) do
    os.remove(f)
  end
  local path = TEST_DISC_DIR .. "/disc.json"
  local f = assert(io.open(path, "w"))
  f:write(vim.json.encode({ pid = vim.fn.getpid(),
                             addr = "127.0.0.1:" .. port }))
  f:close()
  return path
end

local function rm(path) os.remove(path) end

-- Pump libuv + flush vim.schedule callbacks.
local function pump(ms)
  local stop = vim.loop.now() + ms
  vim.loop.update_time()
  while vim.loop.now() < stop do
    vim.loop.run("nowait")
    vim.loop.update_time()
  end
  -- Flush any pending vim.schedule callbacks.
  vim.wait(0, function() return false end)
end

-- Read one newline-terminated line from a TCP client (pumps until received).
local function read_line(client)
  local buf, done = "", false
  client:read_start(function(_, data)
    if data then buf = buf .. data end
    if buf:find("\n", 1, true) then client:read_stop(); done = true end
  end)
  local stop = vim.loop.now() + 2000
  vim.loop.update_time()
  while not done and vim.loop.now() < stop do
    vim.loop.run("nowait"); vim.loop.update_time()
  end
  return (buf:match("^(.-)\n") or ""):match("^%s*(.-)%s*$")
end

local T = {}

-- ── Hello frame ───────────────────────────────────────────────────────────────

function T.test_sends_hello_on_connect()
  local srv  = make_server()
  write_discovery(srv.port)
  local got_client = nil
  srv:on_client(function(c) got_client = c end)

  local conn = conn_mod.new({ log = function() end, discovery_dir = TEST_DISC_DIR })
  conn.connect()
  pump(400)

  assert(got_client, "server never accepted a client")
  local line = read_line(got_client)
  local msg  = proto.decode_server(line)
  eq(msg.type, "hello")
  eq(msg.protocol_version, 1)
  assert(type(msg.label) == "string" and #msg.label > 0)

  conn.dispose(); srv:close();
end

-- ── HelloAck logging ──────────────────────────────────────────────────────────

function T.test_logs_hello_ack()
  local srv  = make_server()
  write_discovery(srv.port)
  local got_client = nil
  srv:on_client(function(c) got_client = c end)

  local logs = {}
  local conn = conn_mod.new({ log = function(m) table.insert(logs, m) end, discovery_dir = TEST_DISC_DIR })
  conn.connect()
  pump(400)

  assert(got_client)
  read_line(got_client) -- consume Hello
  got_client:write(vim.json.encode({type="hello_ack",protocol_version=1}).."\n")
  pump(200)

  local found = false
  for _, l in ipairs(logs) do if l:find("HelloAck") then found = true end end
  assert(found, "HelloAck not logged: " .. vim.inspect(logs))

  conn.dispose(); srv:close();
end

-- ── send_state_update ─────────────────────────────────────────────────────────

function T.test_send_state_update()
  local srv  = make_server()
  write_discovery(srv.port)
  local got_client = nil
  srv:on_client(function(c) got_client = c end)

  local conn = conn_mod.new({ log = function() end, discovery_dir = TEST_DISC_DIR })
  conn.connect(); pump(400)
  read_line(got_client)
  got_client:write(vim.json.encode({type="hello_ack",protocol_version=1}).."\n")
  pump(150)

  conn.send_state_update({
    open_files={}, active_file="/tmp/a.lua", selection=vim.NIL,
    diagnostics={}, workspace_folders={}, visible_range=vim.NIL,
  })
  local line = read_line(got_client)
  local msg  = proto.decode_server(line)
  eq(msg.type, "state_update")
  eq(msg.active_file, "/tmp/a.lua")

  conn.dispose(); srv:close();
end

-- ── send_response ─────────────────────────────────────────────────────────────

function T.test_send_response()
  local srv  = make_server()
  write_discovery(srv.port)
  local got_client = nil
  srv:on_client(function(c) got_client = c end)

  local conn = conn_mod.new({ log = function() end, discovery_dir = TEST_DISC_DIR })
  conn.connect(); pump(400)
  read_line(got_client)
  got_client:write(vim.json.encode({type="hello_ack",protocol_version=1}).."\n")
  pump(150)

  conn.send_response(99, {ok=true})
  local line = read_line(got_client)
  local msg  = proto.decode_server(line)
  eq(msg.type, "callback_response"); eq(msg.id, 99); eq(msg.result.ok, true)

  conn.dispose(); srv:close();
end

-- ── callback_request dispatch ─────────────────────────────────────────────────

function T.test_dispatches_callback_request()
  local srv  = make_server()
  write_discovery(srv.port)
  local got_client = nil
  srv:on_client(function(c) got_client = c end)

  local received = {}
  local conn = conn_mod.new({ log = function() end, discovery_dir = TEST_DISC_DIR })
  conn.on_callback(function(id, op) table.insert(received, {id=id, op=op}) end)
  conn.connect(); pump(400)
  read_line(got_client)
  got_client:write(vim.json.encode({type="hello_ack",protocol_version=1}).."\n")
  pump(150)

  got_client:write(vim.json.encode({
    type="callback_request", id=55, op="run_task", name="make"
  }).."\n")
  pump(200)

  eq(#received, 1); eq(received[1].id, 55)
  eq(received[1].op.op, "run_task"); eq(received[1].op.name, "make")

  conn.dispose(); srv:close();
end

-- ── no discovery file ─────────────────────────────────────────────────────────

function T.test_no_discovery_file_logs_warning()
  for _, f in ipairs(vim.fn.glob(TEST_DISC_DIR.."/"..DISC_PATTERN,false,true)) do os.remove(f) end

  local logs = {}
  local conn = conn_mod.new({ log = function(m) table.insert(logs, m) end, discovery_dir = TEST_DISC_DIR })
  conn.connect(); pump(150)

  assert(#logs > 0, "expected a warning")
  conn.dispose()
end

-- ── dispose prevents connection ───────────────────────────────────────────────

function T.test_dispose_prevents_connect()
  local logs = {}
  local conn = conn_mod.new({ log = function(m) table.insert(logs, m) end, discovery_dir = TEST_DISC_DIR })
  conn.dispose()
  conn.connect(); pump(100)
  eq(#logs, 0)
end

return T

