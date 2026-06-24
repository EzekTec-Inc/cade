---
id: tdd-guide
name: tdd-guide
description: Enforce strict test-driven development using a red-green-refactor loop.
category: tdd
tags: [tdd, test, verification]
---

# TDD Guide — Enterprise Public-Facing

Enforce strict Test-Driven Development (TDD) via Red-Green-Refactor cycles. Operates under `strict-project-execution` guardrails.

## 1. The Red-Green-Refactor Loop
For every requested behavior change:
1.  **RED**: Identify the smallest testable behavior, write exactly one failing test, and run the targeted test to verify it fails for the expected reason.
2.  **GREEN**: Write the smallest amount of code necessary to make the test pass, and rerun the test to confirm success.
3.  **REFACTOR**: Refactor codebase layout only if all tests pass. Rerun tests after any edit.

## 2. Security & Privacy Safeguards
Every public-facing interface change **must** include tests for:
*   **Input Validation**: Reject malformed, oversized, or boundary values; block SQL/Command/Path Traversal injections.
*   **Auth**: Reject unauthenticated and unauthorized requests. Never hardcode credentials.
*   **Leaks**: Ensure public errors do not contain stack traces, private paths, or internal configurations.
*   **Privacy**: Never use real personal data/PII in tests. Check that no PII leaks into logs or debug states.

## 3. Test Isolation & Contract Safety
*   Each test must set up and tear down its own preconditions (zero mutable shared state, local transactions only).
*   Mock/stub all external HTTP/network integrations.
*   Test that existing public API response shapes, headers, and status codes are fully preserved.

## 4. Cycle Reporting Format
For every TDD loop, report exactly:
```
RED:      [test added/changed] — [command used] — [failure output summary]
GREEN:    [minimal code changed] — [command used] — [pass confirmed]
REFACTOR: [what changed, or "none"]
```
