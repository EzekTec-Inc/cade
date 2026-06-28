# Skills

A **skill** is a Markdown file (`SKILL.md`) plus optional bundled
resources, packaged together to teach an agent a domain-specific
workflow. CADE skills are inspired by — and partly compatible with —
the Anthropic skill format.

## How skills work

- A skill is loaded **on demand** via the `load_skill` tool.
- Loaded skills become part of the system prompt for that turn and the
  ones that follow until the agent restarts or unloads.
- The static `skills` memory block is **cache-anchored** for prompt
  caching, so loading large skills is cheap on subsequent turns.

## Skill structure

```
skill-name/
├── SKILL.md           (required)
│   ├── YAML frontmatter (name, description)
│   └── Markdown body
├── scripts/           (optional — executable bundled tooling)
└── references/        (optional — lazy-loaded reference docs)
```

`SKILL.md` frontmatter:

```yaml
---
name: skill-name
description: What this skill does. Use when [trigger conditions].
---
```

- `name` — lowercase, hyphenated, gerund-form preferred (e.g. `creating-skills`)
- `description` — leads with action verbs, ends with usage triggers

The body is in **imperative form** ("To accomplish X, do Y"), not second
person.

## Discovery

CADE looks for skills in three locations:

| Scope | Location | Use for |
|---|---|---|
| **Built-in** | `crates/cade-core/src/skills/` | Always available; ship with CADE |
| **Global** | `~/.cade/skills/` | Personal skills shared across projects |
| **Project** | `.cade/skills/` | Per-project skills checked into VCS |

Same-name conflict resolution: project > global > built-in.

## Using skills

### From the CLI

```bash
/skills              # browse all available
/skills <filter>     # filtered search
/<skill_id>          # invoke a loaded skill (e.g. /conventional-commits)
/<skill_id> <prompt> # run a loaded skill with a custom prompt appended to its context block
```

### From the LLM (tool calls)

```
load_skill(id="rust")                # load full content into context
load_skill_ref(skill_id="pptx", doc="editing.md")   # lazy-load a reference
run_skill_script(skill_id="pptx", script="thumbnail")
install_skill(url="https://github.com/...", scope="project")
```

`install_skill` accepts:

- GitHub tree/blob URLs (`github.com/owner/repo/tree/main/path`)
- GitHub shorthand (`owner/repo`)
- Skill registry URLs (`agentskill.sh/@user/skill`)
- Direct `SKILL.md` URLs
- Use the `--skill <name>` selector when a repo contains multiple skills

## Skill blacklist (per agent)

Phase B introduced a per-agent skill blacklist. The agent's system prompt
filters skills out via `render_skills_section_filtered`. Manage it with:

```
POST /v1/agents/:id/skills/disable     { "skill_id": "..." }
POST /v1/agents/:id/skills/enable      { "skill_id": "..." }
GET  /v1/agents/:id/skills             # current effective list
```

Use to silence a noisy or large skill on a specific agent without
uninstalling it globally.

## Authoring guidelines (from `skill-development`)

- **Lean SKILL.md** — keep < 5 k words. Push detail into `references/`.
- **Imperative voice** — "Do X" not "you should do X".
- **Progressive disclosure** — metadata always in context, body when
  triggered, references on demand.
- **Bundled scripts** for deterministic / repeated work.
- **General-purpose only** in shared repositories — no project-specific
  configs.

## Built-in skills (selected)

| Skill | What it teaches |
|---|---|
| `tdd-guide` | Strict red-green-refactor TDD |
| `strict-project-execution` | Minimal-change low-risk workflow + PLAN.md log |
| `software-engineer` | Production-grade Rust (rust10x style) |
| `caveman` | Terse response style |
| `conventional-commits` | Conventional Commit messages |
| `doc-coauthoring` | Structured doc-writing workflow |
| `rust` | Idiomatic Rust patterns |
| `frontend-design` | Distinctive UI design |
| `pdf` `pptx` `docx` | Office-format manipulation |

The full list is visible via `/skills` or as the `skills` memory block.

## Removing a skill

```bash
rm -rf ~/.cade/skills/skill-name        # global
rm -rf .cade/skills/skill-name          # project
```

`/skills` re-discovers on next run; or `/hooks` reload triggers a re-scan
without restart.
