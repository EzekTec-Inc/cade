# Architecture Decision Records (ADRs)

This directory documents the significant architectural decisions made in the evolution of CADE.

| ADR | Title | Status | Scope |
|---|---|---|---|
| [ADR-0001](0001-in-memory-api-key-storage.md) | In-Memory API Key Storage | Accepted | Security & Secrets |
| [ADR-0002](0002-sqlite-unified-knowledge-graph.md) | SQLite Unified Knowledge Graph | Accepted | Storage & Persistence |
| [ADR-0003](0003-direct-wal-busy-timeout-for-sqlite.md) | Direct WAL Busy Timeout for SQLite | Accepted | Storage Concurrency |
| [ADR-0004](0004-adaptive-memory-retention-and-archiving.md) | Adaptive Memory Retention and Archiving | Accepted | Memory System |
| [ADR-0005](0005-headless-approvals-queue-and-notifications.md) | Headless Approvals Queue and Notifications | Accepted | Security & Approvals |
| [ADR-0006](0006-workspace-isolation-and-mutation-locking.md) | Workspace Isolation and Mutation Locking | Accepted | Filesystem Sandboxing |
| [ADR-0007](0007-historical-high-water-mark-pinning-for-consolidation-boundary-markers.md) | Historical High-Water-Mark Pinning for Compaction | Accepted | Memory Compaction |
| [ADR-0008](0008-event-driven-client-approvals-and-slash-commands.md) | Event-Driven Client Approvals and Slash Commands | Accepted | Client/Server Protocols |
| [ADR-0009](0009-adaptive-typewriter-governor-for-tui-streaming.md) | Adaptive Typewriter Governor for TUI Streaming | Accepted | TUI Rendering |
| [ADR-0010](0010-decoupled-async-subagent-executor-trait.md) | Decoupled Async Subagent Executor Trait | Accepted | Subagents Engine |
| [ADR-0011](0011-unified-permissions-service-and-adapters.md) | Unified Permissions Service and Adapters | Accepted | Security & Policies |
| [ADR-0013](0013-pluggable-polymorphic-token-counters.md) | Pluggable Polymorphic Token Counters | Accepted | AI & Tokenization |
| [ADR-0014](0014-firecracker-microvm-hypervisor-sandboxing-and-vsock-exchange.md) | Firecracker MicroVM Hypervisor Sandboxing | Accepted | Execution Backends |
| [ADR-0015](0015-multi-agent-team-coordination-and-git-branch-sandboxing.md) | Multi-Agent Team Coordination and Git Branch Sandboxing | Accepted | Teams & Worktrees |
| [ADR-0016](0016-server-driven-tui-buffer-compaction-and-layout-caching.md) | Server-Driven TUI Buffer Compaction and Layout Caching | Accepted | TUI Viewport |
| [ADR-0017](0017-asynchronous-queue-decoupled-lua-plugins-for-tui-responsiveness.md) | Asynchronous Queue-Decoupled Lua Plugins | Accepted | Plugin Extensibility |
| [ADR-0018](0018-declarative-theme-schema-and-unified-lua-token-bindings.md) | Declarative Theme Schema and Unified Lua Token Bindings | Accepted | TUI Theming |
| [ADR-0019](0019-wasm-reactive-context-caching-and-sse-driven-state-synchronization.md) | WASM Reactive Context Caching and SSE Synchronization | Accepted | GUI Web Client |
| [ADR-0020](0020-capability-mesh-unified-execution-seam.md) | CapabilityMesh Unified Execution Seam | Accepted | Core Tools & Extensibility |
| [ADR-0021](0021-subagent-session-harness-and-lifecycle-sandboxing.md) | Subagent Session Harness and Lifecycle Sandboxing | Accepted | Autonomous Subagents |
| [ADR-0022](0022-cade-gui-sequential-refactoring-and-zero-placeholder-contract.md) | CADE GUI Sequential Refactoring & Zero-Placeholder Contract | Accepted | GUI Architecture |
| [ADR-0023](0023-gemini-and-headroom-proxy-integration.md) | Gemini and Headroom Proxy Integration Strategy | Accepted | AI Routing & Token Compression |
| [ADR-0024](0024-timeline-tool-activity-tree-and-pill-margins.md) | Timeline Tool Activity Tree and Pill Margins | Accepted | TUI Presentation & Layout |
