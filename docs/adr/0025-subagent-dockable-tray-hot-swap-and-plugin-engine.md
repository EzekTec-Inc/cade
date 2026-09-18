# ADR-0025: Subagent Dockable Control Tray, Model Hot-Swap, and Server Plugin Engine

## Status

Accepted

## Context

CADE features an autonomous subagent delegation framework (`run_subagent`, `subagent`, `TeamExecutor`). However, several structural and operational gaps were identified during multi-agent workflows in both `cade-tui` and `cade-gui`:

1. **Background Subagent Failure under Constitutions**:
   When a parent agent's `[project]` memory mandates specific MCP servers (e.g. `serena` or `desktop-commander-mcp`), spawning a background subagent in `mode: "plan"` stripped all MCP tools from the schema. Because the subagent was bound by constitution to use tools it lacked, it looped until hitting iteration limits without converging.
2. **Parallel Concurrency Contention**:
   When parallel subagents executed concurrently with `worktree: true`, simultaneous git worktree commands created git index lock (`.git/index.lock`) contention. Furthermore, near-simultaneous completion created SQLite write transaction contention when writing back to the parent agent's namespace.
3. **Passive & Inaccessible Telemetry**:
   In `cade-tui`, the existing `F5` inspector overlay presented only a passive text buffer with no ability to pause, cancel, steer, hot-swap models, or monitor token costs. In `cade-gui`, `LiveView` did not offer interactive inspection or intervention capabilities.
4. **Dormant Plugin System**:
   While WASM and JS plugin runtimes existed in `crates/cade-agent`, the client HTTP transport was stubbed with `"not implemented"`, preventing dynamic plugin installation from the TUI or Web GUI.

## Decision

We establish an end-to-end architectural overhaul of the subagent, team, and plugin systems across `cade-server`, `cade-agent`, `cade-tui`, and `cade-gui`:

### 1. Full Inherited Tool Permissiveness for Subagents
Subagents automatically inherit the tool permissions specified in the parent agent's active constitution, including configured MCP tools. This prevents the subagent from being trapped in an unfulfillable constitutional conflict.

### 2. Serialized Worktree Mutex & Resilient DB Commits
Parallel subagent operations (`subagent(action="tasks")`, `TeamExecutor`) isolate git worktree creation behind an asynchronous serialization mutex. SQLite memory write-back transactions enforce WAL mode with exponential backoff and jitter to prevent database locking errors.

### 3. TUI Dockable Subagent Control Tray (`Ctrl+B` / `F5`)
- **Dockable Split-Pane**:
  - Terminal width $\ge 110$ columns: 65% Main Timeline / 35% Docked Subagent Control Tray.
  - Terminal width $< 110$ columns: Falls back to full-screen overlay mode.
- **Focus Navigation**:
  - `Tab` (when prompt is empty) or `Ctrl+W` toggles focus between the main prompt editor and the subagent control tray.
- **Interactive Control Suite**:
  - `s` or `i`: Steer active subagent.
  - `m`: Model hot-swap picker.
  - `Space`: Pause / Resume.
  - `x`: Cancel / Kill subagent.
  - Live token burn meters (prompt vs. generation tokens) and runtime cost metrics.

### 4. Next-Turn Model Hot-Swap Protocol
When a model hot-swap is triggered mid-flight:
- The current in-flight LLM call completes normally.
- On the next turn iteration, the subagent loop reads the updated model ID from the shared agent state.
- Conversation history is preserved; system prompts and tool schemas are dynamically re-adapted to the target provider.

### 5. Priority Steering Guidance Queue
Steering messages injected from the TUI or GUI are pushed into the subagent's message queue as high-priority user instructions prefixed with:
```text
[Supervisor Steering Guidance]: <message>
```
The subagent incorporates this guidance on its next iteration cycle.

### 6. Web GUI Live Activity Slide-Over Drawer
In `cade-gui` (`LiveView`), clicking any running run or subagent opens a slide-over drawer from the right displaying live streaming output (`stream_run`), token burn graphs, and quick action controls (Steer, Change Model, Approve/Deny, Cancel).

### 7. First-Class Server Plugin Engine
Implement complete plugin lifecycle endpoints on `cade-server`:
- `POST /v1/plugins/install` — Installs `.wasm` plugin package into `.cade/plugins/`.
- `GET /v1/plugins` — Lists installed plugins, exported tools, and statuses.
- `DELETE /v1/plugins/{id}` — Uninstalls a plugin.
- `GET /v1/plugins/events` — SSE stream for plugin lifecycle and execution events.
Exposed in `cade-tui` via `/plugin` and in `cade-gui` via a dedicated Plugins management view.

## Consequences

- **Positive**: Eliminates background and parallel subagent failures caused by tool filtering and git lock collisions.
- **Positive**: Provides full interactive visibility and steerability over background agents without leaving the TUI or GUI.
- **Positive**: Enables zero-downtime model hot-swapping when tasks require deeper reasoning or hit provider rate limits.
- **Positive**: Activates the WASM plugin engine as a unified capability across CLI and Web dashboards.
