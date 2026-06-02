---
id: goa-governance
name: Government of Alberta (GoA) AI Governance and Repository Structure
description: Enforces strict Government of Alberta (GoA) AI-assisted development workflows (Route 1, 2, 3), required status checks, and repository trust-level mapping (approved, collaboration, incubator) for all code-writing, file modifications, and subagent delegations. Always use this skill when working on GoA repositories or projects under the goa-ddd-ai-playbook structure.
---

# GoA AI Governance & Repository Structure Skill

This skill embeds the official Government of Alberta (GoA) AI-assisted development workflows and repository trust-level regulations directly into CADE's system prompt.

---

## 1. GoA AI-Assisted Development Workflows

CADE must align its execution model strictly with GoA's three authorized SDLC routes:

### 🟢 Route 1: Lightweight AI-Assisted Workflow (Non-SDD)
* **Scope**: Best for brownfield projects and quick localized edits.
* **Operational Rules**:
  * **Strict Human-in-the-Loop (HITL)**: CADE is strictly forbidden from executing any mutating tools (such as `write_file`, `edit_file`, or `bash`) under YOLO mode (auto-approval) without explicit developer confirmation of actions *prior to implementation*.
  * **Interactive Planning**: Every task must begin with a clear `set_plan` outlining the business goals, product realization, and step-by-step implementation list.

### 🟡 Route 2: Structured Multi-Agent Workflow (Non-Spec-Kit SDD)
* **Scope**: Complex tasks requiring collaborative agentic delegation.
* **Operational Rules**:
  * **Explicit Agentic Roles**: When spawning subagents via `run_subagent`, CADE must delegate tasks strictly based on explicit functional roles:
    * **Orchestrator**: Decomposes the parent problem statement and assigns subtasks.
    * **Architect**: Reviews codebase state, loads GoA Playbook MCP standards, and verifies the execution plan for internal consistency.
    * **Coder**: Implements specific code units under the architect's constraints.
    * **Tester**: Generates and executes automated unit/integration tests.
    * **Reviewer**: Cross-checks results against original goals and writes detailed Conventional Commit messages.
  * **Context Inheritance**: Every spawned subagent must inherit the parent's core memory blocks (`project`, `persona`, `active_goal`) and recent conversation history to prevent amnesia and hallucinations.

### 🔴 Route 3: Full Spec-Driven Development (SDD) with Spec Kit
* **Scope**: Greenfield initiatives and highly critical, complex refactors.
* **Operational Rules**:
  * **No Direct Coding**: CADE is strictly prohibited from writing code directly from raw natural language prompts.
  * **Spec-First Cycle**: Requirements must first be modeled as formal, version-controlled **Software Design Documents (SDDs)** or Specs inside `.spec.md` or `.sdd.md`. CADE then ingests these specifications to compile and verify code deterministically.

---

## 2. Repository Structure & Trust-Level Mapping

CADE must visually, mechanically, and structurally enforce GoA's repository trust levels to protect authoritative guidelines from pollution:

### 🟢 `approved/` — Authoritative Guidance (Directives Tier)
* **Scope**: Official, GoA-approved architecture playbooks, ADRs, prompts, skills, and hooks.
* **CADE Execution**:
  * Memory blocks loaded from this folder are mapped directly to CADE's **pinned/directives memory tier**, protecting them from token-budget truncation, turning decay, or archival.
  * **Strict Path Protection**: CADE’s permission manager physically blocks any `write_file` or `edit_file` tools on `approved/` files unless authenticated under a secure, signed `Core-Architect` profile.

### 🟡 `collaboration/` — Shared Contribution Space (Short-Term Tier)
* **Scope**: Shared patterns, real-world project examples, and community proposals.
* **CADE Execution**:
  * Mapped to CADE's **short-term memory tier** (active context, subject to 80-turn idle archival).
  * Serves as the primary promotion pathway (`collaboration/` $\rightarrow$ `approved/`) once reviewed and accepted by the Core TI-DDD team.

### ⚪ `incubator/` — Experimental Safety Valve (Long-Term Tier)
* **Scope**: Local spikes, proofs-of-concept, and experimental scripts.
* **CADE Execution**:
  * Mapped to CADE's **long-term (archived) memory tier**. 
  * Excluded from active prompts; represented in system prompts purely as lightweight, 1-line label excerpts unless semantically retrieved.

---

## 3. Mandatory Security & Code Constraints

* **100% Memory Safety**: All Rust crates must compile strictly under `#![forbid(unsafe_code)]`.
* **SQL Query Safety**: String interpolation or `format!` is strictly forbidden for SQL queries; all database accesses must use parameterized `rusqlite` queries.
* **Terminal Escape Sanitization**: All printed outputs must pass through CADE's `sanitize_for_terminal` to strip control characters and prevent terminal escape injections.
* **Local Credentials Protection**: Restrict write tools on sensitive files like `.env`, `api-token`, or `.cade-db.key`.
