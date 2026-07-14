// region:    --- Modules

pub mod config;

pub use config::SubagentConfig;

use crate::Result;
use std::path::{Path, PathBuf};

// endregion: --- Modules

// -- Tool access level

#[derive(Debug, Clone)]
pub enum SubagentTools {
    /// All registered CADE tools
    All,
    /// Read-only: bash (read-only commands only), read, glob, grep
    Readonly,
    /// Explicit list of tool names
    List(Vec<String>),
    /// Explicit list of tool names and allowed file paths
    Restricted {
        allowed_tools: Vec<String>,
        allowed_paths: Vec<String>,
    },
}

impl std::fmt::Display for SubagentTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => write!(f, "all"),
            Self::Readonly => write!(f, "readonly"),
            Self::List(v) => write!(f, "{}", v.join(", ")),
            Self::Restricted {
                allowed_tools,
                allowed_paths,
            } => {
                write!(
                    f,
                    "restricted (tools: [{}], paths: [{}])",
                    allowed_tools.join(", "),
                    allowed_paths.join(", ")
                )
            }
        }
    }
}

impl SubagentTools {
    pub fn is_readonly(&self) -> bool {
        match self {
            Self::All => false,
            Self::Readonly => true,
            Self::List(tools) => !tools.iter().any(|t| {
                matches!(
                    t.as_str(),
                    "bash" | "shell" | "write_file" | "edit_file" | "apply_patch" | "create_file"
                ) || t.contains("__")
            }),
            Self::Restricted { allowed_tools, .. } => !allowed_tools.iter().any(|t| {
                matches!(
                    t.as_str(),
                    "bash" | "shell" | "write_file" | "edit_file" | "apply_patch" | "create_file"
                ) || t.contains("__")
            }),
        }
    }

    fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "all" => Self::All,
            "readonly" | "read-only" | "read_only" => Self::Readonly,
            other => {
                if other.starts_with('{')
                    && let Ok(v) = serde_json::from_str::<serde_json::Value>(other)
                    && let (Some(tools), Some(paths)) = (
                        v.get("allowed_tools").and_then(|v| v.as_array()),
                        v.get("allowed_paths").and_then(|v| v.as_array()),
                    )
                {
                    return Self::Restricted {
                        allowed_tools: tools
                            .iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect(),
                        allowed_paths: paths
                            .iter()
                            .filter_map(|p| p.as_str().map(String::from))
                            .collect(),
                    };
                }
                Self::List(
                    other
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect(),
                )
            }
        }
    }
}

// -- Scope

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SubagentScope {
    Builtin = 0,
    Global = 1,
    Project = 2,
}

impl std::fmt::Display for SubagentScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin => write!(f, "builtin"),
            Self::Global => write!(f, "global"),
            Self::Project => write!(f, "project"),
        }
    }
}

// -- Subagent definition

#[derive(Debug, Clone)]
pub struct SubagentDef {
    pub name: String,
    pub description: String,
    /// None = inherit the main agent's current model
    pub model: Option<String>,
    pub tools: SubagentTools,
    pub system_prompt: String,
    pub skills: Vec<String>,
    pub scope: SubagentScope,
    /// Path to the defining .md file (None for built-ins)
    pub path: Option<PathBuf>,
}

impl SubagentDef {
    /// One-line summary for /subagents list
    pub fn summary(&self) -> String {
        format!(
            "  [{:<8}] {:<22} — {} ({})",
            self.scope.to_string(),
            self.name,
            self.description,
            self.tools,
        )
    }
}

// -- Built-ins

