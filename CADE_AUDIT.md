# CADE Audit Log

This file is automatically maintained by the `finish_task` tool.
Each entry is generated server-side when the agent completes a task.


## 2026-05-14T21:46:02Z — Add native finish_task tool for automated audit logging

**Reason:** Replace manual PLAN.md changelog process with server-side automated audit log generation in CADE_AUDIT.md

**Files modified:**
- M crates/cade-agent/src/tools/manager.rs
- M crates/cade-cli/src/cli/repl/turn_tools/runner.rs

---

## 2026-05-14T22:15:56Z — Fix set_plan and UpdatePlan client-side dispatch for TUI plan panel

**Reason:** Fix broken TUI plan panel — set_plan and UpdatePlan tool calls were falling through to 'Unknown tool' because the client-side dispatch had no handler for them

**Files modified:**
None

---

## 2026-05-14T23:01:40Z — Make marketplace_url configurable via settings.json with default fallback

**Reason:** The marketplace registry URL was hardcoded, preventing users from pointing to private or self-hosted registries

**Files modified:**
None

---

## 2026-05-14T23:34:20Z — Fix github-mcp-server to use release binary and correct write_tools list

**Reason:** github-mcp-server was using cargo run (debug mode) instead of the prebuilt release binary, wasting startup time and resources. Read-only tools were incorrectly listed as write_tools.

**Files modified:**
None

---
