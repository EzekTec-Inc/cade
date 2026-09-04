# CADE SDK Architecture & Deep Modules

This document details the internal design, seam placement, and runtime contracts of the CADE Rust SDK (`cade-sdk`).

---

## 1. Dual Runtime Topologies

`cade-sdk` decouples agentic intelligence from execution topology, allowing seamless transitions between single-process embedded computing and distributed microservice architectures.

```
Topology A: Zero-Daemon Embedded Model (`EmbeddedSession`)
┌────────────────────────────────────────────────────────┐
│ Host Process (CLI / Web Service / Lambda / Test)       │
│                                                        │
│  ┌──────────────────────────────────────────────────┐  │
│  │ EmbeddedSession (cade-sdk)                       │  │
│  │                                                  │  │
│  │  ┌───────────────┐ ┌─────────────┐ ┌──────────┐  │  │
│  │  │  cade-ai      │ │ cade-agent  │ │cade-store│  │  │
│  │  │  (LlmRouter)  │ │(ToolRuntime)│ │ (SQLite) │  │  │
│  │  └───────┬───────┘ └──────┬──────┘ └────┬─────┘  │  │
│  │          │                │             │        │  │
│  │  ┌───────▼────────────────▼─────────────▼─────┐  │  │
│  │  │ Autonomous Agentic Loop & Event Emitter    │  │  │
│  │  └────────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────┘

Topology B: Daemon Client-Server Model (`AgentSession` / `CadeClientSdk`)
┌─────────────────────────┐                ┌─────────────────────────┐
│ Client Application      │   HTTP / SSE   │ CADE Server Daemon      │
│ (cade-sdk)              │ ─────────────▶ │ (cade-server Axum)      │
│ ├── CadeClientSdk       │ ◀───────────── │ ├── SQLite Database     │
│ └── Typed Event Stream  │                │ ├── MCP Child Processes │
│                         │                │ └── Multi-Tenant Agents │
└─────────────────────────┘                └─────────────────────────┘
```

---

## 2. Deep Modules & Seams

In accordance with the **Codebase Design** principles, `cade-sdk` exposes small, high-leverage interfaces that hide substantial operational complexity:

### A. The `EmbeddedSession` Seam
- **Interface**: 2 core methods: `prompt(text)` and `prompt_stream(text) -> Stream<CadeStreamEvent>`.
- **Hidden Complexity**:
  - Automatically initialises SQLite database schema (`cade-store`) with `PRAGMA busy_timeout = 5000`.
  - Configures `LlmRouter` with fallback API keys and environment variables.
  - Builds full context messages combining **System Prompt**, **3-tier Memory Blocks**, and **Recent Turns**.
  - Executes tool dispatch loop with maximum turn bounds (`max_turns`) and token budgets.

### B. The `CapabilityMesh` Seam (`cade-core::capabilities::mesh`)
- **Role**: Presents a single unified tool execution contract across three heterogeneous capability sources:
  1. **Built-in Native Tools**: High-performance Rust tools (`bash`, `read_file`, `write_file`, `grep`, `glob`, `edit_file`).
  2. **External MCP Processes**: JSON-RPC child servers dynamically spawned over stdio or HTTP.
  3. **Procedural Skills**: Markdown-defined domain playbooks (`.cade/skills/`).
- **Leverage**: Callers invoke `mesh.execute(tool_name, arguments)` without needing to know whether the tool runs as an in-process native function or an out-of-process subprocess.

### C. The `TeamSession` Seam (`team.rs`)
- **Role**: Coordinates multi-agent squads through a declarative supervisor/worker tree.
- **Hidden Complexity**:
  - Automated task decomposition and subagent delegation.
  - Inter-agent communication (`intercom`) channels and progress synchronization.
  - Parallel worker fan-out with concurrency limits.
  - Unified synthesis and final outcome aggregation.

---

## 3. Persistent 3-Tier Memory Architecture

`cade-sdk` manages agent memory across three distinct lifecycle tiers:

```
┌──────────────────────────────────────────────────────────────┐
│ Context Window                                               │
│                                                              │
│ 1. Pinned Tier (Always Loaded)                               │
│    [project], [persona], critical constitutions              │
│                                                              │
│ 2. Short-Term Tier (Sliding Context)                         │
│    Recent conversation history & active goal state           │
│                                                              │
│ 3. Long-Term Tier (Archival Storage)                         │
│    Historical lessons, large code snippets, past decisions   │
│    (Promoted dynamically via Hybrid Vector + BM25 Search)    │
└──────────────────────────────────────────────────────────────┘
```

### Memory Operations API
```rust
// Retrieve all memory blocks for the active agent
let blocks = session.get_memory().await?;

// Search memory using hybrid BM25 + Vector ranking
let results = session.search_memory("authentication token logic", None).await?;
```

---

## 4. RAII Workspace Isolation & Safety Guards

When subagents or automated workflows perform risky filesystem edits, `cade-sdk` integrates `IsolatedWorkspaceGuard`:

- **Worktree Isolation**: Creates an ephemeral git worktree or isolated temporary directory.
- **Zero-Residue Drop**: If an operation errors or is cancelled, the temporary worktree is automatically purged on `Drop` without dirtying the host workspace.
- **Atomic Merge**: On successful task completion, changes are atomically merged back into the target branch with automatic merge conflict detection.
