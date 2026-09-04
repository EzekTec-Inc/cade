# ADR 15: Multi-Agent Team Coordination and Git Branch Sandboxing

* **Status**: Accepted
* **Decided on**: 2026-07-03

## Context

To solve complex engineering tasks (such as large-scale refactoring, multi-file code reviews, or comprehensive security auditing), we need to allow CADE to coordinate multiple specialized subagents working concurrently. 

Executing multiple subagents in parallel introduces three major challenges:
1. **Orchestration Seam**: Where to schedule and manage multi-agent runs.
2. **Concurrency Safety**: How to prevent parallel subagents from overwriting each other's changes on the host workspace.
3. **Telemetry Pollution**: How to present multiple concurrent streams without cluttering the viewport.

## Decision

We decided to implement a server-orchestrated, git-branch-isolated, and tab-rendered **Team Coordination Engine**:

### 1. Server-Side Stateful `TeamOrchestrator`
We will implement a new, deep module `TeamOrchestrator` inside CADE's server layer (`crates/cade-server/src/server/coordination/`):
* Exposes a single high-leverage entrypoint: `run_team(team_spec: TeamSpecification) -> Result<TeamResult>`.
* Managing concurrency, scheduling, and workspace lifecycle is fully concentrated on the server, keeping the CLI and GUI clients light and simple.

### 2. Ephemeral Git Branch Sandboxing & Merge Resolution
To guarantee absolute concurrency safety during parallel MapReduce executions:
* The `TeamOrchestrator` spawns an isolated git branch for each team member (e.g. `temp_branch_subagent_1`).
* Each member executes write tools strictly inside its own cloned, temporary workspace directory (ADR 6).
* On completion, members push their filesystem changes to their temporary branch, and the orchestrator automatically executes standard `git merge` commands on the host repository.
* If a merge conflict occurs, the orchestrator halts, flags the conflict to the approvals queue, and lets the human resolve it interactively.

### 3. Project-Local Declarative YAML (`teams.yaml`)
To define teams and specialized member templates in an AI-navigable and portable format:
* Teams and members are declared in a project-local `teams.yaml` file (or globally in `~/.cade/teams.yaml`).
* Both developers and CADE agents can dynamically inspect, customize, and check team structures into Git.
* Each member specifies its `name`, target `model`, custom `system_prompt` (role), and `tools` constraints.

### 4. Tabbed / Multi-Column Viewport Telemetry
To prevent parallel streams from polluting the viewport into an unreadable firehose:
* Both `cade-tui` and `cade-gui` will expand their rendering panels to support a tabbed or split-column layout when a Team is active.
* Each running member gets its own independent visual tab (e.g. `[Linter]`, `[Optimizer]`), preserving layout boundaries and letting users monitor details selectively.

## Consequences

### Positive (Pros)
* **High Concurrency Safety**: Leverages git's industrial-grade conflict resolution to merge concurrent workspace writes without silent overwrites.
* **Readable Multi-Streaming**: Tabbed viewports completely eliminate interleaved text clutter.
* **Declarative Portability**: Team definitions are easily checked into repository version control and customized via standard text editors.

### Negative (Cons)
* **Git Repository Prerequisite**: Parallel MapReduce coordination is restricted to git repositories (non-git folders will fall back to serialized execution).
* **Storage Latency**: Cloning multiple workspace temporary folders adds storage overhead and minor initialization latency.