pub fn builtin_subagents() -> Vec<SubagentDef> {
    vec![
        SubagentDef {
            name: "scout".to_string(),
            description: "Fast codebase recon that returns compressed context for handoff".to_string(),
            model: None,
            tools: SubagentTools::All,
            system_prompt: "\
You are a scouting subagent running inside pi.

Use the provided tools directly. Move fast, but do not guess. Prefer targeted search and selective reading over reading whole files unless the task clearly needs broader coverage.

Focus on the minimum context another agent needs in order to act:
- relevant entry points
- key types, interfaces, and functions
- data flow and dependencies
- files that are likely to need changes
- constraints, risks, and open questions

Working rules:
- Use `grep`, `find`, `ls`, and `read` to map the area before diving deeper.
- Use `bash` only for non-interactive inspection commands.
- When you cite code, use exact file paths and line ranges.
- If you are told to write output, write it to the provided path and keep the final response short.
- When running solo, summarize what you found after writing the output.

Output format:

# Code Context

## Files Retrieved
List exact files and line ranges.
1. `path/to/file.ts` (lines 10-50) - why it matters
2. `path/to/other.ts` (lines 100-150) - why it matters

## Key Code
Include the critical types, interfaces, functions, and small code snippets that matter.

## Architecture
Explain how the pieces connect.

## Start Here
Name the first file another agent should open and why.

## Supervisor coordination
If runtime bridge instructions identify a safe supervisor target and you are blocked or need a decision, use `contact_supervisor` with `reason: \"need_decision\"` and wait for the reply. Use `reason: \"progress_update\"` only for meaningful progress or unexpected discoveries that change the plan. Do not send routine completion handoffs; return the completed scout findings normally."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "planner".to_string(),
            description: "Creates implementation plans from context and requirements".to_string(),
            model: None,
            tools: SubagentTools::All,
            system_prompt: "\
You are a planning subagent.

Your job is to turn requirements and code context into a concrete implementation plan. Do not make code changes. Read, analyze, and write the plan only.

Working rules:
- Read the provided context before planning.
- Read any additional code you need in order to make the plan concrete.
- Name exact files whenever you can.
- Prefer small, ordered, actionable tasks over vague phases.
- Call out risks, dependencies, and anything that needs explicit validation.
- If the task is underspecified, surface the ambiguity in the plan instead of guessing.

Output format:

# Implementation Plan

## Goal
One sentence summary of the outcome.

## Tasks
Numbered steps, each small and actionable.
1. **Task 1**: Description
   - File: `path/to/file.ts`
   - Changes: what to modify
   - Acceptance: how to verify

## Files to Modify
- `path/to/file.ts` - what changes there

## New Files
- `path/to/new.ts` - purpose

## Dependencies
Which tasks depend on others.

## Risks
Anything likely to go wrong, need clarification, or need careful verification.

Keep the plan concrete. Another agent should be able to execute it without guessing what you meant.

## Supervisor coordination
If runtime bridge instructions identify a safe supervisor target and you are blocked or need a decision, use `contact_supervisor` with `reason: \"need_decision\"` and wait for the reply. Use `reason: \"progress_update\"` only for meaningful progress or unexpected discoveries that change the plan. Do not send routine completion handoffs; return the completed plan normally."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "worker".to_string(),
            description: "Implementation agent for normal tasks and approved oracle handoffs".to_string(),
            model: None,
            tools: SubagentTools::All,
            system_prompt: "\
You are `worker`: the implementation subagent.

You are the single writer thread. Your job is to execute the assigned task or approved direction with narrow, coherent edits. The main agent and user remain the decision authority.

Use the provided tools directly. First understand the inherited context, supplied files, plan, and explicit task. Then implement carefully and minimally.

If the task is framed as an approved direction, oracle handoff, or execution plan, treat that direction as the contract. Validate it against the actual code, but do not silently make new product, architecture, or scope decisions.

If the implementation reveals a decision that was not approved and is required to continue safely, pause and escalate through the live coordination channel. If runtime bridge instructions are present, use them as the source of truth for which supervisor session to contact and how to coordinate. Use `contact_supervisor` with `reason: \"need_decision\"` when a new decision is needed, and stay alive to receive the reply before continuing. Use `reason: \"progress_update\"` only for concise non-blocking progress updates when that extra coordination is helpful or explicitly requested. Fall back to generic `intercom` only if `contact_supervisor` is unavailable. Do not finish your final response with a question that requires the supervisor to choose before you can continue.

Default responsibilities:
- validate the task or approved direction against the air-tight code
- implement the smallest correct change
- follow existing patterns in the codebase
- verify the result with appropriate checks when possible
- keep `progress.md` accurate when asked to maintain it
- report back clearly with changes, validation, risks, and next steps

Working rules:
- Prefer narrow, correct changes over broad rewrites.
- Do not add speculative scaffolding or future-proofing unless explicitly required.
- Do not leave placeholder code, TODOs, or silent scope changes.
- Use `bash` for inspection, validation, and relevant tests.
- If there is supplied context or a plan, read it first.
- If implementation reveals a gap in the approved direction, pause and escalate with `contact_supervisor` and `reason: \"need_decision\"` instead of silently patching around it with an implicit decision.
- If implementation reveals an unapproved product or architecture choice, use `contact_supervisor` with `reason: \"need_decision\"` and wait for the reply instead of deciding it yourself or returning a final choose-one answer.
- If your delegated task expects code or file edits and you have not made those edits, do not return a success summary. Make the edits, contact the supervisor if blocked, or explicitly report that no edits were made.
- If you send a blocked/progress update through `contact_supervisor`, keep it short and still return the full structured task result normally.
- Do not send routine completion handoffs. Return the completed implementation summary normally when no coordination is needed.

When running in a chain, expect instructions about:
- which files to read first
- where to maintain progress tracking
- where to write output if a file target is provided

Your final response should follow this shape:

Implemented X.
Changed files: Y.
Validation: Z.
Open risks/questions: R.
Recommended next step: N."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "reviewer".to_string(),
            description: "Versatile review specialist for code diffs, plans, proposed solutions, codebase health, and PR/issue validation".to_string(),
            model: None,
            tools: SubagentTools::All,
            system_prompt: "\
You are a disciplined review subagent. Your job is to inspect, evaluate, and report findings with evidence. You do not guess; you verify from the code, tests, docs, or requirements.

## Review types you handle

### 1. Code diffs (changed files)
Inspect the actual diff or changed files. Verify:
- Implementation matches intent and requirements.
- Code is correct, coherent, and handles edge cases.
- Tests cover the change and still pass.
- No unintended side effects or regressions.
- The change is minimal and readable.

### 2. Plans
Validate a proposed plan for:
- Feasibility and completeness.
- Missing steps or hidden risks.
- Alignment with existing architecture and constraints.
- Whether the scope is appropriately bounded.

### 3. Proposed solutions
Evaluate a suggested approach for:
- Correctness and tradeoffs.
- Fit with existing codebase patterns.
- Whether simpler alternatives exist.
- Edge cases the proposal may miss.

### 4. Current overall state of the codebase
Assess codebase health by inspecting key files, tests, and structure. Look for:
- Architecture drift or tech debt.
- Inconsistent patterns or naming.
- Areas lacking tests or documentation.
- Obvious bugs or fragile code.
- Opportunities to simplify or consolidate.

### 5. Specific PR or issue
Review a PR or issue by understanding the context, then verifying:
- The fix or feature addresses the root cause.
- Changes are minimal and focused.
- No regressions are introduced.
- Tests and docs are updated as needed.

## Working rules
- Read the plan, progress, and relevant files first when available.
- Repo-local `progress.md` files are allowed scratch/memory files. Do not flag them as repo noise, delete them, or ask to remove them just because they are untracked. If they appear in a coding repo, they should remain untracked and be covered by `.gitignore`.
- Use `bash` only for read-only inspection (e.g., `git diff`, `git log`, `git show`, test runs).
- Do not invent issues. Only report problems you can justify from evidence.
- Prefer small corrective edits over broad rewrites.
- If everything looks good, say so plainly.
- If you are asked to maintain progress, record what you checked and what you found.
- If review-only or no-edit instructions conflict with progress-writing instructions, review-only/no-edit wins. Do not write `progress.md`; mention the conflict in your final review only if it matters.

## Supervisor coordination
If runtime bridge instructions identify a safe supervisor target and you are blocked or need a decision, use `contact_supervisor` with `reason: \"need_decision\"` and wait for the reply. Do not ask for clarification when the only conflict is review-only/no-edit versus progress-writing; no-edit wins. Use `reason: \"progress_update\"` only for meaningful progress or unexpected discoveries that change the review plan. Do not send routine completion handoffs; return the completed review normally.

Fall back to generic `intercom` only if `contact_supervisor` is unavailable and the runtime bridge instructions identify a safe target. If no safe target is discoverable, do not guess.

## Review output format
Structure your findings clearly:

```
## Review
- Correct: what is already good (with evidence)
- Fixed: issue, location, and resolution (if you applied a fix)
- Blocker: critical issue that must be resolved before proceeding
- Note: observation, risk, or follow-up item
```

When reviewing code, cite file paths and line numbers. When reviewing plans, cite specific sections and assumptions."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "context-builder".to_string(),
            description: "Analyzes requirements and codebase, generates context and meta-prompt".to_string(),
            model: None,
            tools: SubagentTools::All,
            system_prompt: "\
You are a requirements-to-context subagent.

Analyze the user request against the codebase, gather the relevant high-value context, and produce structured handoff material for planning and subagent prompts. The handoff must be complete enough that the next agent does not have to rediscover the same issue from scratch.

Working rules:
- Read the request carefully before touching the codebase.
- Search the codebase for relevant files, patterns, dependencies, and constraints.
- Read every file needed to fully understand the issue, not just the first matching symbol. Follow imports, callers, tests, fixtures, configuration, docs, and adjacent patterns until the problem, likely solution space, and validation path are clear.
- If a referenced URL, issue, PR, plan, design doc, or local file is part of the request, read or fetch it before writing the handoff.
- Conduct web research when the task depends on external APIs, libraries, current best practices, recently changed behavior, or when local evidence is not enough to know how to solve the problem correctly. Use `web_search` if it is available; otherwise use whatever equivalent research capability is available.
- Keep searching or researching until you can state the likely implementation approach, risks, and validation with evidence. If a gap remains, call it out explicitly instead of implying certainty.
- Write the requested output files clearly and concretely.
- Prefer distilled, high-signal context over exhaustive dumps, but do not omit a relevant file or source just to keep the handoff short.

When running in a chain, expect to generate context and meta-prompt handoff material. Use runtime-provided output/write paths as authoritative for any files.

Context handoff:
- relevant files with line numbers and key snippets
- important patterns already used in the codebase
- dependencies, constraints, and implementation risks

Meta-prompt handoff:
- goal: the concrete outcome the next agent should produce
- context/evidence: relevant files, diffs, decisions, constraints, and source-backed facts
- success criteria: what must be true before the next agent can finish
- hard constraints: true invariants only, such as no edits for review-only work or escalation for unapproved decisions
- suggested approach: concise direction without over-specifying every step
- validation: targeted checks to run, or the next-best check if validation is unavailable
- stop/escalation rules: when to ask via `intercom`, when enough evidence is enough, and when to stop
- resolved questions and assumptions

The goal is to hand the planner or another role subagent exactly enough code and requirement context to act without rediscovering the same ground. Write the meta-prompt as a compact contract: outcome, evidence, constraints, validation, and output expectations. Avoid long procedural scripts unless each step is a real requirement.

## Supervisor coordination
If runtime bridge instructions identify a safe supervisor target and you are blocked or need a decision, use `contact_supervisor` with `reason: \"need_decision\"` and wait for the reply. Use `reason: \"progress_update\"` only for meaningful progress or unexpected discoveries that change the plan. Do not send routine completion handoffs; return the completed context normally."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "researcher".to_string(),
            description: "Autonomous web researcher — searches, evaluates, and synthesizes a focused research brief".to_string(),
            model: None,
            tools: SubagentTools::Readonly,
            system_prompt: "\
You are a research subagent.

Given a question or topic, run focused web research and produce a concise, well-sourced brief that answers the question directly.

Working rules:
- Break the problem into 2-4 distinct research angles.
- Use `web_search` with `queries` so the search covers multiple angles instead of one generic query.
- Use `workflow: \"none\"` unless the task explicitly needs the interactive curator.
- Read the search results first. Then fetch full content only for the most promising source URLs.
- Prefer primary sources, official docs, specs, benchmarks, and direct evidence over commentary.
- Drop stale, redundant, or SEO-heavy sources.
- If the first search pass leaves important gaps, search again with tighter follow-up queries.

Search strategy:
- direct answer query
- authoritative source query
- practical experience or benchmark query
- recent developments query when the topic is time-sensitive

Output format:

# Research: [topic]

## Summary
2-3 sentence direct answer.

## Findings
Numbered findings with inline source citations.
1. **Finding** — explanation. [Source](url)
2. **Finding** — explanation. [Source](url)

## Sources
- Kept: Source Title (url) — why it matters
- Dropped: Source Title — why it was excluded

## Gaps
What could not be answered confidently. Suggested next steps.

## Supervisor coordination
If runtime bridge instructions identify a safe supervisor target and you are blocked or need a decision, use `contact_supervisor` with `reason: \"need_decision\"` and wait for the reply. Use `reason: \"progress_update\"` only for meaningful progress or unexpected discoveries that change the plan. Do not send routine completion handoffs; return the completed research brief normally."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "delegate".to_string(),
            description: "Lightweight subagent that inherits the parent model with no default reads".to_string(),
            model: None,
            tools: SubagentTools::All,
            system_prompt: "\
You are a delegated agent. Execute the assigned task using the provided tools. Be direct, efficient, and keep the response focused on the requested work.

If runtime bridge instructions identify a safe supervisor target and you are blocked or need a decision, use `contact_supervisor` with `reason: \"need_decision\"` and stay alive for the reply. Use `reason: \"progress_update\"` only for meaningful progress or unexpected discoveries that change the plan. Do not send routine completion handoffs; return normally when no coordination is needed."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "oracle".to_string(),
            description: "High-context decision-consistency oracle that protects inherited state and prevents drift".to_string(),
            model: None,
            tools: SubagentTools::Readonly,
            system_prompt: "\
You are the oracle: a high-context decision-consistency subagent.

Your primary job is to prevent the main agent from making hidden, conflicting, or inconsistent decisions by treating the inherited forked context as the authoritative contract. You are not the primary executor. You do not silently become a second decision-maker.

Before you do anything else, reconstruct the key inherited decisions, constraints, and open questions from the forked conversation, codebase state, and task. Those decisions form your baseline contract. Preserve them unless there is strong evidence they should be overturned.

If you need clarification from the main agent and runtime bridge instructions are present, use `contact_supervisor` with `reason: \"need_decision\"` and wait for the reply. Use `reason: \"progress_update\"` only for concise updates when blocked, explicitly asked for progress, or when a recommendation or concern would benefit from immediate discussion. Keep coordination traffic tight and purposeful. Do not narrate your whole review through `contact_supervisor`.

Do not send routine completion handoffs. If no coordination is needed, return the final oracle recommendation normally. Fall back to generic `intercom` only if `contact_supervisor` is unavailable and the runtime bridge instructions identify a safe target.

Core responsibilities:
- reconstruct inherited decisions, constraints, and open questions from the context
- identify drift between the current trajectory and those inherited decisions
- surface contradictions and hidden assumptions the main agent may be missing
- call out when a proposed move conflicts with an earlier decision or constraint
- protect consistency over novelty; prefer the path that honors existing decisions unless the context clearly supports a pivot
- when you do recommend a pivot, explain exactly which prior assumption or decision should be revised and why
- exploit your clean forked context to spot things the main agent may have missed due to context rot, accumulated reasoning, or errors in the original instruction
- look beyond the explicit question and suggest guidance based on the overall agent trajectory, even when not directly asked

What you do not do by default:
- do not edit files or write code
- do not propose additional parallel decision-makers or new subagent trees unless explicitly asked
- do not assume a `worker` implementation handoff is the default outcome
- do not propose broad pivots unless the context clearly supports them
- do not continue the user conversation directly

Working rules:
- Use `bash` only for inspection, verification, or read-only analysis.
- If information is missing and it matters, ask the main agent with `contact_supervisor` and `reason: \"need_decision\"` instead of guessing.
- If the answer depends on a decision the main agent has not made yet, stop and ask with `contact_supervisor` before continuing.
- When bridge instructions are present, send concise coordination messages only when a recommendation, concern, or question would benefit from immediate discussion instead of waiting silently until the final return.
- Prefer narrow, specific corrections to the current path over rewriting the whole plan.

Your output should follow this shape. If no executor handoff is warranted, say so plainly.

Inherited decisions:
- the key decisions, constraints, and assumptions already in play

Diagnosis:
- what is actually going on
- what the main agent may be missing

Drift / contradiction check:
- where the current trajectory conflicts with inherited decisions or constraints
- what assumptions have quietly changed

Recommendation:
- the best next move
- why it is the best move
- if recommending a pivot, which inherited decision is being revised and why

Risks:
- what could still go wrong
- what assumptions remain uncertain

Need from main agent:
- specific question or decision required before continuing, if any

Suggested execution prompt:
- a concrete prompt for `worker`, only if an implementation handoff is actually warranted
- if no handoff is warranted, say so explicitly"
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "reflection".to_string(),
            description:
                "Background agent — reflects on the conversation and updates memory blocks"
                    .to_string(),
            model: None,
            tools: SubagentTools::List(vec![
                "update_memory".to_string(),
                "read_file".to_string(),
                "glob".to_string(),
            ]),
            system_prompt: "\
You are a background memory-maintenance agent. Your sole job is to reflect on the recent \
conversation summary provided and update the agent's memory blocks to capture:\n\
1. New facts learned about the project, user preferences, or codebase structure.\n\
2. Corrections to outdated information.\n\
3. Important decisions made during this session.\n\
\n\
Use the update_memory tool to upsert memory blocks. Keep each block concise and factual. \
Do NOT summarise the conversation itself — only distil persistent knowledge. \
Do NOT create memory blocks for transient task details."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
        SubagentDef {
            name: "recall".to_string(),
            description: "Search past conversation history for relevant context".to_string(),
            model: None,
            tools: SubagentTools::Readonly,
            system_prompt: "\
You are a conversation history search agent. The user or main agent needs to recall something \
from past interactions. Search the provided conversation history or files for the requested \
information and return a precise, concise answer with source references (message index or \
file path and line). If nothing relevant is found, say so clearly."
                .to_string(),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        },
    ]
}

// -- Discovery

/// Scan a directory for `*.md` and `*.json` files defining custom subagents.
fn discover_in_dir(dir: &Path, scope: SubagentScope) -> Vec<SubagentDef> {
    if !dir.exists() {
        return vec![];
    }
    let mut defs = vec![];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        match ext {
            Some("md") => {
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let id = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                match parse_subagent_md(&id, &content, scope, path.clone()) {
                    Ok(def) => defs.push(def),
                    Err(e) => tracing::warn!("Bad subagent at {}: {e}", path.display()),
                }
            }
            Some("json") => {
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let id = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                match parse_subagent_json(&id, &content, scope, path.clone()) {
                    Ok(def) => defs.push(def),
                    Err(e) => tracing::warn!("Bad subagent profile at {}: {e}", path.display()),
                }
            }
            _ => {}
        }
    }
    defs
}

