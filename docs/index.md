# CADE Documentation

Welcome to the CADE documentation. CADE is a local-first Rust coding-AI
assistant with desktop extensions, persistent memory, and a Cargo workspace
of 14 crates.

## Read these first

| Doc | Use when |
|---|---|
| [getting-started.md](getting-started.md) | Installing CADE and running your first session |
| [slash-commands.md](slash-commands.md) | Looking up `/`-prefixed commands |
| [keybindings.md](keybindings.md) | TUI keyboard shortcuts |
| [configuration.md](configuration.md) | Editing `settings.json` or environment variables |

## Architecture & development

| Doc | Use when |
|---|---|
| [architecture.md](architecture.md) | Understanding the workspace and data flow |
| [development.md](development.md) | Setting up a dev environment, building, testing |
| [roadmap.md](roadmap.md) | Reviewing shipped features and what's next |

## Subsystems

| Doc | Use when |
|---|---|
| [memory-system.md](memory-system.md) | Reading or writing memory blocks, debugging consolidation |
| [agents-and-conversations.md](agents-and-conversations.md) | Managing agents, conversations, checkpoints |
| [permissions.md](permissions.md) | Controlling tool approval and protected paths |
| [hooks.md](hooks.md) | Wiring shell scripts into lifecycle events |
| [skills.md](skills.md) | Loading, installing, or authoring skills |
| [subagents.md](subagents.md) | Delegating work to sub-agents |
| [mcp-servers.md](mcp-servers.md) | Adding external MCP tool servers |
| [intelligent-tool-selection.md](intelligent-tool-selection.md) | Tuning tool reranking |
| [cost-and-pricing.md](cost-and-pricing.md) | Capping spend and inspecting token costs |
| [execution-backends.md](execution-backends.md) | Running tools locally, in Docker, or over SSH |
| [themes.md](themes.md) | Customising TUI colour schemes |
| [gui-dashboard.md](gui-dashboard.md) | Using the WASM dashboard |

## Historical material

`docs/history/` contains shipped or partially-shipped plan documents
preserved for archaeology. They do not describe current behaviour.
