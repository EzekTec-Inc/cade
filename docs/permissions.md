# Permissions

CADE asks before running tools. The permission system controls **whether**
to ask and **what's protected** even with approval bypassed.

## Modes

Lives in `cade-core::permissions::PermissionMode`. Cycle with `/mode <name>`,
or shortcut commands:

| Mode | Internal name | Icon | Behaviour |
| --- | --- | --- | --- |
| **Safe** | `default` | ✅ | All tool calls require approval |
| **Edit freely** | `acceptEdits` | 📝 | File edits auto-approved; other tools ask |
| **Plan only** | `plan` | 📖 | Read-only — state-mutating tools are blocked |
| **Full access** | `bypassPermissions` | ⚡ | All tools auto-approved (`/yolo`) |

Aliases accepted: `safe`, `edit-freely`, `plan-only`, `full-access`,
`yolo`. Display names are user-facing; internal names appear in
`settings.json`.

## File I/O Sandboxing (Granular RBAC)

CADE agents and subagents execute tool calls through `cade_agent::tools::manager::dispatch`, which enforces path-based sandboxing when interacting with the file system (`read_file`, `write_file`, `bash`, etc.).
If an agent attempts to access a path outside of the provided `allowed_paths`, the tool dispatcher immediately rejects the request with a `[Blocked by RBAC]` error. This sandbox is strictly configurable per session and heavily enforced during subagent execution.

## Per-tool rules

```bash
/permissions                       # show current mode + rules
/approve-always <pattern>          # add a permanent allow rule
/deny-always <pattern>             # add a permanent deny rule
```

Patterns can match by tool name (e.g. `bash`) or by argument
substring. They persist into `~/.cade/settings.json` under
`permissions.always_allow` / `permissions.always_deny`.

Example:

```json
{
  "permissions": {
    "always_allow": ["read_file", "glob"],
    "always_deny": ["bash:rm -rf"]
  }
}
```

Allow rules win over deny? **No.** Deny rules win — a deny match short-
circuits before any allow check.

## Path protection (always on, even in YOLO)

`crates/cade-core/src/permissions/rules.rs::path_is_protected` denies
**writes** to these paths regardless of mode:

- `.git/`, `.git/config`, etc.
- `.ssh/`, including any sub-path
- `.env`, `.env.local`, `.env.*`
- `~/.cade/db.key` (the SQLite encryption key)
- `.cade/db.key` and `./.cade/db.key`

**Reads** are allowed (so the agent can `cat .git/HEAD` to inspect, but
cannot `echo > .git/config`).

Bash command sniffing also flags suspicious patterns:

- `eval $PAYLOAD`
- `cat /etc/passwd`
- `cat ~/.ssh/id_rsa`
- redirects writing into protected paths

## Plan mode specifics

`/plan` enables a read-only sandbox. The full toolset is still presented
to the LLM, but state-mutating tools (any `write_*`, `edit_file`, `bash`,
`run_subagent`, etc.) return a "blocked by plan mode" error before
execution.

Use plan mode when:

- Reviewing an LLM's proposed changes before committing
- Exploring an unfamiliar codebase
- Running an agent against a production checkout

## YOLO mode

`/yolo` (alias for `bypassPermissions`) is intended for sandboxed
environments — Docker, VM, ephemeral CI runner. It auto-approves
**every** tool call but **does not** disable path protection or
suspicious-command detection.

> **Warning** — combining `/yolo` with a real working directory and a
> network-connected server is at-your-own-risk. The path protection list
> is not exhaustive (it covers credentials and CADE's own DB key, not
> arbitrary user secrets).

## Programmatic access

The Rust API is in `crates/cade-core/src/permissions/`:

```rust
let mgr = PermissionManager::new(PermissionMode::Default);
let outcome = mgr.resolve("write_file", &args, /*is_tool_for_review=*/false);
match outcome {
    Outcome::Allow => /* run */,
    Outcome::Deny  => /* refuse */,
    Outcome::Ask   => /* prompt user */,
}
```

Tests in `crates/cade-core/src/permissions/tests.rs` verify path
protection, suspicious-bash detection, and granular allow/deny rules.

## Plan-mode + hooks combined

A `PreToolUse` hook can supplement plan mode by blocking specific
patterns even in `default` mode. See [hooks.md](hooks.md).

## Mode Model Retention (Auto-switching LLMs per Mode)

To optimize cost and quality across different tasks, CADE automatically remembers and swaps the active LLM model when you switch between permission modes.

### Key Characteristics

- **Automatic Mapping:** When you are in a specific mode (e.g., `/plan` or `/default`) and run `/model <model_name>`, CADE automatically registers that model as your preference for the active mode.
- **Seamless Auto-Switching:** Whenever you switch modes (via `/mode`, `/plan`, `/default`, `/yolo`, or dynamically cycling with the `Tab` / `Shift-Tab` keyboard shortcuts), CADE automatically patches the backend agent, updates your active toolset, and displays a TUI notification (`🔄 Auto-switching model to ...`).
- **Restart Persistence:** Your preferred model mappings are persisted in the gitignored **Local Settings** layer (`.cade/settings.local.json`). When you restart CADE, it will automatically resolve and apply your preferred model for your startup permission mode.