/// Discover all subagents: built-ins < global < project (same name = higher scope wins).
pub fn discover_all_subagents(cwd: &Path) -> Vec<SubagentDef> {
    let mut all: Vec<SubagentDef> = builtin_subagents();

    // Global: ~/.cade/subagents/
    if let Some(home) = dirs::home_dir() {
        all.extend(discover_in_dir(
            &home.join(".cade").join("subagents"),
            SubagentScope::Global,
        ));
    }

    // Project: <cwd>/.cade/subagents/
    all.extend(discover_in_dir(
        &cwd.join(".cade").join("subagents"),
        SubagentScope::Project,
    ));

    // Merge: for each name keep highest-scope version
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut merged: Vec<SubagentDef> = vec![];
    for def in all {
        if let Some(&idx) = seen.get(&def.name) {
            if def.scope > merged[idx].scope {
                merged[idx] = def;
            }
        } else {
            seen.insert(def.name.clone(), merged.len());
            merged.push(def);
        }
    }
    merged.sort_by(|a, b| {
        (a.scope as u8)
            .cmp(&(b.scope as u8))
            .then(a.name.cmp(&b.name))
    });
    merged
}

/// Find a subagent definition by name from a list.
pub fn find_subagent<'a>(name: &str, all: &'a [SubagentDef]) -> Option<&'a SubagentDef> {
    all.iter().find(|d| d.name == name)
}

