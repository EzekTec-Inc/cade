# CADE Domain Model & Ubiquitous Language

This file serves as the official, ubiquitous glossary for CADE. It establishes precise definitions for all domain concepts to ensure a shared, unambiguous vocabulary across developers, users, and the agents themselves.

---

## Glossary of Terms

### 1. Agentic Identity & Orchestration

#### Agent
A persistent, named persona with its own configured LLM model, permission settings, active toolset, and dedicated memory registries. An Agent is the root entity for all interactions, configurations, and conversations.

#### Subagent
An independent, sandboxed agent spawned programmatically by a parent Agent to handle isolated tasks (e.g., refactoring, testing, or code review). A Subagent operates in an ephemeral environment, utilizes a filtered subset of tools, and reports only its final answer back to keep the parent's context window clean.

#### Team
A named, collaborative squad of specialized Subagents configured under a specific coordination mode (e.g., Coordinate, Route, Broadcast, Tasks). Teams are executed concurrently to divide and conquer complex tasks.

#### Member
A specialized subagent configuration template assigned to a **Team**. A Member specifies a custom role, tool constraints, model overrides, and a unique system prompt.

---

### 2. Conversational Lifecycle & Versioning

#### Conversation
A single, chronologically ordered thread of messages (prompts, tool calls, and assistant responses) belonging to a specific **Agent**.

#### Checkpoint
A point-in-time snapshot of the project workspace. It captures the exact `git HEAD` commit hash, an optional user-defined label/description, and the specific message in the **Conversation** at which the snapshot was captured.

#### Fork
A new, sibling **Conversation** branched from an existing **Checkpoint**. The Fork initializes the workspace at the exact commit hash captured by the Checkpoint, allowing users or agents to safely explore alternative paths.

---

### 3. Memory & Knowledge Grounding

#### Memory Block
A persistent, key-value fact node surfaced directly to the LLM via the system prompt. Memory blocks are classified into three **Tiers**:
* **Pinned**: Critical, permanent configurations (e.g., `persona`, `human`, `project`) that are never archived.
* **Short-term**: Active task notes and decision nodes that are automatically archived after 80 idle turns.
* **Long-term (Archived)**: Replaced in the active prompt by a label and a short text snippet, fully retrievable on demand.

#### Unified Knowledge Graph
A centralized, database-backed network of structured semantic facts (edges and nodes) shared concurrently by all **Agents** and **Subagents**. It acts as CADE's non-ephemeral grounding memory and supports semantic similarity queries.

#### Consolidation (Compaction)
An automated background process triggered when an Agent's context window utilization nears exhaustion. It compresses older, dropped conversation turns into a summarized form and auto-extracts durable facts to **Memory Blocks**.

#### Artifact
A content-addressed storage entity used to offload extremely large, non-conversational text segments (e.g., compiler logs, massive code diffs, or downloaded reference documents) out of the active context window.

---

### 4. Workspace Safety & Concurrent Guardrails

#### File Lock Manager
A centralized, thread-safe service that coordinates exclusive write locks on absolute file paths, preventing race conditions when multiple parallel **Subagents** attempt to mutate the same source files.

#### Ephemeral Branch Sandboxing (Workspace Isolation)
A workflow isolation strategy where concurrent **Subagents** execute inside cloned, temporary workspace folders. This isolates compilation and file mutation, merging all changes back into the primary workspace as a unified, conflict-resolved merge request.

---

### 5. Client Integration & Human-In-The-Loop

#### Approval Requested Event
An asynchronous Server-Sent Event (SSE) pushed by the server to notify the client instantly that a background **Subagent** has suspended execution and is awaiting user permission on a mutating tool call.

#### /approvals, /approve, /deny
Interactive REPL slash commands that enable the user to view the active pending approvals queue, authorize executing a blocked tool call, or deny it to redirect the subagent's plan.

#### Adaptive Typewriter Governor
A rendering governor in `cade-tui` that evaluates the unrevealed streaming backlog and dynamically scales the character reveal rate up to an instant snap-to-bottom bypass, eliminating latency during large data dumps.

#### Subagent Executor Trait
An asynchronous, trait-based seam in CADE's server that decouples subagent execution, database persistence, and LLM routes from downstream HTTP endpoints, enabling high modularity and mockable testability.

#### Permission Service
An asynchronous, trait-based security seam in `cade-core` that unifies CADE's human-in-the-loop authorization boundaries, delegating prompting and approval actions to pluggable, context-specific adapters.
