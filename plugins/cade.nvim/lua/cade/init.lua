-- cade/init.lua — Public API for CADE inline completions

local M = {}

--- Setup CADE completions with user options.
---@param opts table|nil  See cade.config for available options.
function M.setup(opts)
  require("cade.config").setup(opts)
end

-- ── Completion state passthrough ─────────────────────────────────────────────

function M.accept()
  return require("cade.ghost").accept()
end

function M.accept_line()
  return require("cade.ghost").accept_line()
end

function M.accept_word()
  return require("cade.ghost").accept_word()
end

function M.dismiss()
  return require("cade.ghost").dismiss()
end

function M.is_visible()
  return require("cade.ghost").is_visible()
end

--- Toggle completions on/off.
function M.toggle()
  local cfg = require("cade.config")
  cfg.current.enabled = not cfg.current.enabled
  if not cfg.current.enabled then
    require("cade.ghost").clear()
  end
  vim.notify("CADE completions " .. (cfg.current.enabled and "enabled" or "disabled"))
end

-- ── Server probe (overridable for tests) ─────────────────────────────────────

--- Probe whether the CADE server is reachable. Returns true/false.
--- Override M._probe_server in tests to avoid real network calls.
---@return boolean
function M._probe_server()
  local cfg = require("cade.config").get()
  local url = string.format("http://127.0.0.1:%d/v1/agents", cfg.server_port)
  local result = vim.system(
    { "curl", "--silent", "--max-time", "1", "-o", "/dev/null", "-w", "%{http_code}", url },
    { text = true }
  ):wait()
  return result.code == 0 and vim.trim(result.stdout or "") ~= "000"
end

-- ── Status ───────────────────────────────────────────────────────────────────

--- Build and return a status summary string. Also calls vim.notify().
---@return string
function M.status()
  local cfg = require("cade.config").get()

  local reachable = M._probe_server()
  local server_icon = reachable and "✓" or "✗"
  local server_label = reachable and "reachable" or "unreachable"

  local agent_display = cfg.agent_id ~= "" and cfg.agent_id or "(not set)"
  local key_display   = (cfg.api_key ~= "" and cfg.api_key ~= nil) and "SET" or "NOT SET"

  -- Telemetry line
  local http = require("cade.http")
  local latency_str
  if http._last_request_at and http._last_done_at then
    local ttft  = http._last_first_token
      and math.floor((http._last_first_token - http._last_request_at) * 1000)
      or  nil
    local total = math.floor((http._last_done_at - http._last_request_at) * 1000)
    if ttft then
      latency_str = string.format("ttft=%dms total=%dms", ttft, total)
    else
      latency_str = string.format("total=%dms", total)
    end
  else
    latency_str = "(no data)"
  end

  local lines = {
    "CADE Completions",
    string.format("  Status:     %s", cfg.enabled and "enabled" or "disabled"),
    string.format("  Agent ID:   %s", agent_display),
    string.format("  Server:     http://127.0.0.1:%d  %s %s", cfg.server_port, server_icon, server_label),
    string.format("  API key:    %s", key_display),
    string.format("  Debounce:   %dms", cfg.debounce_ms),
    string.format("  Filetype:   %s", vim.bo.filetype ~= "" and vim.bo.filetype or "(any)"),
    string.format("  Latency:    %s", latency_str),
  }

  local text = table.concat(lines, "\n")
  vim.notify(text, vim.log.levels.INFO)
  return text
end

return M
