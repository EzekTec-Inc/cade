describe("interactive edit windows", function()
  local edit, buf, original_fetch, original_mode, original_notify, original_visualmode, original_selection
  local requests, notifications, initial_windows
  local function map(bufnr, key)
    for _, mapping in ipairs(vim.api.nvim_buf_get_keymap(bufnr, "n")) do
      if mapping.lhs == key then return mapping.callback end
    end
    error("Missing mapping: " .. key)
  end
  local function open()
    edit.hover_edit()
    local prompt = vim.api.nvim_get_current_buf()
    vim.cmd("stopinsert")
    return prompt
  end
  local function review_buffer()
    for _, win in ipairs(vim.api.nvim_list_wins()) do
      if not initial_windows[win] then
        local candidate = vim.api.nvim_win_get_buf(win)
        if vim.bo[candidate].filetype == "diff" then return candidate, win end
      end
    end
    error("Review window missing")
  end
  before_each(function()
    require("cade.config").setup({ agent_id = "test" })
    edit = require("cade.edit")
    original_fetch, original_mode, original_notify = edit.fetch_edit, vim.fn.mode, vim.notify
    original_visualmode, original_selection = vim.fn.visualmode, vim.o.selection
    requests, notifications, initial_windows = {}, {}, {}
    for _, win in ipairs(vim.api.nvim_list_wins()) do initial_windows[win] = true end
    vim.fn.mode = function() return "n" end
    vim.notify = function(message) table.insert(notifications, message) end
    edit.fetch_edit = function(prefix, selected, suffix, instruction, language, token, done, failure)
      local request = { prefix = prefix, selected = selected, suffix = suffix, instruction = instruction,
        token = token, done = done, failure = failure, cancelled = false }
      table.insert(requests, request)
      return function() request.cancelled = true end
    end
    buf = vim.api.nvim_create_buf(false, true)
    vim.api.nvim_set_current_buf(buf)
    vim.api.nvim_buf_set_lines(buf, 0, -1, false, { "original", "after" })
    vim.api.nvim_win_set_cursor(0, { 1, 0 })
  end)
  after_each(function()
    vim.cmd("stopinsert")
    for _, win in ipairs(vim.api.nvim_list_wins()) do
      if not initial_windows[win] and vim.api.nvim_win_is_valid(win) then
        vim.api.nvim_win_close(win, true)
      end
    end
    edit.fetch_edit, vim.fn.mode, vim.notify = original_fetch, original_mode, original_notify
    vim.fn.visualmode, vim.o.selection = original_visualmode, original_selection
    if vim.api.nvim_buf_is_valid(buf) then vim.api.nvim_buf_delete(buf, { force = true }) end
  end)

  it("keeps instruction separate, retries multiline errors, reviews and applies", function()
    local prompt = open()
    local review = review_buffer()
    assert.is_false(vim.bo[review].modifiable)
    vim.api.nvim_buf_set_lines(prompt, 0, -1, false, { "rewrite", "---", "keep separator" })
    map(prompt, "<CR>")()
    assert.are.equal("rewrite\n---\nkeep separator", requests[1].instruction)
    assert.are.equal("", requests[1].prefix)
    assert.are.equal("\nafter", requests[1].suffix)
    requests[1].token("partial")
    requests[1].failure("failed\nwith details")
    assert.is_truthy(table.concat(vim.api.nvim_buf_get_lines(review, 0, -1, false), "\n"):find("failed | with details", 1, true))
    map(prompt, "<CR>")()
    assert.are.equal(requests[1].instruction, requests[2].instruction)
    requests[1].done()
    requests[2].token("replacement")
    requests[2].done()
    local diff = table.concat(vim.api.nvim_buf_get_lines(review, 0, -1, false), "\n")
    assert.is_truthy(diff:find("-original", 1, true))
    assert.is_truthy(diff:find("+replacement", 1, true))
    assert.are.same({ "original", "after" }, vim.api.nvim_buf_get_lines(buf, 0, -1, false))
    map(prompt, "<C-S>")()
    assert.are.same({ "replacement", "after" }, vim.api.nvim_buf_get_lines(buf, 0, -1, false))
    assert.is_false(vim.api.nvim_buf_is_valid(prompt))
    assert.is_false(vim.api.nvim_buf_is_valid(review))
  end)

  it("review window close cancels transport and closes both scratch buffers", function()
    local prompt = open()
    local review, win = review_buffer()
    vim.api.nvim_buf_set_lines(prompt, 0, -1, false, { "rewrite" })
    map(prompt, "<CR>")()
    vim.api.nvim_win_close(win, true)
    assert.is_true(requests[1].cancelled)
    requests[1].token("late")
    requests[1].done()
    assert.is_false(vim.api.nvim_buf_is_valid(prompt))
    assert.is_false(vim.api.nvim_buf_is_valid(review))
    assert.are.same({ "original", "after" }, vim.api.nvim_buf_get_lines(buf, 0, -1, false))
  end)

  it("target destruction closes the UI and cancels transport", function()
    local prompt = open()
    local review = review_buffer()
    vim.api.nvim_buf_set_lines(prompt, 0, -1, false, { "rewrite" })
    map(prompt, "<CR>")()
    vim.api.nvim_buf_delete(buf, { force = true })
    requests[1].done()
    assert.is_true(requests[1].cancelled)
    assert.is_false(vim.api.nvim_buf_is_valid(prompt))
    assert.is_false(vim.api.nvim_buf_is_valid(review))
  end)

  it("rejects blockwise selection before opening windows", function()
    vim.fn.mode = function() return "\22" end
    edit.hover_edit()
    assert.are.equal(buf, vim.api.nvim_get_current_buf())
    assert.are.equal(0, #requests)
    assert.is_truthy(notifications[1]:find("blockwise"))
    for _, win in ipairs(vim.api.nvim_list_wins()) do assert.is_true(initial_windows[win]) end
  end)

  it("selects complete UTF-8 characters and respects exclusive selection", function()
    vim.api.nvim_buf_set_lines(buf, 0, -1, false, { "aéz" })
    vim.api.nvim_buf_set_mark(buf, "<", 1, 0, {})
    vim.api.nvim_buf_set_mark(buf, ">", 1, 1, {})
    vim.fn.visualmode = function() return "v" end
    vim.o.selection = "inclusive"
    assert.are.equal("aé", edit._get_visual_selection())
    vim.o.selection = "exclusive"
    assert.are.equal("a", edit._get_visual_selection())
  end)
end)
