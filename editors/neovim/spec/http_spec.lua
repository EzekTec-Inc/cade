-- spec/http_spec.lua — TDD tests for cade/http.lua
--
-- Seven behaviours (Enhancement 2):
--   1. _parse_sse_line: stream_delta returns {type="delta", content=...}
--   2. _parse_sse_line: [DONE] returns {type="done"}
--   3. _parse_sse_line: error payload returns {type="error", message=...}
--   4. _parse_sse_line: empty or non-data line returns nil
--   5. _parse_sse_line: stream_start/stream_end (non-delta events) return nil
--   6. fetch(): calls on_error immediately when agent_id is ""
--   7. fetch(): returns a callable cancel function when agent_id is set

local http
local config

describe("http", function()
  before_each(function()
    package.loaded["cade.http"]   = nil
    package.loaded["cade.config"] = nil
    config = require("cade.config")
  end)

  -- ── _parse_sse_line ────────────────────────────────────────────────────────

  it("_parse_sse_line returns delta for stream_delta event", function()
    config.setup({ agent_id = "agent-x", api_key = "" })
    http = require("cade.http")

    local result = http._parse_sse_line(
      'data: {"message_type":"stream_delta","content":"fn main"}'
    )
    assert.is_not_nil(result)
    assert.are.equal("delta", result.type)
    assert.are.equal("fn main", result.content)
  end)

  it("_parse_sse_line returns done for [DONE]", function()
    config.setup({ agent_id = "agent-x", api_key = "" })
    http = require("cade.http")

    local result = http._parse_sse_line("data: [DONE]")
    assert.is_not_nil(result)
    assert.are.equal("done", result.type)
  end)

  it("_parse_sse_line returns error for error payload", function()
    config.setup({ agent_id = "agent-x", api_key = "" })
    http = require("cade.http")

    local result = http._parse_sse_line('data: {"error":"bad request"}')
    assert.is_not_nil(result)
    assert.are.equal("error", result.type)
    assert.are.equal("bad request", result.message)
  end)

  it("_parse_sse_line returns nil for empty and non-data lines", function()
    config.setup({ agent_id = "agent-x", api_key = "" })
    http = require("cade.http")

    assert.is_nil(http._parse_sse_line(""))
    assert.is_nil(http._parse_sse_line("event: ping"))
    assert.is_nil(http._parse_sse_line(": keep-alive"))
  end)

  it("_parse_sse_line returns nil for non-delta SSE event types", function()
    config.setup({ agent_id = "agent-x", api_key = "" })
    http = require("cade.http")

    local start = http._parse_sse_line(
      'data: {"message_type":"stream_start","model":"claude"}'
    )
    assert.is_nil(start)

    local stop = http._parse_sse_line(
      'data: {"message_type":"stream_end"}'
    )
    assert.is_nil(stop)
  end)

  -- ── fetch() ────────────────────────────────────────────────────────────────

  it("fetch() calls on_error immediately when agent_id is empty", function()
    config.setup({ agent_id = "", _settings_path = "/nonexistent/settings.json" })
    http = require("cade.http")

    local errors = {}
    local cancel = http.fetch("prefix", "suffix", "lua",
      function(_) end,
      function() end,
      function(msg) table.insert(errors, msg) end
    )

    assert.is_function(cancel)
    assert.are.equal(1, #errors)
    assert.truthy(errors[1]:find("agent_id"))
  end)

  it("fetch() returns a callable cancel function when agent_id is set", function()
    config.setup({ agent_id = "agent-test", api_key = "sk-test", server_port = 19998 })
    http = require("cade.http")

    local cancel = http.fetch("prefix", "suffix", "lua",
      function(_) end,
      function() end,
      function(_) end
    )

    assert.is_function(cancel)
    -- Cancel immediately — must not error
    local ok = pcall(cancel)
    assert.is_true(ok)
  end)

  -- ── Telemetry ──────────────────────────────────────────────────────────────

  it("fetch() sets _last_request_at to a number when called", function()
    config.setup({ agent_id = "agent-test", api_key = "sk-test", server_port = 19997 })
    http = require("cade.http")

    http._last_request_at = nil
    local cancel = http.fetch("prefix", "suffix", "lua",
      function(_) end,
      function() end,
      function(_) end
    )
    cancel()

    assert.is_number(http._last_request_at)
  end)
end)
