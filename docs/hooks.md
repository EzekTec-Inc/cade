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

## Practical Hook Recipes

### Recipe 1: PreToolUse Destructive Command Blocker
Block force pushes, branch deletions, and dangerous shell commands:
```bash
#!/usr/bin/env bash
# .cade/hooks/block-destructive.sh
set -euo pipefail
payload=$(cat)
command=$(jq -r '.args.command // ""' <<< "$payload")

if [[ "$command" =~ (git\ push\ .*(force|-f)|git\ branch\ -D|rm\ -rf\ /|mkfs) ]]; then
  echo "Blocked by hook: Command '$command' is classified as destructive" >&2
  exit 2
fi
exit 0
```
Configuration in `.cade/settings.json`:
```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "bash",
        "command": ".cade/hooks/block-destructive.sh"
      }
    ]
  }
}
```

### Recipe 2: PostToolUse Linter & Diagnostic Context Injector
Automatically run a syntax check after code writes and inject compiler errors directly into the LLM's tool result:
```bash
#!/usr/bin/env bash
# .cade/hooks/inject-diagnostics.sh
set -euo pipefail

# Run cargo check silently to capture compiler warnings/errors
if ! diag=$(cargo check --message-format=short 2>&1); then
  # Inject compiler feedback into LLM context
  jq -n --arg d "$diag" '{"additionalContext": ("Compiler diagnostics:\n" + $d)}'
  exit 0
fi

echo '{}'
exit 0
```

### Recipe 3: Desktop Notification on Subagent Completion
Send a native desktop notification when a background subagent finishes:
```bash
#!/usr/bin/env bash
# .cade/hooks/notify-subagent.sh
payload=$(cat)
subagent=$(jq -r '.subagent // "worker"' <<< "$payload")
outcome=$(jq -r '.outcome // "completed"' <<< "$payload")

notify-send "CADE Subagent Finished" "Subagent [$subagent] finished with outcome: $outcome"
exit 0
```
Configuration in `~/.cade/settings.json`:
```json
{
  "hooks": {
    "SubagentStop": [
      {
        "command": "~/.cade/hooks/notify-subagent.sh"
      }
    ]
  }
}
```

## Hot reload

`/hooks` reloads from disk without restarting the session. MCP server
config and permission rules reload at the same time.

## Headless mode

Hooks are honoured in `cade --prompt "..."` non-interactive runs and in
`--output-format json|stream-json` modes. Exit code 2 from a hook in a
headless run terminates the run with a non-zero exit.
