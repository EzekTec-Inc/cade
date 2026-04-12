-- cade/http.lua — Async curl SSE client for /v1/agents/:id/complete
-- Returns a cancel() function for aborting in-flight requests.

local M = {}

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

      sse_buffer = sse_buffer .. chunk

      -- Split on newlines; keep trailing partial line in sse_buffer
      local lines = vim.split(sse_buffer, "\n", { plain = true })
      sse_buffer = table.remove(lines) or ""

      for _, line in ipairs(lines) do
        line = vim.trim(line)
        if line:sub(1, 6) == "data: " then
          local payload = line:sub(7)

          if payload == "[DONE]" then
            done = true
            vim.schedule(on_done)
            return
          end

          local ok, obj = pcall(vim.json.decode, payload)
          if ok and obj then
            if obj.message_type == "stream_delta" and obj.content then
              accumulated = accumulated .. obj.content
              local snap = accumulated
              vim.schedule(function() on_token(snap) end)
            elseif obj.error then
              done = true
              vim.schedule(function() on_error(obj.error) end)
              return
            end
          end
        end
      end
    end,
  }, function(result)
    -- on_exit: curl process ended
    if not done then
      if result.code ~= 0 then
        vim.schedule(function()
          on_error("cade.nvim: curl exited with code " .. result.code)
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