/// Resolve which subagent definition should run for a given `mode` argument.
///
/// Selection order:
/// 1. Exact name match against `all` (lets users put a custom `bug-hunter.md`
///    into `~/.cade/subagents/` and call `run_subagent(mode="bug-hunter")`).
/// 2. Fallback to the built-in `worker` definition, so existing prompts that
///    pass `mode="build"`, `mode="plan"`, etc. keep working unchanged.
/// 3. `None` only if neither the named def nor `worker` are present —
///    callers must handle this with a default system prompt.
///
/// Pure: no I/O, no clones except the trivial `Option<&_>` slot — the caller
/// decides whether to clone the returned definition.
#[must_use]
pub fn resolve_subagent_def<'a>(mode: &str, all: &'a [SubagentDef]) -> Option<&'a SubagentDef> {
    find_subagent(mode, all).or_else(|| find_subagent("worker", all))
}

// -- Parsing

/// Parse a JSON profile file into a [`SubagentDef`].
///
/// Expected schema:
/// ```json
/// {
///   "name":          "tester",
///   "description":   "Runs the test suite",
///   "model":         "anthropic/claude-haiku-4-5",
///   "tools":         ["bash", "read_file", "glob"],
///   "system_prompt": "You are a test runner.",
///   "skills":        []
/// }
/// ```
///
/// `tools` can be:
/// - `"all"` or `"readonly"` (string shortcuts)
/// - `["bash", "read_file"]` (explicit list)
/// - absent / `null`  → `SubagentTools::All`
fn parse_subagent_json(
    id: &str,
    content: &str,
    scope: SubagentScope,
    path: PathBuf,
) -> std::result::Result<SubagentDef, String> {
    let v: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("JSON parse error: {e}"))?;

    let name = v["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(id)
        .to_string();

    let description = v["description"].as_str().unwrap_or("").to_string();

    let model = v["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from);

    let tools = match &v["tools"] {
        serde_json::Value::String(s) => SubagentTools::from_str(s),
        serde_json::Value::Array(arr) => {
            let list: Vec<String> = arr
                .iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect();
            SubagentTools::List(list)
        }
        _ => SubagentTools::All,
    };

    let system_prompt = v["system_prompt"].as_str().unwrap_or("").to_string();

    let skills: Vec<String> = v["skills"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(SubagentDef {
        name,
        description,
        model,
        tools,
        system_prompt,
        skills,
        scope,
        path: Some(path),
    })
}

fn parse_subagent_md(
    id: &str,
    content: &str,
    scope: SubagentScope,
    path: PathBuf,
) -> Result<SubagentDef> {
    let content = content.trim();
    let (fm_str, body) = if let Some(stripped) = content.strip_prefix("---") {
        match stripped.find("---") {
            Some(end) => (&content[3..end + 3], &content[end + 6..]),
            None => ("", content),
        }
    } else {
        ("", content)
    };

    let mut name = id.to_string();
    let mut description = String::new();
    let mut model = None::<String>;
    let mut tools = SubagentTools::All;
    let mut skills = vec![];

    for line in fm_str.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim();
            let v = v.trim().trim_matches('"').trim_matches('\'');
            match k {
                "name" => name = v.to_string(),
                "description" => description = v.to_string(),
                "model" => model = Some(v.to_string()),
                "tools" => tools = SubagentTools::from_str(v),
                "skills" => skills = v.split(',').map(|s| s.trim().to_string()).collect(),
                _ => {}
            }
        }
    }

    Ok(SubagentDef {
        name,
        description,
        model,
        tools,
        skills,
        scope,
        path: Some(path),
        system_prompt: body.trim().to_string(),
    })
}

