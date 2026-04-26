# Hooks

User-defined shell scripts that fire at lifecycle events. CADE's hook
system mirrors CADE Code's — same event names, same exit codes, same
JSON-on-stdin contract.

## Configuration

Edit `~/.cade/settings.json` (global) or `.cade/settings.json` (project).
Project hooks merge with global ones; project-defined hooks run first.

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "bash",
        "hooks": [
          { "type": "command", "command": "/path/to/audit.sh" }
        ]
      }
    ],
    "PostToolUse": [],
    "PostToolUseFailure": [],
    "PermissionRequest": [],
    "UserPromptSubmit": [],
    "Stop": [],
    "SubagentStop": [],
    "SessionStart": [],
    "SessionEnd": [],
    "Notification": []
  }
}
```

`matcher` is a regex (or literal) against the tool name. Empty / `null` /
`*` matches all tools. Only tool-related events (`PreToolUse`,
`PostToolUse`, `PostToolUseFailure`, `PermissionRequest`) honour the
matcher; the rest run unconditionally.

## Events

| Event | When | Can block? | stdin payload |
|---|---|---|---|
| `SessionStart` | TUI starts or `--prompt` run begins | No | `{ session_id, agent_id, cwd, … }` |
| `SessionEnd` | Session terminates | No | `{ session_id, duration_ms, … }` |
| `UserPromptSubmit` | User pressed Enter | Yes | `{ prompt, agent_id, … }` |
| `PreToolUse` | About to invoke a tool | Yes | `{ tool_name, args, agent_id, … }` |
| `PermissionRequest` | About to prompt user for approval | No (logging only) | `{ tool_name, args, mode, … }` |
| `PostToolUse` | Tool completed successfully | Inject context | `{ tool_name, args, output, … }` |
| `PostToolUseFailure` | Tool errored | Inject context | `{ tool_name, args, error, … }` |
| `Stop` | Agent emitted finish | No | `{ agent_id, finish_reason, … }` |
| `SubagentStop` | A subagent finished | No | `{ subagent, parent_id, … }` |
| `Notification` | Toast / sound / OS notification | No | `{ level, message, … }` |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Allow — proceed normally |
| `1` | Log to stderr, continue |
| `2` | **Block** — stderr is fed back to the agent as a tool error |

Other exit codes are treated as `1`.

## Injecting context (PostToolUse)

`PostToolUse` and `PostToolUseFailure` hooks may print a JSON object on
stdout to inject extra context into the tool result the LLM sees:

```bash
#!/usr/bin/env bash
# stdout is interpreted as JSON
echo '{"additionalContext": "Linter found 3 warnings; review before commit."}'
```

The agent sees the original tool output **plus** `additionalContext`
appended.

## Reading stdin

The full event payload arrives on stdin as a single JSON line. Parse it
with `jq`:

```bash
#!/usr/bin/env bash
payload=$(cat)
tool=$(jq -r '.tool_name' <<< "$payload")
case "$tool" in
  bash) exec /path/to/bash-audit.sh "$payload" ;;
  *)    exit 0 ;;
esac
```

## Worked example: gating writes by branch

```bash
#!/usr/bin/env bash
# .cade/hooks/branch-guard.sh
payload=$(cat)
branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)
tool=$(jq -r '.tool_name' <<< "$payload")

if [[ "$branch" == "main" && "$tool" =~ ^(write_file|edit_file|bash)$ ]]; then
  echo "Refusing write on main branch" >&2
  exit 2
fi
exit 0
```

Wire it into `settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "*",
        "hooks": [{ "type": "command", "command": ".cade/hooks/branch-guard.sh" }]
      }
    ]
  }
}
```

## Worked example: SessionStart context boost

This project uses `.cade/hooks/rag-session-start.sh` to verify the
workspace index is fresh and bump the agent into the right working
directory. See the file for an end-to-end pattern.

## Hot reload

`/hooks` reloads from disk without restarting the session. MCP server
config and permission rules reload at the same time.

## Headless mode

Hooks are honoured in `cade --prompt "..."` non-interactive runs and in
`--output-format json|stream-json` modes. Exit code 2 from a hook in a
headless run terminates the run with a non-zero exit.
