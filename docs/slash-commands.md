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
| `/compact` | `/consolidate` — manually trigger Sleeptime consolidation (surfaces structured telemetry, skip reasoning, and syncs context % usage) |

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
| `/plugin [list]` | List installed plugins, enabled state, and registered capabilities |
| `/plugin install <url> [id]` | Install a plugin from a remote URL or registry into the active workspace |
| `/plugin uninstall <id>` | Safely uninstall and deregister a plugin by ID |
| `/skills [filter]` | Browse installed skills |
| `/subagents` | `/agents-list` — list discovered subagents and dockable control tray |
| `/marketplace` | Browse, inspect, and install plugins from the central marketplace |
| `/hooks` | Show configured hooks and trigger hot-reload |

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

## Practical Command Examples

### Example 1: Snapshotting and Restoring Working Tree State
Before asking CADE to perform a massive or risky code refactor:
```bash
# 1. Create a snapshot commit
/checkpoint before-major-refactor
# Output: ✓ Checkpoint created: cp-8106c89f [before-major-refactor]

# 2. If the agent makes an undesired change, immediately revert:
/undo
# Output: ✓ Restored working tree to checkpoint: cp-8106c89f
```

### Example 2: Inspecting Live Context Budget & Compaction Telemetry
```bash
/context
```
Output:
```text
Context Window Utilization:
  Active Model: anthropic/claude-sonnet-4-5 (200k tokens)
  Current Prompt: 18,420 tokens (9.2% of window)
  Budget Pressure: 34% of message allocation (threshold: 70%)
  Compaction State: Idle (last compaction: 14 turns ago)
  Eager Trigger: 6 turns remaining until eager threshold
```

### Example 3: Dynamic Subagent Intervention
When a long-running background subagent is exploring the wrong directory:
```bash
/steer sa_a94b Please stop inspecting node_modules; focus strictly on crates/cade-core/src
```
The subagent receives this instruction immediately as a system intervention message and adjusts course mid-flight.

### Example 4: Manual Memory Compaction
To force an immediate summarization and fact extraction pass:
```bash
/compact
```
Output:
```text
✓ Context compacted (session_summary: 1,420 chars)
```
Or if no dropped messages require compaction:
```text
✓ Compact skipped: no dropped turns to consolidate
```
The client queries post-consolidation token statistics upon completion, updating the real-time context token percentage indicator in the TUI status bar.

### Example 5: Managing Plugins via PluginEngine
To inspect, install, or uninstall dynamic plugins and capability packs:
```bash
# List all discovered plugins and their active states
/plugin list

# Install a plugin bundle into the project workspace
/plugin install https://github.com/example/cade-plugin-pack.git my-plugin

# Uninstall and deregister a plugin
/plugin uninstall my-plugin
```

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

---

## CLI Update Arguments

In addition to using `/update` inside an active TUI session, CADE can also be checked or updated directly from your terminal shell completely headless and server-free:

* **`cade --check-update`**: Checks if a new release is available and exits immediately.
* **`cade --update`**: Downloads, cryptographically verifies, and applies the latest CADE update (updating both `cade` and `cade-server`) and exits immediately.
