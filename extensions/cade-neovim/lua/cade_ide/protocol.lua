-- lua/cade_ide/protocol.lua
-- Lua mirror of crates/cade-ide-mcp/src/protocol.rs
--
-- All messages are newline-delimited JSON (one object per line).
-- Discriminant field is "type" on top-level messages and "op" on CallbackOp.
--
-- Dependencies: vim.json  (Neovim ≥ 0.9 built-in)
-- In unit tests a json shim is injected via M._json.

local M = {}

-- Injectable JSON backend (replaced in tests).
M._json = vim and vim.json or {
  encode = function(v) error("no json encoder") end,
  decode = function(s) error("no json decoder") end,
}

-- ---------------------------------------------------------------------------
-- Encode an AdapterMessage → single JSON line (ends with "\n").
-- ---------------------------------------------------------------------------

--- @param msg table  AdapterMessage
--- @return string    newline-terminated JSON line
function M.encode(msg)
  return M._json.encode(msg) .. "\n"
end

-- ---------------------------------------------------------------------------
-- Decode one JSON line from the server → ServerMessage table.
-- Raises on malformed JSON or missing "type" field.
-- ---------------------------------------------------------------------------

--- @param line string  raw line (trailing newline is stripped)
--- @return table       ServerMessage
function M.decode_server(line)
  local trimmed = line:match("^%s*(.-)%s*$")
  if trimmed == "" then error("empty frame") end
  local ok, obj = pcall(M._json.decode, trimmed)
  if not ok then error("invalid JSON: " .. tostring(obj)) end
  if type(obj) ~= "table" then error("expected JSON object") end
  if type(obj.type) ~= "string" then error("ServerMessage missing 'type' field") end
  return obj
end

-- ---------------------------------------------------------------------------
-- Constructors for AdapterMessage variants
-- ---------------------------------------------------------------------------

--- Hello frame sent on connect.
--- @param label string   e.g. "neovim-0.11"
--- @return table
function M.hello(label)
  return { type = "hello", label = label, protocol_version = 1 }
end

--- StateUpdate frame.
--- @param snap table  StateSnapshot
--- @return table
function M.state_update(snap)
  local msg = { type = "state_update" }
  for k, v in pairs(snap) do msg[k] = v end
  return msg
end

--- CallbackResponse frame.
--- @param id     integer
--- @param result table   { ok = true } | { err = "..." }
--- @return table
function M.callback_response(id, result)
  return { type = "callback_response", id = id, result = result }
end

-- ---------------------------------------------------------------------------
-- Helpers for building StateSnapshot sub-objects
-- ---------------------------------------------------------------------------

--- @param line   integer  0-based
--- @param character integer 0-based
function M.position(line, character)
  return { line = line, character = character }
end

--- @param start_pos table  position
--- @param end_pos   table  position
function M.range(start_pos, end_pos)
  return { start = start_pos, ["end"] = end_pos }
end

return M
