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

## Summary Table

| # | Issue | Severity | Status | File |
|---|-------|----------|--------|------|
| 1 | Double ToolResult render for ask_user_question | High | Open | src/cli/repl.rs |
| 2 | Dead code: thinking bar word count update | Low | Open | src/cli/repl.rs |
| 3 | Unused _stdout parameter in meta-tool handlers | Low | Open | src/cli/repl.rs |
| 4 | Track metadata status mismatch | Low | Open | conductor/tracks/.../metadata.json |
| 5 | No automated test coverage for modal to viewport path | Medium | Open | N/A |
