---
id: spec-kit
name: spec-kit
description: Expert in Spec-Driven Development (SDD) using GitHub Spec Kit.
---

# Spec-Driven Development (SDD) Guide

Define what to build before building it. Spec Kit prevents "vibe coding" by requiring structured definition before implementation.

## Setup & Initialization
1. Ensure `uv` is installed (`uv --version`).
2. Install specify-cli: `uv tool install specify-cli --from git+https://github.com/github/spec-kit.git@main`.
3. Verify: `specify version`.
4. Initialize project: `specify init --template default` or `specify init`.

## The 4-Phase SDD Cycle
1.  **Spec (`.spec.md`)**: Define core requirements, user stories, and schemas. No code.
2.  **Plan (`.plan.md`)**: Outline technical implementation, architecture, and constraints.
3.  **Tasks (`.tasks.md`)**: Break down implementation into concrete actionable checkboxes.
4.  **Implement**: Write code conforming strictly to the tasks, keeping a clean diff.

## Community Extensions & Presets
*   **AIDE**: 7-step AI-driven engineering lifecycle presets (`specify preset install aide`).
*   **Canon**: Baseline-driven workflows for spec-first, code-first, and spec-drift cycles.
*   **Product Forge**: PM-oriented SDD definitions.
