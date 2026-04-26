# Subagents

A **subagent** is a CADE agent the main agent can spawn programmatically
to handle isolated work. Subagents have their own context, their own
tools (filtered subset of the parent's), and report only their final
answer back — keeping the main agent's context window clean.

## When to use one

- **Deep codebase exploration** — searching, reading many files
- **Large file rewrites** — concentrated edits on one component
- **Code review** — independent assessment with no prior bias
- **Background memory maintenance** — reflection, summarisation
- **Long-running tasks** — anything > a few minutes that doesn't need
  user interaction

## Spawning one (from the LLM)

```
run_subagent(
  agent_id="worker",          # which subagent definition to use
  prompt="<task>",
  description="<short label shown in TUI>",
  model="anthropic/claude-haiku-4-5",   # optional override
  background=false,            # detach into a /v1/runs run
  test_command="cargo test"    # optional verification command
)
```

Returns the subagent's final assistant message. Intermediate text and
tool calls are **not** streamed back to the parent — only the result.

## Built-in subagents

Defined in `crates/cade-agent/src/subagents/mod.rs::builtin_subagents`.

| Name | Tools | What it does |
|---|---|---|
| `worker` | All | Highly capable unified worker — explore, plan, implement, review |
| `reflection` | `update_memory`, `read_file`, `glob` | Background memory maintenance |
| `recall` | Read-only | Search past conversations and files for context |

Discover all visible subagents (built-in + global + project):

```bash
/subagents
```

## Custom subagents

Drop a Markdown file into `~/.cade/subagents/` (global) or
`.cade/subagents/` (project):

```markdown
---
name: bug-hunter
description: Inspect a stack trace and find the root cause
model: anthropic/claude-sonnet-4-5
tools:
  - read_file
  - glob
  - grep
---

You are a bug-hunting agent. Given a stack trace and a workspace,
identify the root cause...
```

Frontmatter fields:

- `name` (required) — id used in `run_subagent(agent_id=...)`
- `description` (required) — shown in `/subagents`
- `model` (optional) — override the parent's model
- `tools` (optional) — `all`, `readonly`, or a specific list

Same-name conflict: project > global > built-in.

## Defence layers (recursion safety)

CADE caps subagent runaway via six defences:

1. **Depth cap** — `CADE_SUBAGENT_MAX_DEPTH` (default 3). Every nested
   `run_subagent` increments depth; over the cap, the call refuses
   before any LLM is hit.
2. **Tool-list filter** — subagents never see `run_subagent` in their
   own toolset (so they cannot recurse via the tool).
3. **Re-entry guard** — depth is bumped at each level even if the tool
   filter is bypassed.
4. **Global semaphore** — `CADE_MAX_SUBAGENTS` (default 4) caps
   concurrent subagent runs across the whole server.
5. **Per-level iteration cap** — `CADE_SUBAGENT_MAX_ITERS` (default 10)
   limits agentic-loop iterations inside a single subagent.
6. **DB-pollution watchdog** — server tests verify subagent runs do not
   leak agent rows or message history into the parent's DB.

All six are tunable via env vars; see [configuration.md](configuration.md).

## Memory & state

A subagent runs **in-memory only**:

- **No** ephemeral agent rows are created in `agents`
- **No** messages are persisted
- The parent's full tool list (minus `run_subagent`) is dispatched via
  the same `cade_agent::tools::manager::dispatch` path
- Final result is returned as a string to the parent's tool-call result

The reflection subagent is the exception — it explicitly calls
`update_memory` so its work survives.

## Background runs

Set `background: true` to detach. The call returns immediately with a
run id; query progress via `GET /v1/runs/:id` or re-attach with
`/v1/runs/:id/stream`.

## Performance tips

- **Pick a cheaper model** for read-heavy work (`worker` defaults to the
  parent's model; override per-call for cost).
- **Filter tools** when authoring custom subagents — fewer tools means
  smaller schemas in the prompt.
- **Use `description`** generously — shown in TUI cards, helps you
  monitor running subagents at a glance.
