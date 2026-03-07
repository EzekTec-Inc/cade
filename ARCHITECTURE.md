# Architecture of CADE

CADE (Coding AI with Desktop Extensions) is a helpful coding AI agent that's skilled beyond just coding, but skilled in keeping the developer efficient and organized while building digital solutions.

## Core Components

*   **`src/main.rs`**: Entry point for the `cade` CLI application.
*   **`src/bin/cade-server.rs`**: Entry point for the `cade-server` backend.
*   **`src/lib.rs`**: Defines shared modules.

## CLI (`cade`)

### Modules

*   **`src/cli/repl.rs`**: Implements the interactive REPL (Read-Eval-Print Loop) for CADE.
*   **`src/cli/headless.rs`**: Implements headless mode for running CADE from the command line without a UI.
*   **`src/cli/args.rs`**: Defines command-line arguments using `clap`.
*   **`src/cli/export_import.rs`**: Handles exporting and importing agent configurations and data.

### UI

*   **`src/ui/app.rs`**: Main application logic for the terminal UI using `ratatui`.
*   **`src/ui/output.rs`**: Renders CADE's output to the terminal.
*   **`src/ui/markdown.rs`**: Handles markdown rendering in the UI.
*   **`src/ui/input.rs`**: Manages user input and command handling.
*   **`src/ui/question.rs`**: Implements interactive question prompts in the UI.
*   **`src/ui/status.rs`**: Manages the status bar at the bottom of the terminal.
*   **`src/ui/menu.rs`**: Implements menuing system for the UI.

### Agent

*   **`src/agent/client.rs`**: Defines the `CadeClient` for interacting with the `cade-server` REST API.
*   **`src/agent/session.rs`**: Manages agent sessions and conversation history.
*   **`src/agent/tools.rs`**: Defines the tools available to the agent.

## Server (`cade-server`)

### API

*   **`src/server/api/mod.rs`**: Defines the Axum router and API endpoints.
*   **`src/server/api/agents.rs`**: Handles agent-related API endpoints (create, list, get, delete, patch, tools, memory, conversations).
*   **`src/server/api/messages.rs`**: Handles message-related API endpoints (send, stream, search).
*   **`src/server/api/runs.rs`**: Handles background run API endpoints (get, stream).
*   **`src/server/api/providers.rs`**: Handles LLM provider API endpoints (add, list, presets, remove).
*   **`src/server/api/models.rs`**: Handles LLM model listing.

### LLM

*   **`src/server/llm/mod.rs`**: Defines the `LlmProvider` trait and related types.
*   **`src/server/llm/anthropic.rs`**: Implements the Anthropic LLM provider.
*   **`src/server/llm/openai.rs`**: Implements the OpenAI LLM provider.
*   **`src/server/llm/gemini.rs`**: Implements the Gemini LLM provider.
*   **`src/server/llm/catalogue.rs`**: Defines the model catalogue.
*   **`src/server/llm/ollama.rs`**: Implements the Ollama LLM provider.

### Storage

*   **`src/server/storage/sqlite.rs`**: Implements SQLite storage for agent state, messages, and providers.
*   **`src/server/crypto.rs`**: Implements encryption for sensitive data.

### Configuration and Utilities

*   **`src/server/config.rs`**: Defines the `ServerConfig` struct and related functions.
*   **`src/server/rate_limit.rs`**: Implements rate limiting for API requests.

## Shared Modules

*   **`src/mcp/mod.rs`**: Implements the Management Control Plane (MCP) for managing servers, agents, and tools.
*   **`src/permissions/mod.rs`**: Handles permissions and access control.
*   **`src/skills/mod.rs`**: Implements the skill system.
*   **`src/subagents/mod.rs`**: Implements sub-agent functionality.
*   **`src/hooks/mod.rs`**: Defines hooks for extending CADE's functionality.
*   **`src/settings/manager.rs`**: Manages application settings.
*   **`src/toolsets/mod.rs`**: Defines the `Toolset` enum for different editing paradigms.

## Desktop Extensions

*   **`src/desktop/mod.rs`**: Groups desktop-related modules.
*   **`src/desktop/capture.rs`**: Implements screen capture functionality.
*   **`src/desktop/control.rs`**: Implements desktop control (mouse, keyboard) functionality.
*   **`src/desktop/notify.rs`**: Implements desktop notifications.
*   **`src/desktop/tray.rs`**: Implements the system tray icon.

## Toolsets
* **`src/toolsets/mod.rs`**: Defines the `Toolset` enum which determines which family of tools to attach to the agent.
    * `Default`: String-replace editing, optimised for Claude/Anthropic models.
    * `Codex`: Patch-based editing (unified diff), optimised for OpenAI/GPT models.
    * `Gemini`: String-replace variant, optimised for Google Gemini models.

## Tools
* **`src/tools/manager.rs`**: Manages the available tools.
* **`src/tools/bash.rs`**: Implements the `bash` tool.
* **`src/tools/fs.rs`**: Implements file system tools (`read_file`, `write_file`, `edit_file`, etc.).
* **`src/tools/search.rs`**: Implements the `grep` and `glob` tools.
* **`src/tools/desktop.rs`**: Implements the desktop interaction tools (`desktop_screenshot`, `desktop_list_windows`, `desktop_control`, `desktop_notify`).
* **`src/tools/ask.rs`**: Implements the `ask_user_question` tool.
