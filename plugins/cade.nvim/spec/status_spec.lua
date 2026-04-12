-- spec/status_spec.lua — TDD tests for :CadeStatus / require("cade").status()
--
-- Three behaviours:
--   1. status() returns a string containing "Agent", "Server", and "API key"
--   2. When server is reachable (probe returns code 0), output contains "✓"
--   3. When server is unreachable (probe returns non-0), output contains "✗"

describe("cade.status()", function()
  local config

  before_each(function()
    package.loaded["cade"] = nil
    package.loaded["cade.config"] = nil

    config = require("cade.config")
    config.setup({
      agent_id    = "agent-test-123",
      api_key     = "sk-test",
      server_port = 9999,
    })
  end)

  it("returns a string containing Agent, Server, and API key fields", function()
    local cade = require("cade")
    -- Stub the probe helper to avoid real network calls
    cade._probe_server = function() return false end

    local result = cade.status()

    assert.is_string(result)
    assert.truthy(result:find("Agent"), "expected 'Agent' in status output")
    assert.truthy(result:find("Server"), "expected 'Server' in status output")
    assert.truthy(result:find("API key"), "expected 'API key' in status output")
  end)

  it("shows ✓ when server is reachable", function()
    local cade = require("cade")
    cade._probe_server = function() return true end

    local result = cade.status()

    assert.truthy(result:find("✓"), "expected '✓' in status when server is reachable")
  end)

  it("shows ✗ when server is unreachable", function()
    local cade = require("cade")
    cade._probe_server = function() return false end

    local result = cade.status()

    assert.truthy(result:find("✗"), "expected '✗' in status when server is unreachable")
  end)
end)
