# ADR 0022: cade-gui Sequential Architecture Refactoring and Zero-Placeholder Contract

## Status
Accepted

## Context
`cade-gui` serves as the graphical management and interactive chat interface for CADE. To guarantee complete operational integrity without placeholders, assumptions, or silent mock states, all functional views and state pipelines must strictly map 1:1 against real `cade-server` endpoints (`/api/v1/*`), SSE event streams, and `cade-api-types`.

## Decision
Refactor `cade-gui` across 6 distinct, sequential vertical slices:

1. **Slice 1: Core Harness, Workspace Context & SSE State Bus** (`crates/cade-gui/src/api_engine.rs`, `chat_session.rs`, `types.rs`)
   - Real working tree / CWD telemetry reflection in the workspace selector.
   - Robust SSE stream reconnection, heartbeat processing, and cancellation handles (`SafeAbortHandle`).
   - Clean typed error boundaries without unhandled panics or silent fails.

2. **Slice 2: Overview & Topology Dashboard** (`crates/cade-gui/src/components/dashboard.rs`)
   - Live metrics polling against `/api/v1/metrics`.
   - Dynamic topology rendering reflecting active agents, providers, MCP servers, and tool permissions.

3. **Slice 3: Interactive Chat, Tool Approvals & Stream Rendering** (`crates/cade-gui/src/components/chat.rs`, `markdown.rs`)
   - High-performance chunked SSE streaming with syntax-highlighted markdown.
   - Inline interactive tool approval prompts (diff inspections, command executions, parameter confirmations).
   - `@` file selector and `/` slash command popup palettes.

4. **Slice 4: Code & Workspace Inspection** (`crates/cade-gui/src/components/code.rs`, `crates/cade-gui/src/components/logs_page.rs`)
   - Real filesystem tree traversal via `cade-server` workspace endpoints.
   - Live audit and event log querying with structured filtering.

5. **Slice 5: Memory Blocks & Knowledge Graph** (`crates/cade-gui/src/components/memory.rs`, `tools_page.rs`)
   - Direct inspection and editing of core memory blocks (`persona`, `human`, `project`, `active_goal`).
   - MCP tool registry display with dynamic grant/deny toggle controls.

6. **Slice 6: Models, Providers, Telemetry & Settings** (`crates/cade-gui/src/components/models_page.rs`, `providers.rs`, `usage.rs`, `settings.rs`, `api_keys.rs`)
   - Dynamic provider credential verification and latency probing.
   - Token budget metrics, spend limits, and persistent theme/UI preference configuration.

## Consequences
- Every view and user action produces verifiable REST/SSE network calls to `cade-server`.
- All mock macros (`stub_page!`) and placeholder pages are eliminated.
- Clean isolation between presentation components and `ApiClientEngine`.
