-- spec/config_spec.lua — TDD tests for agent_id resolution in config.lua
--
-- Three behaviours:
--   1. $CADE_AGENT_ID unset + settings.json exists → last_agent wins
--   2. $CADE_AGENT_ID set → env var wins (overrides file)
--   3. $CADE_AGENT_ID unset + settings.json missing → agent_id is ""

local config -- re-required per test to reset state

-- Helper: write a temporary settings.json and return its path
local function write_fixture(dir, content)
  local path = dir .. "/settings.json"
  local f = io.open(path, "w")
  f:write(content)
  f:close()
  return path
end

describe("config.agent_id resolution", function()
  local orig_env
  local tmpdir

  before_each(function()
    -- Isolate: save and clear CADE_AGENT_ID
    orig_env = vim.env.CADE_AGENT_ID
    vim.env.CADE_AGENT_ID = nil

    -- Create a temp directory for fixture files
    tmpdir = vim.fn.tempname()
    vim.fn.mkdir(tmpdir, "p")

    -- Force re-require so defaults are re-evaluated
    package.loaded["cade.config"] = nil
  end)

  after_each(function()
    -- Restore env
    if orig_env then
      vim.env.CADE_AGENT_ID = orig_env
    else
      vim.env.CADE_AGENT_ID = nil
    end
    -- Cleanup tmpdir
    if tmpdir then
      vim.fn.delete(tmpdir, "rf")
    end
  end)

  it("falls back to last_agent from settings.json when env var is unset", function()
    local fixture = write_fixture(tmpdir, vim.fn.json_encode({
      last_agent = "agent-from-settings-file",
    }))

    config = require("cade.config")
    -- Override the settings path for testing
    config.setup({ _settings_path = fixture })

    assert.are.equal("agent-from-settings-file", config.get().agent_id)
  end)

  it("prefers env var when CADE_AGENT_ID is set", function()
    write_fixture(tmpdir, vim.fn.json_encode({
      last_agent = "agent-from-settings-file",
    }))

    vim.env.CADE_AGENT_ID = "agent-from-env"

    config = require("cade.config")
    config.setup({})

    assert.are.equal("agent-from-env", config.get().agent_id)
  end)

  it("returns empty string when env var is unset and settings.json is missing", function()
    local missing_path = tmpdir .. "/nonexistent/settings.json"

    config = require("cade.config")
    config.setup({ _settings_path = missing_path })

    assert.are.equal("", config.get().agent_id)
  end)
end)

describe("config.keymaps", function()
  before_each(function()
    package.loaded["cade.config"] = nil
    vim.env.CADE_AGENT_ID = nil
  end)

  it("default keymaps table has all five expected keys", function()
    local config = require("cade.config")
    config.setup({})
    local km = config.get().keymaps
    assert.is_table(km)
    assert.is_string(km.accept)
    assert.is_string(km.accept_line)
    assert.is_string(km.accept_word)
    assert.is_string(km.dismiss)
    assert.is_string(km.toggle)
  end)

  it("partial override merges correctly, unspecified keys keep defaults", function()
    local config = require("cade.config")
    config.setup({ keymaps = { accept = "<C-y>" } })
    local km = config.get().keymaps
    assert.are.equal("<C-y>",       km.accept)
    assert.are.equal("<C-]>",       km.accept_line)
    assert.are.equal("<M-]>",       km.accept_word)
    assert.are.equal("<C-e>",       km.dismiss)
    assert.are.equal("<leader>ct",  km.toggle)
  end)

  it("setup({ keymaps = false }) sets keymaps to false", function()
    local config = require("cade.config")
    config.setup({ keymaps = false })
    assert.is_false(config.get().keymaps)
  end)
end)
