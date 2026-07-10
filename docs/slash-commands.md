# Slash Commands Reference

All commands exposed by the CLI/TUI. Type `/help` inside a session for an
in-app version. The CLI parser lives in `crates/cade-cli/src/cli/repl/slash.rs`;
the GUI palette uses the same triggers via `cade-core::resources::palette`.

> Convention: `<arg>` is required, `[arg]` is optional. Empty input opens an
> interactive picker for most commands that take an argument.

## Session

| Command | Aliases | Description |
|---|---|---|
| `/help` | `/?`, `/menu` | Show all commands |
| `/exit` | `/quit`, `/q` | Quit the session |
| `/clear` | | Clear the visible timeline (server state untouched) |
| `/new` | | Start a fresh conversation on the current agent |
| `/agent` | | Show current agent name + id |
| `/info` | | Detailed agent + workspace info |
| `/feedback` | | Submit feedback to the CADE team |
| `/logout` | | Clear credentials and return to login |
| `/stream` | | Toggle streaming on/off |
| `/update` | | Check for and apply CADE updates (updates CLI and server) |

## Agents & conversations

| Command | Aliases | Description |
|---|---|---|
| `/agents` | `/agent-list` | List agents on the server |
| `/new-agent` | | Create a new agent (interactive) |
| `/rename <name>` | | Rename current agent |
| `/delete [name]` | `/del`, `/rm-agent` | Delete agent (current if no arg) |
| `/pin` | | Pin current agent as the global default |
| `/resume` | | Browse past conversations and switch |
| `/init` | | Generate a starter `project` memory block |
| `/checkpoint [label]` | `/cp` | Save a working-tree checkpoint (git commit) |
| `/tree` | `/checkpoints`, `/session-tree` | Browse + restore checkpoints |
| `/fork [label]` | | Branch a new conversation from a checkpoint |
| `/undo` | | Restore the most recent checkpoint |
| `/artifacts` | | List stored artifacts (logs, diffs, reports) |
| `/export [path]` | | Export current agent to JSON |

## Model & permissions

| Command | Aliases | Description |
|---|---|---|
| `/model [provider/name]` | | Switch model; empty arg opens picker |
| `/compaction-model <name>` | | Set per-agent summarisation model |
| `/reasoning [level]` | | Set reasoning effort: `none\|low\|medium\|high\|xhigh` |
| `/toolset [name]` | | Show / switch toolset: `default\|codex\|gemini` |
| `/mode [name]` | | Show / set permission mode |
| `/default` | `/normal` | Switch to default permission mode |
| `/plan` | | Switch to read-only plan mode |
| `/yolo` | | Bypass all permission prompts |
| `/permissions` | | Show current mode + rules |
| `/approve-always <pattern>` | | Permanent allow rule |
| `/deny-always <pattern>` | | Permanent deny rule |

## Approvals & Multi-Agent Steering

| Command | Description |
|---|---|
| `/approvals` | List all active pending tool approvals |
| `/approve <id>` | Approve a pending tool authorization request |
| `/deny <id> [feedback...]` | Deny a request with optional steering feedback (notifies the subagent) |
| `/steer <subagent_id> <message>` | Intervene/redirect an active background subagent with instructions |

## Memory

| Command | Description |
|---|---|
| `/memory` | List all memory blocks |
| `/memory view <label>` | Show full content of a block |
| `/memory set <label> <value>` | Set a block value |
| `/memory edit <label>` | Edit a block in `$EDITOR` |
| `/memory delete <label>` | Remove a block |
| `/memory history <label>` | Show last 5 revisions |
| `/memory pin <label>` | Pin a block (exempt from aging) |
| `/memory unpin <label>` | Unpin |
| `/remember <text>` | Add to the `working_set` block |
| `/search <query>` | Full-text search across messages |
| `/reflect [focus]` | Trigger reflection subagent to extract memory |
| `/summarize` | `/summary` — show the auto-generated session summary |
| `/compact` | `/consolidate` — manually trigger Sleeptime consolidation |

See [memory-system.md](memory-system.md) for tier semantics.

## Tools, MCP & skills

| Command | Description |
|---|---|
| `/mcp` | Interactive picker to manage MCP servers and their tools |
| `/link` | Re-scan and re-attach all tools to the active session |
| `/unlink` | Detach all tools from the active session |
| `/mcp-save <name>` | Persist a connected server to `settings.json` |
| `/connect <name>` | Re-attach a saved MCP server |
| `/disconnect <name>` | Stop and detach an MCP server |
| `/skills [filter]` | Browse installed skills |
| `/subagents` | `/agents-list` — list discovered subagents |
| `/hooks` | Show configured hooks |

## Cost, telemetry & status

| Command | Description |
|---|---|
| `/context` | Show context-window usage % |
| `/stats [model]` | Per-model token usage |
| `/usage` | Cumulative token usage for the session |
| `/cost` | Cost breakdown (tokens × pricing) |
| `/pricing [sync\|edit]` | View or sync pricing rules |
| `/backend [name]` | Show / switch execution backend (local/docker/ssh) |

## Display

| Command | Description |
|---|---|
| `/theme [name]` | Switch theme; empty arg opens picker |
| `/copy` | Copy last assistant reply to clipboard |
| `/mouse` | `/select` — toggle scroll-wheel capture (off by default; text selection works natively) |
| `/todos` | Toggle visibility of the active plan checklist (`Ctrl+T`) |
| `/todo` | Show contents of `.cade-todo.md` (static scratchpad) |
| `/debug-last` | Dump the last assistant message as stored on the server |
| `/providers` | `/provider-list` — list LLM providers |

## GUI dashboard parity

The WASM dashboard at `/dashboard` understands a subset of these commands
through its **command palette** (`Ctrl+P`). Commands that require a
terminal-only feature (e.g. `/mouse`, `/export`) display a toast pointing
to the CLI/TUI. See [gui-dashboard.md](gui-dashboard.md).

## Authoring custom commands

Slash commands are defined in `crates/cade-cli/src/cli/repl/slash.rs` as a
single `SlashCmd` enum and a `parse_slash` matcher. To add one:

1. Add a variant to `SlashCmd`.
2. Map a trigger string in `parse_slash`.
3. Handle it in `crates/cade-cli/src/cli/repl/commands.rs`.
4. If it should appear in the GUI palette, add a `CmdDef` entry to
   `crates/cade-core/src/resources/palette.rs::CMD_DEFS`.
