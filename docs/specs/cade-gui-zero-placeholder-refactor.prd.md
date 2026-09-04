# Product Requirements Document (PRD): cade-gui Zero-Placeholder Refactor

## 1. Objective
Refactor `cade-gui` from end-to-end to eliminate all mock states, placeholder pages, and unverified assumptions. Ensure that every button, form, viewer, stream, and telemetry metric binds 1:1 to real `cade-server` `/api/v1/*` endpoints with robust error handling, reconnection logic, and responsive UI/UX.

---

## 2. User Personas & Scenarios
- **AI Systems Engineer**: Monitors active subagent execution, reviews diffs/AST edits inline, approves or denies dangerous tool executions (bash, write_file), and observes token consumption.
- **Developer / Contributor**: Inspects live memory blocks (knowledge graph, active goal), models, and provider endpoints, altering configuration without touching raw configuration files manually.

---

## 3. Scope & Vertical Slices

### Slice 1: Core Harness, Workspace Context & SSE State Bus
- **Goal**: Establish rock-solid state management, live workspace CWD synchronization, and resilient SSE event stream parsing with cancellation handles.
- **Key Modules**: `crates/cade-gui/src/api_engine.rs`, `chat_session.rs`, `types.rs`, `components/sidebar.rs`.
- **Success Criteria**:
  - Sidebar reflects real working directory and active project metadata.
  - In-flight streams can be aborted cleanly via `SafeAbortHandle` without memory leaks.
  - Toast and error notifications display meaningful HTTP status failures.

### Slice 2: Overview & Topology Dashboard
- **Goal**: High-fidelity visualization of active agent topology, model routing, MCP server connectivity, and memory allocations.
- **Key Modules**: `crates/cade-gui/src/components/dashboard.rs`.
- **Success Criteria**: Live polling of `/api/v1/metrics` and dynamic interactive SVG node graph.

### Slice 3: Interactive Chat, Tool Approvals & Stream Rendering
- **Goal**: Chunked streaming conversation view with inline tool approvals, diff viewer, file picker (`@`), and command autocomplete (`/`).
- **Key Modules**: `crates/cade-gui/src/components/chat.rs`, `markdown.rs`.
- **Success Criteria**: Tool calls display structured diffs; user can approve/reject executions with instant feedback to server.

### Slice 4: Code & Workspace Inspection
- **Goal**: Real filesystem tree browsing and audit log explorer.
- **Key Modules**: `crates/cade-gui/src/components/code.rs`, `logs_page.rs`.
- **Success Criteria**: Read files on demand with syntax highlighting; filter historical audit trail by event type and keyword.

### Slice 5: Memory Blocks & Tools / Approvals
- **Goal**: Interactive memory graph inspection/editing and MCP tool permissions management.
- **Key Modules**: `crates/cade-gui/src/components/memory.rs`, `tools_page.rs`.
- **Success Criteria**: Edit memory blocks (`human`, `project`, `persona`) with live patch sync; toggle tool permissions.

### Slice 6: Models, Providers, Telemetry & Settings
- **Goal**: Provider configuration, key management, spend metrics, and global UI preferences.
- **Key Modules**: `crates/cade-gui/src/components/models_page.rs`, `providers.rs`, `usage.rs`, `settings.rs`, `api_keys.rs`.
- **Success Criteria**: Add/remove providers, test endpoint latency, configure spend caps, and persist theme settings.