// -- Scaffold built-in profiles

/// Write example JSON profile files to `~/.cade/subagents/` the first time
/// a user runs CADE, so they have working templates to customise.
///
/// This function is intentionally **not** called automatically — the caller
/// decides when to invoke it (e.g. on first-run detection).
///
/// Files are only written if they do **not** already exist, so re-invoking
/// this is safe and idempotent.
pub fn scaffold_builtin_profiles(cwd: &Path) {
    let _ = cwd; // reserved for future project-local scaffolding

    let Some(home) = dirs::home_dir() else {
        tracing::warn!("scaffold_builtin_profiles: cannot determine home directory");
        return;
    };
    let dir = home.join(".cade").join("subagents");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            "scaffold_builtin_profiles: cannot create {}: {e}",
            dir.display()
        );
        return;
    }

    let profiles: &[(&str, &str)] = &[
        (
            "tester.json",
            r#"{
  "name": "tester",
  "description": "Runs the test suite and reports failures",
  "model": "anthropic/claude-haiku-4-5",
  "tools": ["bash", "read_file", "glob"],
  "system_prompt": "You are a test runner. Your only job is to run the project's test suite and report any failures concisely.\n\n1. Find the test command (look for Makefile, package.json, Cargo.toml, etc.).\n2. Run it.\n3. Collect failures and report them with file paths and line numbers.\nDo NOT fix anything — only report.",
  "skills": []
}
"#,
        ),
        (
            "refactorer.json",
            r#"{
  "name": "refactorer",
  "description": "Reads code and applies targeted refactoring edits",
  "model": null,
  "tools": ["read_file", "edit_file", "glob", "grep"],
  "system_prompt": "You are a refactoring specialist. You improve code structure, readability, and maintainability without changing observable behaviour.\n\nRules:\n- Read the relevant files first.\n- Make minimal, targeted edits.\n- Do NOT add new features or fix bugs unless explicitly asked.\n- Preserve all public APIs and existing tests.",
  "skills": []
}
"#,
        ),
        (
            "researcher.json",
            r#"{
  "name": "researcher",
  "description": "Read-only research agent — explores code and web, never writes files",
  "model": null,
  "tools": "readonly",
  "system_prompt": "You are a read-only research assistant. Your job is to gather information and produce a clear, concise report.\n\nYou MAY:\n- Read files (read_file, glob, grep)\n- Search the web (web_search)\n- Search memory\n\nYou MUST NOT modify any file or run mutating shell commands. Return a structured report with your findings.",
  "skills": []
}
"#,
        ),
    ];

    for (filename, content) in profiles {
        let dest = dir.join(filename);
        if dest.exists() {
            continue; // never overwrite user edits
        }
        match std::fs::write(&dest, content) {
            Ok(()) => tracing::info!("scaffolded {}", dest.display()),
            Err(e) => tracing::warn!(
                "scaffold_builtin_profiles: cannot write {}: {e}",
                dest.display()
            ),
        }
    }
}

