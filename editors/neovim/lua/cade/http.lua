-- cade/http.lua — Async curl SSE client for /v1/agents/:id/complete
-- Returns a cancel() function for aborting in-flight requests.

local M = {}

--- Telemetry: timestamps (os.clock()) set by the most recent fetch() call.
M._last_request_at  = nil  -- when fetch() fired
M._last_first_token = nil  -- when first stream_delta arrived
M._last_done_at     = nil  -- when stream ended (done or error)

--- Parse a single SSE line and return a typed result, or nil if not actionable.
--- This is a pure function — no I/O, no state. Exported for testing.
---@param line string  A raw SSE line (may include leading/trailing whitespace)
---@return table|nil  {type="delta",content=string} | {type="done"} | {type="error",message=string} | nil
function M._parse_sse_line(line)
  line = vim.trim(line)
  if line:sub(1, 6) ~= "data: " then return nil end
  local payload = line:sub(7)

  if payload == "[DONE]" then
    return { type = "done" }
  end

  local ok, obj = pcall(vim.json.decode, payload)
  if not ok or type(obj) ~= "table" then return nil end

  if obj.message_type == "stream_delta" and obj.content then
    return { type = "delta", content = obj.content }
  end

  if obj.message_type == "stream_end" then
    return { type = "done" }
  end

  if obj.error then
    return { type = "error", message = obj.error }
  end

  return nil
end

--- Stream a completion from the CADE server.
---@param prefix   string  Code before cursor
---@param suffix   string  Code after cursor
---@param language string  Filetype (e.g. "rust", "lua")
---@param on_token fun(accumulated: string)  Called on each new SSE delta
---@param on_done  fun()                     Called when stream ends normally
---@param on_error fun(msg: string)          Called on error (may be silent)
---@return fun()  cancel  Call to abort the in-flight curl process
function M.fetch(prefix, suffix, language, on_token, on_done, on_error)
  local cfg = require("cade.config").get()

  if cfg.agent_id == "" then
    on_error("cade.nvim: agent_id not configured")
    return function() end
  end

  -- Record request start time
  M._last_request_at  = os.clock()
  M._last_first_token = nil
  M._last_done_at     = nil

  local url = string.format(
    "http://127.0.0.1:%d/v1/agents/%s/complete",
    cfg.server_port,
    cfg.agent_id
  )

  local body = vim.json.encode({
    prefix     = prefix,
    suffix     = suffix,
    language   = language,
    max_tokens = cfg.max_tokens,
    model      = cfg.model ~= "" and cfg.model or vim.NIL,
  })

  local headers = {
    "-H", "Content-Type: application/json",
    "-H", "Accept: text/event-stream",
  }
  if cfg.api_key ~= "" then
    vim.list_extend(headers, { "-H", "Authorization: Bearer " .. cfg.api_key })
  end

  local cmd = vim.list_extend(
    { "curl", "--silent", "--no-buffer", "-N", "-X", "POST", "-d", body },
    headers
  )
  table.insert(cmd, url)

  local accumulated = ""
  local sse_buffer  = "" -- partial SSE line accumulator
  local raw_stdout = ""
  local done        = false

  local handle = vim.system(cmd, {
    text   = true,
    stdout = function(err, chunk)
      if done then return end

      if err then
        vim.schedule(function() on_error(err) end)
        return
      end
      if not chunk then return end -- stream closed

      raw_stdout = raw_stdout .. chunk
      sse_buffer = sse_buffer .. chunk

      -- Split on newlines; keep trailing partial line in sse_buffer
      local lines = vim.split(sse_buffer, "\n", { plain = true })
      sse_buffer = table.remove(lines) or ""

      for _, line in ipairs(lines) do
        local parsed = M._parse_sse_line(line)
        if parsed then
          if parsed.type == "done" then
            done = true
            M._last_done_at = os.clock()
            vim.schedule(on_done)
            return
          elseif parsed.type == "delta" then
            if M._last_first_token == nil then
              M._last_first_token = os.clock()
            end
            accumulated = accumulated .. parsed.content
            local snap = accumulated
            vim.schedule(function() on_token(snap) end)
          elseif parsed.type == "error" then
            done = true
            M._last_done_at = os.clock()
            vim.schedule(function() on_error(parsed.message) end)
            return
          end
        end
      end
    end,
  }, function(result)
    -- on_exit: curl process ended
    if not done then
      if result.code ~= 0 then
        vim.schedule(function()
          local err_msg = "cade.nvim: curl exited with code " .. result.code
          if raw_stdout ~= "" then
            local clean_body = vim.trim(raw_stdout)
            if clean_body:find("Unauthorized") or clean_body:find("invalid API key") then
              err_msg = "CADE server returned 401 Unauthorized. Please check that CADE_API_KEY is configured correctly on both server and client."
            else
              err_msg = err_msg .. "\nServer response: " .. clean_body
            end
          elseif result.stderr and result.stderr ~= "" then
            err_msg = err_msg .. "\nError: " .. vim.trim(result.stderr)
          end
          on_error(err_msg)
        end)
      else
        vim.schedule(on_done)
      end
    end
  end)

  -- Return cancel function
  return function()
    done = true
    pcall(function() handle:kill(9) end)
  end
end

return M
