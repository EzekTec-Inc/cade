-- spec/edit_spec.lua — TDD tests for CADE hover edit

local edit

local function make_buf(lines)
  local buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_set_current_buf(buf)
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines or { "" })
  return buf
end

describe("edit hover selection", function()
  local orig_vmode

  before_each(function()
    package.loaded["cade.edit"] = nil
    package.loaded["cade.config"] = nil
    require("cade.config").setup({})
    edit = require("cade.edit")
    orig_vmode = vim.fn.visualmode
  end)

  after_each(function()
    vim.fn.visualmode = orig_vmode
  end)

  it("get_visual_selection correctly sets s_col to 0 and end_col to line_len in V mode", function()
    local buf = make_buf({ "first line of text", "second line of text" })
    
    -- Set visual marks
    vim.api.nvim_buf_set_mark(buf, "<", 1, 5, {})
    vim.api.nvim_buf_set_mark(buf, ">", 2, 10, {})
    
    -- Mock visualmode to return "V"
    vim.fn.visualmode = function() return "V" end

    local selected_text, s_row, s_col, e_row, e_col, mode = edit._get_visual_selection()
    
    assert.are.equal("V", mode)
    assert.are.equal(0, start_row or s_row) -- 0-indexed first line
    assert.are.equal(0, start_col or s_col) -- forced to 0 in visual line mode
    assert.are.equal(1, end_row or e_row)   -- 0-indexed second line
    assert.are.equal(19, end_col or e_col)  -- length of "second line of text"
  end)

  it("get_visual_selection keeps columns as selected in characterwise v mode", function()
    local buf = make_buf({ "first line of text", "second line of text" })
    
    -- Set visual marks (inclusive)
    vim.api.nvim_buf_set_mark(buf, "<", 1, 5, {})
    vim.api.nvim_buf_set_mark(buf, ">", 2, 10, {})
    
    -- Mock visualmode to return "v"
    vim.fn.visualmode = function() return "v" end

    local selected_text, s_row, s_col, e_row, e_col, mode = edit._get_visual_selection()
    
    assert.are.equal("v", mode)
    assert.are.equal(0, s_row)
    assert.are.equal(5, s_col) -- 1-indexed column 6 is 5 0-indexed
    assert.are.equal(1, e_row)
    assert.are.equal(11, e_col) -- 1-indexed column 11 is 11 exclusive
  end)
end)
