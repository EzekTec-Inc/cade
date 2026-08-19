# ADR-0020: CapabilityMesh Unified Execution Seam

## Status

Accepted

## Context

CADE integrates capabilities across multiple disparate providers:
1. **Native Built-in Tools** (Rust functions compiled into the agent runtime: `bash`, `read_file`, `write_file`).
2. **Third-Party MCP Servers** (Child processes communicating via `stdio`, `sse`, or Streamable-HTTP).
3. **Declarative Skills** (File packages containing `SKILL.md`, references, and scripts).
4. **Sandboxed Subagents** (Isolated agent loops with restricted tool filters).

Prior to this decision, tool discovery, schema transformation, ITS tag attachment, and execution dispatch were fragmented:
- `crates/cade-server/src/server/api/messages/context.rs`, `subagent.rs`, and `tools.rs` each independently queried SQLite, polled `state.mcp.all_tool_schemas()`, stripped private fields, and attached ITS tags.
- Hot-reloaded MCP servers required manual, error-prone database synchronization from outside callers.
- Callers were tightly coupled to the underlying transport and execution mechanism of each capability.

## Decision

We introduce the **`CapabilityMesh`** as the single canonical deep seam between agent execution loops and all executable assets in CADE:

1. **Crate Placement**:
   - The core trait `CapabilityMesh`, `CapabilityIntent`, `CapabilityOutput`, and `ExecutionError` reside in `crates/cade-core/src/capabilities/mesh.rs`.
   - Concrete adapters (`McpAdapter`, `BuiltinAdapter`, `SkillAdapter`) reside in `crates/cade-mcp`, `crates/cade-agent`, and `crates/cade-core`.

2. **Interface Contract**:
   ```rust
   #[async_trait]
   pub trait CapabilityMesh: Send + Sync {
       /// Executes an action through the mesh.
       async fn execute(
           &self,
           intent: CapabilityIntent,
           cx: &mut ExecutionContext,
       ) -> Result<CapabilityOutput, ExecutionError>;

       /// Returns the active capability catalog formatted for LLM schema injection.
       async fn active_catalog(&self, cx: &ExecutionContext) -> Vec<TaggedToolSchema>;
   }
   ```

3. **Execution & Streaming**:
   - `execute` is unary, returning `Result<CapabilityOutput, ExecutionError>`.
   - Granular real-time progress (`notifications/progress`) and telemetry are streamed via an optional `tokio::sync::mpsc::Sender<CapabilityEvent>` provided within `ExecutionContext`.

4. **Catalog Caching & Tag Decoration**:
   - `CapabilityMesh` maintains an in-memory, read-through cache of all registered schemas, complete with ITS tags (`cade`, `mcp`, `core_mcp`, `meta`).
   - SQLite tool definitions are mirrored asynchronously in the background on configuration mutation and hot-reload.

5. **Resilience & Fault Recovery**:
   - On transient transport drops (e.g. dropped stdio pipes or network blips), the mesh performs a single transparent reconnect with exponential backoff before returning an error.
   - Persistent failures return structured `ExecutionError::Disconnected` and invalidate the in-memory catalog cache.

## Consequences

### Positive
- **High Locality**: Adding or modifying transports, caching policies, or security checks touches only `CapabilityMesh` without altering prompt builders or agent loops.
- **High Leverage**: Parent agents, subagents, TUI, and REST endpoints share an identical dispatch interface for all capabilities.
- **Zero Schema Fragmentation**: LLM tool schema injection is consistent across all turns and subagent contexts.

### Negative
- Requires a one-time refactor of `crates/cade-server/src/server/api/messages/context.rs` and `subagent.rs` to route through `CapabilityMesh`.
