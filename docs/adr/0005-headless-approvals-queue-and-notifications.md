# ADR 5: Headless and Background Tool Approvals Queue with Desktop Notifications

* **Status**: Proposed
* **Decided on**: 2026-06-25

## Context

CADE allows spawning background subagents (`background = true`) and executing headless prompts. In these environments, there is no active, interactive terminal standard input (`stdin`) attached to the subagent's execution thread.

Under default security permissions, any mutating tool call (e.g., `bash`, `write_file`) requires user confirmation. If a background subagent encounters a mutating call, it currently has no clean way to request approval:
1. Blocking standard input would hang the background thread indefinitely.
2. Automatically failing the call (fail-closed) degrades the autonomy and success rate of complex tasks.
3. Automatically running the call (fail-open) violates the project's security constraints.

We need a non-blocking, secure mechanism to handle tool permissions for background and headless runs.

## Decision

We decided to implement a **Database-Backed Approvals Queue** coupled with **Local Desktop Notifications** to handle background tool permissions:

1. **Pending Approvals Table**: When a background or headless subagent attempts to execute a restricted tool, the executor halts execution and writes a record into the `pending_approvals` table.
2. **State Suspension**: The subagent thread suspends execution, periodically polling the record status or waiting on a database-reactive event trigger.
3. **Desktop Notification**: CADE fires a local desktop notification (using system-native toast notification APIs) detailing the subagent's request (e.g., `"Subagent requests permission to run bash: cargo test"`). Where supported, these notifications include interactive **Approve** and **Deny** buttons.
4. **Interactive Intercept**: 
   * The user can approve/deny directly from the notification bubble, the CADE TUI (using `/approvals` or `/approve <id>`), or the CADE Web GUI Dashboard.
   * On approval, the database record status changes to `Approved`, the subagent thread is woken up, and tool execution proceeds safely.
   * On denial, the status changes to `Denied`, and the subagent receives a `PermissionDenied` error, allowing it to adapt its strategy.

## Consequences

### Positive (Pros)
* **Uncompromised Security**: High-security, fail-closed permission models are maintained even for asynchronous background executions.
* **Autonomy Preservation**: Subagents can make progress on long-running tasks without crashing when they require a single mutating command.
* **Seamless Multi-Channel UX**: Users can monitor and approve background tasks through their preferred interface (Desktop banner, TUI command, or Web Dashboard).

### Negative (Cons)
* **Database & IPC Complexity**: Requires maintaining state synchronization between background threads, the central SQLite database, and client interfaces.
* **Notification Dependency**: Relies on host OS capabilities for desktop notifications and action button support, which can vary across Linux, macOS, and Windows.
