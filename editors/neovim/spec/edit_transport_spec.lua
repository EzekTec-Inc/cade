describe("edit transport", function()
  local edit, original_system, original_schedule, options, exited, queue, events, killed
  local function flush()
    local pending = queue
    queue = {}
    for _, fn in ipairs(pending) do fn() end
  end
  local function request()
    return edit.fetch_edit("", "original", "", "rewrite", "lua",
      function(text) table.insert(events, { "token", text }) end,
      function() table.insert(events, { "done" }) end,
      function(err) table.insert(events, { "error", tostring(err) }) end)
  end
  before_each(function()
    require("cade.config").setup({ agent_id = "test" })
    edit = require("cade.edit")
    queue, events, killed = {}, {}, 0
    original_system, original_schedule = vim.system, vim.schedule
    vim.schedule = function(fn) table.insert(queue, fn) end
    vim.system = function(_, opts, on_exit)
      options, exited = opts, on_exit
      return { kill = function() killed = killed + 1 end }
    end
  end)
  after_each(function()
    vim.system, vim.schedule = original_system, original_schedule
  end)

  it("streams fragmented SSE and emits only one terminal event", function()
    request()
    options.stdout(nil, 'data: {"message_type":"stream_delta","content":"rep')
    options.stdout(nil, 'lacement"}\n\ndata: [DONE]\n\n')
    options.stdout(nil, 'data: {"error":"late"}\n')
    exited({ code = 0 })
    flush()
    assert.are.same({ { "token", "replacement" }, { "done" } }, events)
  end)

  it("does not treat clean EOF after partial output as success", function()
    request()
    options.stdout(nil, 'data: {"message_type":"stream_delta","content":"partial"}\n')
    exited({ code = 0 })
    exited({ code = 0 })
    flush()
    assert.are.equal(2, #events)
    assert.are.equal("token", events[1][1])
    assert.are.equal("error", events[2][1])
    assert.is_truthy(events[2][2]:find("without a completion event"))
  end)

  it("accepts an explicit completion without a final newline", function()
    request()
    options.stdout(nil, 'data: {"message_type":"stream_end"}')
    exited({ code = 0 })
    flush()
    assert.are.same({ { "done" } }, events)
  end)

  it("cancellation suppresses already queued tokens and terminal callbacks", function()
    local cancel = request()
    options.stdout(nil, 'data: {"message_type":"stream_delta","content":"late"}\ndata: [DONE]\n')
    cancel()
    exited({ code = 0 })
    flush()
    assert.are.same({}, events)
    assert.are.equal(1, killed)
  end)

  it("makes stdout failure terminal", function()
    request()
    options.stdout("read failure", nil)
    options.stdout(nil, 'data: [DONE]\n')
    exited({ code = 1 })
    flush()
    assert.are.same({ { "error", "read failure" } }, events)
  end)

  it("reports process failure and synchronous spawn failure", function()
    request()
    exited({ code = 22, stderr = "bad response" })
    flush()
    assert.are.equal("error", events[1][1])
    assert.is_truthy(events[1][2]:find("bad response"))
    events = {}
    vim.system = function() error("spawn failed") end
    request()
    flush()
    assert.are.equal("error", events[1][1])
    assert.is_truthy(events[1][2]:find("spawn failed"))
  end)
end)
