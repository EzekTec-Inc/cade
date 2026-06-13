-- spec/ghost_spec.lua — TDD tests for cade/ghost.lua
--
-- Nine behaviours:
--   1. show() sets _pending and is_visible() returns true
--   2. show("") is a no-op — is_visible() stays false
--   3. show(nil) is a no-op — is_visible() stays false
--   4. clear() resets all state
--   5. accept() returns false when no pending text
--   6. accept() inserts full text into buffer, clears state
--   7. accept_line() inserts first line, keeps remainder in _pending (multi-line)
--   8. accept_line() on single-line text clears entirely
--   9. accept_word() with leading space: space included in word, remainder cleared

local ghost

local function make_buf(lines)
  local buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_set_current_buf(buf)
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines or { "" })
  vim.api.nvim_win_set_cursor(0, { 1, #(lines and lines[1] or "") })
  return buf
end

describe("ghost", function()
  before_each(function()
    package.loaded["cade.ghost"] = nil
    package.loaded["cade.config"] = nil
    require("cade.config").setup({})
    ghost = require("cade.ghost")
    make_buf({ "prefix " })
  end)

  after_each(function()
    ghost.clear()
  end)

  -- ── State ──────────────────────────────────────────────────────────────────

  it("show() sets _pending and is_visible() returns true", function()
    ghost.show("some completion")
    assert.is_true(ghost.is_visible())
    assert.are.equal("some completion", ghost._pending)
  end)

  it("show('') is a no-op — is_visible() stays false", function()
    ghost.show("")
    assert.is_false(ghost.is_visible())
    assert.is_nil(ghost._pending)
  end)

  it("show(nil) is a no-op — is_visible() stays false", function()
    ghost.show(nil)
    assert.is_false(ghost.is_visible())
    assert.is_nil(ghost._pending)
  end)

  it("clear() resets _pending, _buf, and _mark_ids", function()
    ghost.show("text")
    ghost.clear()
    assert.is_nil(ghost._pending)
    assert.is_nil(ghost._buf)
    assert.are.same({}, ghost._mark_ids)
    assert.is_false(ghost.is_visible())
  end)

  -- ── Accept guards ──────────────────────────────────────────────────────────

  it("accept() returns false when no pending text", function()
    ghost.clear()
    assert.is_false(ghost.accept())
  end)

  -- ── Full acceptance ────────────────────────────────────────────────────────

  it("accept() inserts full text into buffer and clears state", function()
    local buf = make_buf({ "hello " })
    vim.api.nvim_win_set_cursor(0, { 1, 6 })
    ghost.show("world")
    local ok = ghost.accept()

    assert.is_true(ok)
    assert.is_false(ghost.is_visible())
    assert.is_nil(ghost._pending)
    local content = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
    assert.are.equal("hello world", content[1])
  end)

  -- ── Partial acceptance (accept_line) ───────────────────────────────────────

  it("accept_line() inserts first line and keeps remainder in _pending", function()
    local inserted = {}
    local orig = vim.api.nvim_put
    vim.api.nvim_put = function(lines, ...) vim.list_extend(inserted, lines); orig(lines, ...) end

    ghost.show("line1\nline2\nline3")
    local ok = ghost.accept_line()

    vim.api.nvim_put = orig

    assert.is_true(ok)
    assert.are.equal("line1", inserted[1])
    assert.are.equal("line2\nline3", ghost._pending)
    assert.is_true(ghost.is_visible())
  end)

  it("accept_line() on single-line text clears state entirely", function()
    local inserted = {}
    local orig = vim.api.nvim_put
    vim.api.nvim_put = function(lines, ...) vim.list_extend(inserted, lines); orig(lines, ...) end

    ghost.show("only_line")
    local ok = ghost.accept_line()

    vim.api.nvim_put = orig

    assert.is_true(ok)
    assert.are.equal("only_line", inserted[1])
    assert.is_nil(ghost._pending)
    assert.is_false(ghost.is_visible())
  end)

  -- ── Partial acceptance (accept_word) ───────────────────────────────────────

  it("accept_word() with leading space includes space in consumed word", function()
    local inserted = {}
    local orig = vim.api.nvim_put
    vim.api.nvim_put = function(lines, ...) vim.list_extend(inserted, lines); orig(lines, ...) end

    ghost.show(" world")
    local ok = ghost.accept_word()

    vim.api.nvim_put = orig

    assert.is_true(ok)
    assert.are.equal(" world", inserted[1])
    assert.is_nil(ghost._pending)
    assert.is_false(ghost.is_visible())
  end)
end)