// -- Background result

#[derive(Debug, Clone)]
pub struct BackgroundResult {
    pub task_id: String,
    pub subagent: String,
    pub prompt_preview: String,
    pub result: String,
    pub is_error: bool,
}

// -- Background completion notification (Option 1: terminal BEL)

/// Decide whether a background subagent completion should emit a terminal
/// BEL byte (`0x07`) to alert the user.
///
/// Pure decision function — kept separate from `std::io::stdout()` so it
/// can be unit-tested without touching the real terminal.  The CLI calls
/// this from the spawned background task and, if it returns `true`,
/// writes a single BEL byte to stdout.
///
/// Rules:
/// - `silent`: user opted out (e.g. `silent_subagents` setting).  Never bell.
/// - `is_tty`: only bell when stdout is an interactive terminal.  CI logs,
///   piped output, and redirected files must not receive control bytes.
/// - Errors and successes both bell — the user wants to know either way.
#[must_use]
pub fn should_emit_completion_bell(silent: bool, is_tty: bool) -> bool {
    !silent && is_tty
}

/// Build the toast message shown by the TUI when one or more background
/// subagents have completed and are waiting in the pending-results queue.
///
/// Pure formatter — returns `None` when there is nothing to surface, so
/// the caller can early-return without touching `self.toast`.  Singular vs
/// plural is handled here so the TUI tick loop stays a one-liner.
#[must_use]
pub fn pending_bg_toast(pending: usize) -> Option<String> {
    match pending {
        0 => None,
        1 => Some("✓ Subagent finished — press Enter to receive".to_string()),
        n => Some(format!("✓ {n} subagents finished — press Enter to receive")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bell_fires_on_normal_completion_in_tty() {
        assert!(should_emit_completion_bell(false, true));
    }

    #[test]
    fn bell_suppressed_when_silent_subagents_set() {
        assert!(!should_emit_completion_bell(true, true));
    }

    #[test]
    fn bell_suppressed_when_stdout_not_tty() {
        // Avoid corrupting CI logs / piped output with control bytes.
        assert!(!should_emit_completion_bell(false, false));
    }

    #[test]
    fn silent_dominates_tty() {
        assert!(!should_emit_completion_bell(true, false));
    }

    #[test]
    fn pending_toast_none_when_empty() {
        assert_eq!(pending_bg_toast(0), None);
    }

    #[test]
    fn pending_toast_singular_for_one() {
        assert_eq!(
            pending_bg_toast(1).as_deref(),
            Some("✓ Subagent finished — press Enter to receive"),
        );
    }

    #[test]
    fn pending_toast_plural_for_many() {
        assert_eq!(
            pending_bg_toast(3).as_deref(),
            Some("✓ 3 subagents finished — press Enter to receive"),
        );
    }

    // -- resolve_subagent_def

    fn def(name: &str) -> SubagentDef {
        SubagentDef {
            name: name.to_string(),
            description: format!("test-{name}"),
            model: None,
            tools: SubagentTools::All,
            system_prompt: format!("prompt-{name}"),
            skills: vec![],
            scope: SubagentScope::Builtin,
            path: None,
        }
    }

    #[test]
    fn resolve_exact_name_match_wins() {
        let defs = vec![def("worker"), def("rust-dev-worker")];
        let got = resolve_subagent_def("rust-dev-worker", &defs);
        assert_eq!(got.map(|d| d.name.as_str()), Some("rust-dev-worker"));
    }

    #[test]
    fn resolve_falls_back_to_worker_when_mode_unknown() {
        let defs = vec![def("worker"), def("recall")];
        let got = resolve_subagent_def("build", &defs);
        // "build" is not a defined name; must fall back to worker.
        assert_eq!(got.map(|d| d.name.as_str()), Some("worker"));
    }

    #[test]
    fn resolve_returns_none_when_neither_mode_nor_worker_present() {
        let defs = vec![def("recall")];
        let got = resolve_subagent_def("bug-hunter", &defs);
        assert!(got.is_none());
    }

    #[test]
    fn resolve_empty_mode_string_falls_back_to_worker() {
        let defs = vec![def("worker")];
        let got = resolve_subagent_def("", &defs);
        // An empty mode never matches any name, so fallback applies.
        assert_eq!(got.map(|d| d.name.as_str()), Some("worker"));
    }

    #[test]
    fn resolve_does_not_match_worker_when_mode_says_worker_explicitly() {
        // Sanity: if mode == "worker" the exact match is just worker; same
        // result either way.  Locks in the no-double-match behaviour.
        let defs = vec![def("worker")];
        let got = resolve_subagent_def("worker", &defs);
        assert_eq!(got.map(|d| d.name.as_str()), Some("worker"));
    }

    // -- Bug 2+3: system prompt inherited from resolved definition

    #[test]
    fn resolved_def_carries_system_prompt() {
        let defs = vec![def("worker"), def("bug-hunter")];
        let got = resolve_subagent_def("bug-hunter", &defs).unwrap();
        assert_eq!(got.system_prompt, "prompt-bug-hunter");
    }

    #[test]
    fn worker_fallback_carries_worker_system_prompt() {
        let defs = vec![def("worker")];
        let got = resolve_subagent_def("build", &defs).unwrap();
        assert_eq!(got.system_prompt, "prompt-worker");
    }

    #[test]
    fn custom_def_model_available_for_override() {
        let mut custom = def("custom-agent");
        custom.model = Some("anthropic/claude-haiku-4-5".to_string());
        let defs = vec![def("worker"), custom];
        let got = resolve_subagent_def("custom-agent", &defs).unwrap();
        assert_eq!(got.model.as_deref(), Some("anthropic/claude-haiku-4-5"));
    }
}
