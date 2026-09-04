# ADR 8: Event-Driven Client Approvals Integration and Slash Commands

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

We previously implemented a database-backed approvals queue (ADR 5) on the CADE server to handle permission requests from background or headless subagents. 

However, we need to design how the interactive CADE CLI/REPL client (`cade-cli`) detects these pending requests and how users can review and approve them within their terminal session. We must avoid heavy background database polling to reduce resource usage, while ensuring that the user is immediately made aware of any blocked subagent that requires attention.

## Decision

We decided to implement a hybrid **Event-Driven Push and Manual Slash Command** architecture to integrate background approvals into the CLI client:

### 1. Real-Time Push (SSE Notification)
The CADE server already streams agent status updates, tool execution chunks, and completed metrics to the CLI client via Server-Sent Events (SSE). We will introduce a new event type on this channel:

```json
{
  "event": "approval_requested",
  "data": {
    "approval_id": "app-123456",
    "subagent_id": "sub-worker-4",
    "tool_name": "bash",
    "arguments": "{\"command\": \"cargo test\"}"
  }
}
```

When the subagent suspends and writes its request to the approvals queue, the server instantly emits this `approval_requested` SSE payload. 

The CLI client intercepts this event in its async stream receiver loop and:
* Rings the terminal bell (`\x07`).
* Displays an asynchronous, non-blocking toast warning:  
  `⚠️ Background Subagent [sub-worker-4] requests permission to run bash. Type /approvals to review.`

### 2. Manual Slash Commands (CLI Inbox & Review)
We will introduce three new CLI slash commands inside the REPL loop:

* `/approvals`: Performs a rapid `GET /v1/approvals` REST API request to the server, listing all active pending approvals, their IDs, requesting subagents, tools, and arguments.
* `/approve <id>`: Issues a `POST /v1/approvals/{id}/action` with `{ "action": "approve" }`. This updates the DB record, immediately waking up the blocked background subagent thread.
* `/deny <id>`: Issues a `POST /v1/approvals/{id}/action` with `{ "action": "deny" }`. This resumes the subagent with a `PermissionDenied` error, allowing it to adapt its plan.

## Consequences

### Positive (Pros)
* **Zero Resource Polling**: No background threads are needed to continuously poll the database, keeping CADE's local CPU footprint minimal.
* **Instantaneous Response**: The subagent event-stream pushes requests instantly, preventing subagents from waiting unnecessarily.
* **Unified Control**: The user has full, interactive terminal-based control over background subagents without leaving the main chat flow.

### Negative (Cons)
* **CLI Context Interruption**: Presenting an asynchronous alert mid-chat can slightly disrupt the user's reading flow, though the warning will be non-blocking and placed in a dedicated notification line.
