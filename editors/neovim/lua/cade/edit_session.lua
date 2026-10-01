-- Internal edit lifecycle. The UI never owns target coordinates or readiness.
local M = {}
local Session = {}
Session.__index = Session
local ns = vim.api.nvim_create_namespace("cade_edit_session")

local function text(buf, range)
  return table.concat(vim.api.nvim_buf_get_text(buf, range[1], range[2], range[3], range[4], {}), "\n")
end

function M.new(buf, range, on_change)
  local self = setmetatable({
    buf = buf,
    original = text(buf, range),
    state = "editing",
    proposal = nil,
    instruction = "",
    attempt = 0,
    on_change = on_change or function() end,
  }, Session)
  self.mark = vim.api.nvim_buf_set_extmark(buf, ns, range[1], range[2], {
    end_row = range[3], end_col = range[4], hl_group = "Visual",
    right_gravity = false, end_right_gravity = true,
    invalidate = true, undo_restore = false,
  })
  self.autocmd = vim.api.nvim_create_autocmd({ "BufUnload", "BufWipeout" }, {
    buffer = buf, once = true, callback = function() self:cancel() end,
  })
  return self
end

function Session:release()
  self.attempt = self.attempt + 1
  local cancel = self.cancel_request
  self.cancel_request = nil
  if cancel then pcall(cancel) end
  if self.autocmd then
    pcall(vim.api.nvim_del_autocmd, self.autocmd)
    self.autocmd = nil
  end
  if self.mark then
    pcall(vim.api.nvim_buf_del_extmark, self.buf, ns, self.mark)
    self.mark = nil
  end
end

function Session:cancel()
  if self.state == "cancelled" or self.state == "applied" then return end
  self.state = "cancelled"
  self:release()
  self.on_change(self)
end

function Session:target()
  if not vim.api.nvim_buf_is_valid(self.buf) or not vim.api.nvim_buf_is_loaded(self.buf) then
    return nil, "Target buffer is no longer available"
  end
  if not vim.bo[self.buf].modifiable or vim.bo[self.buf].readonly then
    return nil, "Target buffer is not writable"
  end
  local pos = self.mark and vim.api.nvim_buf_get_extmark_by_id(self.buf, ns, self.mark, { details = true }) or {}
  if #pos == 0 or pos[3].invalid or pos[3].end_row == nil then
    return nil, "Target range is no longer available"
  end
  local range = { pos[1], pos[2], pos[3].end_row, pos[3].end_col }
  local ok, current = pcall(text, self.buf, range)
  if not ok or current ~= self.original then
    return nil, "Target text changed; cancel and select it again"
  end
  return range
end

-- fetch is the owned edit transport (or a controlled test adapter).
function Session:start(instruction, fetch, context)
  if self.state == "cancelled" or self.state == "applied" then return false end
  instruction = vim.trim(instruction)
  if instruction == "" then return false end
  self.attempt = self.attempt + 1
  local attempt = self.attempt
  local cancel = self.cancel_request
  self.cancel_request = nil
  if cancel then pcall(cancel) end
  self.instruction, self.proposal, self.error = instruction, "", nil
  self.state = "streaming"
  local function active()
    return self.attempt == attempt and self.state == "streaming"
  end
  local function fail(err)
    if not active() then return end
    self.state, self.error, self.proposal = "failed", tostring(err), nil
    self.cancel_request = nil
    self.on_change(self)
  end
  local range, err = self:target()
  if not range then fail(err); return false end
  self.on_change(self)
  -- An observer may have closed the UI synchronously.
  if not active() then return false end
  local ok, handle = pcall(fetch, context.prefix, self.original, context.suffix, instruction, context.language,
    function(snap)
      if not active() then return end
      self.proposal = snap
      self.on_change(self)
    end,
    function()
      if not active() then return end
      self.state, self.cancel_request = "ready", nil
      self.on_change(self)
    end,
    fail)
  if not ok then
    fail(handle)
  elseif active() then
    self.cancel_request = handle
  elseif self.attempt ~= attempt or self.state == "cancelled" then
    if type(handle) == "function" then pcall(handle) end
  end
  return ok
end

function Session:apply()
  if self.state ~= "ready" then return false, "No completed proposal to apply" end
  local range, err = self:target()
  if not range then return false, err end
  local ok, failure = pcall(vim.api.nvim_buf_set_text, self.buf, range[1], range[2], range[3], range[4],
    vim.split(self.proposal, "\n", { plain = true }))
  if not ok then return false, tostring(failure) end
  self.state = "applied"
  self:release()
  self.on_change(self)
  return true
end

return M
