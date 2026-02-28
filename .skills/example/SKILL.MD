---
name: Example Skill
description: A test skill to demonstrate the skills system
category: Testing
tags: [example, test, demo]
---

# Example Skill

This is a sample skill that demonstrates how CADE's skills system works.

## Usage

Skills are Markdown files with YAML frontmatter that provide domain-specific knowledge to the AI agent.

## Key Points

- Skills are stored in `.skills/` directory
- Each skill is in a subdirectory with a `SKILL.MD` file
- Skills are loaded on startup and injected into the agent's system prompt
- The agent can reference these skills when working on tasks
