-- test/runner.lua
-- Minimal test harness for cade_ide tests.
-- Run with:  nvim --headless --noplugin -u test/runner.lua
--
-- Each test file is discovered in test/*.lua (excluding this runner).
-- Tests are plain functions named test_* inside a returned table.

vim.opt.runtimepath:prepend(vim.fn.fnamemodify(debug.getinfo(1).source:sub(2), ":h:h"))

local passed, failed, errors = 0, 0, {}

local function run_file(path)
  local ok, mod = pcall(dofile, path)
  if not ok then
    failed = failed + 1
    table.insert(errors, ("LOAD ERROR %s: %s"):format(path, mod))
    return
  end
  if type(mod) ~= "table" then return end
  for name, fn in pairs(mod) do
    if type(fn) == "function" and name:match("^test_") then
      local ok2, err = pcall(fn)
      if ok2 then
        passed = passed + 1
        io.write(("  ✓ %s :: %s\n"):format(vim.fn.fnamemodify(path, ":t:r"), name))
      else
        failed = failed + 1
        table.insert(errors, ("  ✗ %s :: %s\n    %s"):format(
          vim.fn.fnamemodify(path, ":t:r"), name, err))
      end
    end
  end
end

local test_dir = vim.fn.fnamemodify(debug.getinfo(1).source:sub(2), ":h")
local files = vim.fn.glob(test_dir .. "/*_test.lua", false, true)
table.sort(files)

for _, f in ipairs(files) do run_file(f) end

io.write(("\n%d passed, %d failed\n"):format(passed, failed))
for _, e in ipairs(errors) do io.write(e .. "\n") end

vim.cmd(failed > 0 and "cquit 1" or "qa!")
