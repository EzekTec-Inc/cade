# ADR 11: Unified Permissions Service and Pluggable Adapters

* **Status**: Accepted
* **Decided on**: 2026-06-25

## Context

CADE enforces robust security and authorization boundaries before executing mutating commands (`bash`, `write_file`, `edit_file`, etc.). Previously, the permissions and approvals logic was scattered across multiple disjointed locations:
1. Rule validation inside the core library (`cade-core`).
2. Interactive terminal prompting inside the CLI (`cade-cli`).
3. Database-backed approvals queue polling inside headless background subagents (`cade-server`).

This tight coupling and fragmentation violated the core principles of **locality** and **depth**. It duplicated state logic and made the addition of new client authorization frontends (such as Web GUI button approvals) extremely complex.

## Decision

We decided to unify CADE's security boundaries under a single, cohesive **`PermissionService`** trait defined in the core library (`crates/cade-core/src/permissions/service.rs`):

```rust
#[async_trait]
pub trait PermissionService: Send + Sync {
    /// Request permission to execute a tool. Returns true if approved, false if denied.
    async fn request_permission(
        &self,
        tool_name: &str,
        args: &Value,
    ) -> Result<bool, String>;
}
```

### Pluggable Adapters
By separating the service interface from the implementation, we enable pluggable adapters across crates, avoiding circular dependencies:
1. **`YoloBypassAdapter`** (`cade-core`): Automatically authorizes all tool executions (ideal for headless CI/CD runs or secure sandboxes).
2. **`HeadlessQueueAdapter`** (`cade-server`): Integrates with CADE's database approvals queue. When a background subagent requests a mutating command, this adapter writes to the database, fires system notifications, suspends the thread asynchronously, and polls until resolved.

The subagent tool-execution engine now invokes `PermissionService::request_permission` uniformly:
```rust
let service = HeadlessQueueAdapter {
    db: state.db.clone(),
    parent_agent_id: parent_agent_id.to_string(),
    subagent_id: subagent_id.clone(),
};
service.request_permission(&tc.name, &tc.arguments).await;
```

## Consequences

### Positive (Pros)
* **Single Source of Truth**: Security verification follows a single, uniform abstraction seam across all crates and environments.
* **Polymorphic Flexibility**: Adding new frontend authorization channels (e.g., clicking "Approve" in the Web GUI dashboard or receiving system D-Bus webhooks) is as simple as implementing a new `PermissionService` adapter.
* **Tighter Codebase Health**: Completely decouples background subagent loops from specific database schemas and polling loops, maintaining high **depth** and clean interfaces.

### Negative (Cons)
* **Asynchronous Trait Overhead**: Requires compile-time async trait translation, which is natively handled by Rust's stable `#[async_trait]` macro.
