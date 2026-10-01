# ADR-0028: Working Session tool grants and shared approval outcomes

## Status

Accepted. Implements the user's confirmed session lifetime, Subagent sharing, and strict-Bash precedence decisions.

## Decision

“Allow for this session” grants the named tool, not an exact argument JSON value. A Working Session belongs to an active client in one canonical workspace and spans its Conversations and Runs. A fresh client receives a fresh Working Session; reconnecting the existing client retains its identity. Closing the Working Session invalidates grants held by already-running descendants as well as future Runs.

Permission evaluation shares a live grant handle with descendant Subagents rather than copying remembered rules at Run or child creation. Explicit deny rules, protected paths, Plan mode, and the accepted Execution Scope still constrain every invocation. A Remembered Tool Grant overrides repeated `strict_bash` prompting; configured allow rules alone retain their existing strict-Bash behavior.

Run and Subagent approval adapters use one interpretation of once, session, denial, and feedback outcomes. Session approval must be published through the same decision path, so accepting a decision is not dependent on which adapter happens to wait for it. Preserve existing cancellation and replay behavior and do not turn question answers into tool grants.

## Consequences

- Conversation identity is not a Working Session identity. Switching Conversations does not revoke a session grant, and deleting a Conversation does not define session closure.
- Native model aliases use the same canonical identity as dispatch; qualified external identities retain their namespace and case so distinct tools cannot acquire each other's grants.
- Retain existing transport action names; a remembered approval requires an active Working Session rather than silently treating a Conversation as one.
- Tests cross the production grant/decision seam: repeated changed arguments, different tools and sessions, late grants observed by existing Subagents, closure, and existing policy denials.
- Normal client exit awaits explicit session closure; interrupted tasks attempt closure on drop. A five-minute renewable lease bounds grants left by a killed or disconnected client, with renewal every thirty seconds. Reconnection within that lease retains the identity; an expired session cannot be resurrected or silently regain its grants.
- If the daemon loses or expires the identity while the CLI is still active, the client opens a fresh Working Session with empty grants before new execution. Already-running descendants retain their old, expired scope; they do not silently migrate into the new session.
- Automatically released approvals retain session-dependent outcomes, and matching Decision Dialogs are retired using their approval IDs. Remote resolution is distinct from a user denial.
