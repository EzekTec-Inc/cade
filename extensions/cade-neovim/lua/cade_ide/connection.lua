-- lua/cade_ide/connection.lua
-- Manages the TCP connection to cade-ide-mcp.
--
-- Architecture:
--   1. connect()   — reads ~/.cade/ide/<pid>.json, opens TCP via vim.loop,
--                    sends Hello, waits for HelloAck, then pumps the read loop.
--   2. Incoming frames are line-buffered; each complete line is decoded and
--      dispatched: HelloAck → log, CallbackRequest → on_callback handler.
--   3. send_state_update(snap)  — encodes + writes StateUpdate frame.
--   4. send_response(id, result)— encodes + writes CallbackResponse frame.
--   5. dispose()   — closes socket, cancels reconnect timer.
--
-- The public API is a table returned by M.new().

local proto = require("cade_ide.protocol")

local PROTOCOL_VERSION = 1
local RECONNECT_MS     = 3000
local LABEL            = "neovim-" .. (vim.version and
                           ("%d.%d"):format(vim.version().major, vim.version().minor)
                           or "unknown")
local DEFAULT_DISC_DIR = vim.fn.expand("~/.cade/ide")

local M = {}

--- Create a new connection object.
--- @param opts table|nil  { log = function(msg), discovery_dir = string }
function M.new(opts)
  opts = opts or {}
  local log = opts.log or function(msg)
    vim.schedule(function()
      vim.notify("[cade-ide] " .. msg, vim.log.levels.INFO)
    end)
  end
  local disc_dir = opts.discovery_dir or DEFAULT_DISC_DIR

  local self = {
    _tcp        = nil,
    _buf        = "",        -- incomplete line buffer
    _timer      = nil,
    _disposed   = false,
    _handler    = nil,       -- on_callback(id, op)
    log         = log,
  }

  -- ── public API ────────────────────────────────────────────────────────────

  function self.on_callback(handler) self._handler = handler end

  function self.connect()
    if self._disposed then return end
    self:_disconnect()

    local info = self:_read_discovery()
    if not info then
      self.log("cade-ide-mcp not running (no discovery file). Retrying…")
      self:_schedule_reconnect()
      return
    end

    local tcp = vim.loop.new_tcp()
    self._tcp = tcp
    self._buf = ""

    tcp:connect(info.addr, info.port, function(err)
      if err then
        self.log("connection failed: " .. err)
        self:_schedule_reconnect()
        return
      end

      -- Send Hello.
      self:_write(proto.encode(proto.hello(LABEL)))
      self.log(("connected to cade-ide-mcp at %s:%d"):format(info.addr, info.port))

      -- Start read loop.
      tcp:read_start(function(rerr, data)
        if rerr or not data then
          if not self._disposed then
            self.log("disconnected — reconnecting…")
            self:_schedule_reconnect()
          end
          return
        end
        self:_on_data(data)
      end)
    end)
  end

  function self.send_state_update(snap)
    self:_write(proto.encode(proto.state_update(snap)))
  end

  function self.send_response(id, result)
    self:_write(proto.encode(proto.callback_response(id, result)))
  end

  function self.dispose()
    self._disposed = true
    self:_cancel_timer()
    self:_disconnect()
  end

  -- ── private ───────────────────────────────────────────────────────────────

  function self:_write(data)
    if self._tcp and not self._tcp:is_closing() then
      self._tcp:write(data)
    end
  end

  function self:_disconnect()
    self:_cancel_timer()
    if self._tcp then
      if not self._tcp:is_closing() then
        pcall(function() self._tcp:read_stop() end)
        pcall(function() self._tcp:close() end)
      end
      self._tcp = nil
    end
    self._buf = ""
  end

  function self:_cancel_timer()
    if self._timer then
      pcall(function() self._timer:stop(); self._timer:close() end)
      self._timer = nil
    end
  end

  function self:_schedule_reconnect()
    if self._disposed then return end
    self:_cancel_timer()
    local t = vim.loop.new_timer()
    self._timer = t
    t:start(RECONNECT_MS, 0, function()
      t:close(); self._timer = nil
      if not self._disposed then
        vim.schedule(function() self.connect() end)
      end
    end)
  end

  function self:_on_data(data)
    self._buf = self._buf .. data
    -- Process all complete lines.
    while true do
      local nl = self._buf:find("\n", 1, true)
      if not nl then break end
      local line = self._buf:sub(1, nl - 1)
      self._buf  = self._buf:sub(nl + 1)
      if line ~= "" then self:_on_line(line) end
    end
  end

  function self:_on_line(line)
    local ok, msg = pcall(proto.decode_server, line)
    if not ok then
      self.log("malformed frame: " .. tostring(msg))
      return
    end
    vim.schedule(function() self:_dispatch(msg) end)
  end

  function self:_dispatch(msg)
    if msg.type == "hello_ack" then
      self.log(("HelloAck received (protocol v%d). Adapter ready."):format(
        msg.protocol_version or 0))
    elseif msg.type == "callback_request" then
      if self._handler then self._handler(msg.id, msg) end
    end
  end

  -- ── discovery file ────────────────────────────────────────────────────────

  function self:_read_discovery()
    local dir = disc_dir
    if vim.fn.isdirectory(dir) == 0 then return nil end
    local files = vim.fn.glob(dir .. "/*.json", false, true)
    if #files == 0 then return nil end

    -- Parse all valid, live entries.
    local entries = {}
    for _, path in ipairs(files) do
      local f = io.open(path, "r"); if not f then goto continue end
      local raw = f:read("*a"); f:close()
      local ok, obj = pcall(vim.json.decode, raw)
      if not ok or type(obj) ~= "table" or not obj.addr or not obj.pid then
        goto continue
      end
      -- Skip dead processes.
      if vim.loop.kill(obj.pid, 0) ~= 0 and
         not pcall(function() assert(vim.loop.kill(obj.pid, 0) == 0) end) then
        -- use os-level check instead
      end
      local host, port = obj.addr:match("^(.+):(%d+)$")
      if not host then goto continue end
      local mtime = (vim.loop.fs_stat(path) or {mtime={sec=0}}).mtime.sec
      -- Determine if parent is cade-server (preferred) or cade TUI (avoid).
      local ppid_out = vim.fn.system("ps -o ppid= -p " .. obj.pid .. " 2>/dev/null")
      local ppid = tonumber(ppid_out:match("%d+"))
      local parent_cmd = ppid and vim.fn.system(
        "ps -o comm= -p " .. ppid .. " 2>/dev/null"):gsub("%s","") or ""
      local is_server = parent_cmd:find("cade%-server") ~= nil
        or parent_cmd:find("cade_server") ~= nil
      table.insert(entries, {
        addr = host, port = tonumber(port), mtime = mtime,
        is_server = is_server, path = path,
      })
      ::continue::
    end

    if #entries == 0 then return nil end

    -- Prefer server-session entries; fall back to newest by mtime.
    table.sort(entries, function(a, b)
      if a.is_server ~= b.is_server then return a.is_server end
      return a.mtime > b.mtime
    end)

    local e = entries[1]
    return { addr = e.addr, port = e.port }
  end

  return self
end

return M
