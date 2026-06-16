# CADE Usage Guide

Welcome to **CADE** (Coding AI assistant with Desktop Extensions), a stateful, self-improving Rust CLI coding agent that operates fully locally in your terminal. This guide covers how to use CADE effectively for day-to-day software development tasks.

## 1. Getting Started

### Starting CADE
CADE operates on a client-server model.
1. **Start the server:** 
   ```sh
   ANTHROPIC_API_KEY=sk-... cade-server
   ```
2. **Start the interactive CLI client:** 
   ```sh
   cade
   ```

To start a fresh conversation (wiping transient session context), run:
```sh
cade --new
```

To run CADE in read-only mode (great for audits or safe codebase exploration):
```sh
cade --permission-mode plan
```

## 2. Core Concepts

### Agents and Sessions
When you chat with CADE, you are talking to a specific "Agent." Agents have memory, personas, and tracked active goals. By default, CADE restores your last active agent and conversation when you start it in a specific project directory.

### Interactive Terminal (TUI)
CADE provides a rich Terminal User Interface:
- **Chat View:** Your main conversation history.
- **Plan Panel:** Real-time checklist of tasks CADE is currently executing.
- **Status Bar:** Shows current model, cost, tokens, and active tools.

## 3. Essential Slash Commands

CADE includes a powerful command palette. Type `/` in the chat input to see available commands or type them directly:

- `/help` — List all available commands.
- `/mode [plan|build]` — Switch between read-only (`plan`) and full-access (`build`) modes.
- `/model [id]` — Switch the active LLM provider/model (e.g., `anthropic/claude-3-5-sonnet-latest`).
- `/reason [tier]` — Set reasoning effort (e.g., `low`, `high`) for supported models. This setting persists across sessions.
- `/memory` — Open the persistent memory management overlay.
- `/skills` — List and manage loaded skills.
- `/theme list` — Show available UI themes.
- `/theme [name]` — Change the TUI theme dynamically.

## 4. Subagents

CADE can spawn "subagents" to handle complex or long-running tasks in the background without cluttering your main chat history.
- If you ask CADE to "do a deep code review of crate X," it will likely launch a background subagent.
- **Tracker Cards:** Background subagents appear as color-coded cards in the UI (`green` for safe read-only tasks, `yellow` for write tasks).
- Subagents automatically sync their findings back to the main agent when they finish.

## 5. Security & Permissions

CADE executes commands directly on your local machine.
- CADE strictly enforces path sandboxing and tool restrictions. 
- In **Plan Mode**, all destructive operations (writing files, running shell scripts) are strictly blocked.
- You can override or manage MCP server access globally in `~/.cade/settings.json` or locally in your project's `.cade/settings.local.json`.