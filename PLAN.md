## 2026-04-27T21:40:00Z — code-review: cleanup batch (M9r, M6, M5, M1, N1-N3)

**Task:** Apply 6 lower-severity findings from validated code review (M9r, M6, M5, M1, N1-N3).

**Files modified:**
- `crates/cade-server/src/server/api/run.rs` (M9r, M1)
- `crates/cade-server/src/server/consolidation.rs` (M5)
- `crates/cade-agent/src/tools/manager.rs` (M6, N3)
- `crates/cade-agent/src/tools/runtime/mod.rs` (N1, N2)

**Previous behavior:**
- Agentic loop exit tracked as "done" indiscriminately.
- Tool name matching failed on MCP-prefixed names.
- Dead no-op `should_skip_noisy_tool` logic existed.
- Skill load/unload forced string round-tripping.
- Stray comments existed in manager/runtime.

**New behavior:**
- Exit status accurately tracked by `RunExitStatus`.
- Tool name resolution works for all MCP names.
- Cleanup of dead no-op logic.
- Direct JSON argument passing.
- Stripped stray comments.

**TDD record:**
- Tests: `cade-server` 258/258, `cade-agent` 113/113. 
- All passing.

**Dependency policy:** No new dependencies.

**Rollback steps:**
```sh
git reset --hard HEAD~1
```
