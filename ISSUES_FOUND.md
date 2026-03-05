# CADE — Issues Found

> Discovered during code review on 2026-03-04.
> All issues relate to the track: **Fix: Tool run progress and results not displayed after question modal**.

---

## Issue #1 — Double ToolResult Render for ask_user_question (Bug)

**Severity:** High
**Status:** Open
**File:** src/cli/repl.rs
**Lines:** ~2074-2077 and ~2559-2562

### Description
After a user answers a question modal, two RenderLine::ToolResult lines are pushed to the viewport for a single ask_user_question tool call:

1. **First push** inside handle_ask_user_question() at line ~2559, which pushes the formatted answer summary.
2. **Second push** inside dispatch_tool_calls() at line ~2074, which unconditionally pushes another ToolResult after execute_tool() returns.

### Impact
A duplicate result line appears in the REPL viewport after every ask_user_question call, causing visual noise and user confusion.

### Root Cause
dispatch_tool_calls() wraps all tool calls with an unconditional ToolResult push after execute_tool() returns. Meta-tools like ask_user_question that handle their own result rendering are not excluded from this second push.

### Suggested Fix
- Skip the unconditional ToolResult push in dispatch_tool_calls() for tools that manage their own rendering, or
- Remove the internal ToolResult push from handle_ask_user_question() and rely solely on the one in dispatch_tool_calls().

---

## Issue #2 — Dead Code: Thinking Bar Word Count Update (Minor Bug)

**Severity:** Low
**Status:** Open
**File:** src/cli/repl.rs
**Line:** ~1895

### Description
A recent commit renamed the variable words to _words to suppress a compiler warning. This variable was intended to update the thinking bar text with a word count proxy during streaming. The rename silently disables that logic — the bar is never updated with a count.

### Impact
The thinking bar does not update with a word/line count proxy during streaming — a silent regression.

### Suggested Fix
Either restore the bar update logic using _words, or remove the dead code entirely if the feature is no longer desired.

---

## Issue #3 — Unused stdout Parameter in Meta-Tool Handlers (Code Smell)

**Severity:** Low
**Status:** Open
**File:** src/cli/repl.rs
**Lines:** ~2612, ~3371

### Description
handle_run_subagent() and handle_install_skill() both accept a _stdout: &mut io::Stdout parameter that is never used inside the function body. It was renamed from stdout to _stdout to suppress the compiler warning, but the unnecessary parameter remains.

### Impact
No functional impact currently, but creates a misleading API surface. Callers must pass a stdout handle that is silently ignored, causing confusion during future maintenance.

### Suggested Fix
Remove the _stdout parameter from both function signatures and update all call sites accordingly.

---

## Issue #4 — Track Metadata Status Mismatch (Process Issue)

**Severity:** Low
**Status:** Open
**File:** conductor/tracks/fix_tool_progress_display_after_question_modal_20260304/metadata.json

### Description
Git commit b9aa0e4 claims the track is complete, but metadata.json still shows status: in_progress. The track was never formally verified with a runtime test before being declared complete.

### Suggested Fix
1. Resolve Issues #1-#3 above.
2. Run CADE, trigger the ask_user_question modal, make a selection, and confirm tool results render correctly in the viewport.
3. Update metadata.json to status: complete only after a successful runtime test.

---

## Issue #5 — No Automated Test Coverage for Question Modal to Viewport Path

**Severity:** Medium
**Status:** Open
**File:** N/A (missing test file)

### Description
There are no unit or integration tests exercising the full path:
  ask_user_question tool call
    -> QuestionWidget rendered
    -> User makes selection
    -> ToolResult pushed to TuiApp
    -> Viewport redraws correctly

Correctness of the modal to viewport rendering pipeline relies entirely on manual testing, making it fragile to future regressions.

### Suggested Fix
Add tests covering:
- AskUserQuestionTool::parse_questions() with valid and invalid inputs.
- AskUserQuestionTool::format_result() for answer formatting.
- TuiApp::push() ensuring RenderLine::ToolResult is correctly appended to lines.
- A mock end-to-end test for the handle_ask_user_question -> ToolResult push flow.

---

## Issue #6 — OpenAI 400: object schema missing properties (Bug)

**Severity:** High
**Status:** Closed
**File:** src/mcp/mod.rs

### Description
When an MCP tool has no parameters, the MCP spec allows returning `{"type": "object"}` for the input schema. However, OpenAI's strict JSON schema parser rejects this, requiring `{"type": "object", "properties": {}}` at a minimum.

### Impact
Tools with no arguments registered via MCP fail to execute or get attached to the agent when using OpenAI models.

### Suggested Fix
Update the parameter conversion in `src/mcp/mod.rs` to dynamically inject an empty `properties` object if the type is `object` but `properties` is missing.
*(Fixed by injecting the empty properties object).*

---

## Issue #7 — Anthropic 400: max_tokens: 16384 > 4096 (Bug)

**Severity:** High
**Status:** Closed
**Files:** src/server/api/messages.rs, src/server/llm/anthropic.rs, src/server/llm/catalogue.rs

### Description
CADE was hardcoding `MAX_TOKENS = 16384` for all CompletionRequests. While this works for GPT-4.5 and Claude 3.5 Sonnet, older models like Claude 3 Haiku and Opus enforce a strict 4096 limit. The Anthropic client also used a `DEFAULT_MAX_TOKENS = 8192` which overwrote smaller limits.

### Impact
Models with smaller token bounds (like Claude 3 Haiku) return HTTP 400 errors and fail to stream responses.

### Suggested Fix
Introduce a `max_tokens` field in the static model catalogue, pass it dynamically through the `CompletionRequest`, and remove the hardcoded global constants.
*(Fixed by implementing dynamic token limits via the catalogue).*

---

## Summary Table

| # | Issue | Severity | Status | File |
|---|-------|----------|--------|------|
| 1 | Double ToolResult render for ask_user_question | High | Open | src/cli/repl.rs |
| 2 | Dead code: thinking bar word count update | Low | Open | src/cli/repl.rs |
| 3 | Unused _stdout parameter in meta-tool handlers | Low | Open | src/cli/repl.rs |
| 4 | Track metadata status mismatch | Low | Open | conductor/tracks/.../metadata.json |
| 5 | No automated test coverage for modal to viewport path | Medium | Open | N/A |
| 6 | OpenAI 400 object schema missing properties for no-arg tools | High | Closed | src/mcp/mod.rs |
| 7 | Anthropic 400 max_tokens limit exceeded | High | Closed | src/server/llm/anthropic.rs, src/server/api/messages.rs, src/server/llm/catalogue.rs |
