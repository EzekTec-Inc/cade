# Architecture of CADE

CADE (Coding AI with Desktop Extensions) is a helpful coding AI agent that's skilled beyond just coding, but skilled in keeping the developer efficient and organized while building digital solutions.


## Important Note about Source Trees

The codebase uses a Cargo workspace structure.
*   **`crates/`**: Contains the actual implementations (live code).
*   **`src/`**: The root `src/` directory (excluding `main.rs`, `lib.rs`, `bin/cade-server.rs`) contains dead code/legacy files. `lib.rs` simply re-exports from the workspace crates. Always modify the files in `crates/`.

## Core Components

*   **`src/main.rs`**: Entry point for the `cade` CLI application.
*   **`src/bin/cade-server.rs`**: Entry point for the `cade-server` backend.
*   **`src/lib.rs`**: Defines shared modules.

## CLI (`cade`)

### Modules

*   **`crates/cade-cli/src/cli/repl.rs`**: Implements the interactive REPL (Read-Eval-Print Loop) for CADE.
*   **`crates/cade-cli/src/cli/headless.rs`**: Implements headless mode for running CADE from the command line without a UI.
*   **`crates/cade-cli/src/cli/args.rs`**: Defines command-line arguments using `clap`.
*   **`crates/cade-cli/src/cli/export_import.rs`**: Handles exporting and importing agent configurations and data.

### UI

*   **`crates/cade-cli/src/ui/app.rs`**: Main application logic for the terminal UI using `ratatui`.
*   **`crates/cade-cli/src/ui/component.rs`**: Defines the foundational `Component` trait for unified rendering and input handling.
*   **`crates/cade-cli/src/ui/editor.rs`**: Implements the `Editor` component with bracketed-paste support and text-editing primitives.
*   **`crates/cade-cli/src/ui/autocomplete.rs`**: Pluggable autocomplete providers (Tab path completion, `@` file picker, slash commands).
*   **`crates/cade-cli/src/ui/markdown.rs`**: Handles markdown rendering in the UI.
*   **`crates/cade-cli/src/ui/question.rs`**: Implements interactive question prompts in the UI.
*   **`crates/cade-cli/src/ui/menu.rs`**: Implements menuing system for the UI.

### Agent

*   **`crates/cade-agent/src/agent/client.rs`**: Defines the `CadeClient` for interacting with the `cade-server` REST API.
*   **`crates/cade-agent/src/agent/session.rs`**: Manages agent sessions and conversation history.
*   **`crates/cade-agent/src/agent/tools.rs`**: Defines the tools available to the agent.

## Server (`cade-server`)

### API

*   **`crates/cade-server/src/server/api/mod.rs`**: Defines the Axum router and API endpoints.
*   **`crates/cade-server/src/server/api/agents.rs`**: Handles agent-related API endpoints (create, list, get, delete, patch, tools, memory, conversations).
*   **`crates/cade-server/src/server/api/messages.rs`**: Handles message-related API endpoints (send, stream, search).
*   **`crates/cade-server/src/server/api/runs.rs`**: Handles background run API endpoints (get, stream).
*   **`crates/cade-server/src/server/api/providers.rs`**: Handles LLM provider API endpoints (add, list, presets, remove).
*   **`crates/cade-server/src/server/api/models.rs`**: Handles LLM model listing.

### LLM

*   **`crates/cade-server/src/server/llm/mod.rs`**: Defines the `LlmProvider` trait and related types.
*   **`crates/cade-server/src/server/llm/anthropic.rs`**: Implements the Anthropic LLM provider.
*   **`crates/cade-server/src/server/llm/openai.rs`**: Implements the OpenAI LLM provider.
*   **`crates/cade-server/src/server/llm/gemini.rs`**: Implements the Gemini LLM provider.
*   **`crates/cade-server/src/server/llm/catalogue.rs`**: Defines the model catalogue.
*   **`crates/cade-server/src/server/llm/ollama.rs`**: Implements the Ollama LLM provider.

### Storage

*   **`crates/cade-server/src/server/storage/sqlite.rs`**: Implements SQLite storage for agent state, messages, and providers.
*   **`crates/cade-server/src/server/crypto.rs`**: Implements encryption for sensitive data.

### Configuration and Utilities

*   **`crates/cade-server/src/server/config.rs`**: Defines the `ServerConfig` struct and related functions.
*   **`crates/cade-server/src/server/rate_limit.rs`**: Implements rate limiting for API requests.

## Shared Modules

*   **`crates/cade-agent/src/mcp/mod.rs`**: Implements the Management Control Plane (MCP) for managing servers, agents, and tools.
*   **`crates/cade-core/src/permissions/mod.rs`**: Handles permissions and access control.
*   **`crates/cade-core/src/skills/mod.rs`**: Implements the skill system.
*   **`crates/cade-agent/src/subagents/mod.rs`**: Implements sub-agent functionality.
*   **`crates/cade-core/src/hooks/mod.rs`**: Defines hooks for extending CADE's functionality.
*   **`crates/cade-core/src/settings/manager.rs`**: Manages application settings.
*   **`crates/cade-core/src/toolsets/mod.rs`**: Defines the `Toolset` enum for different editing paradigms.

## Desktop Extensions

*   **`crates/cade-desktop/src/desktop/mod.rs`**: Groups desktop-related modules.
*   **`crates/cade-desktop/src/desktop/capture.rs`**: Implements screen capture functionality.
*   **`crates/cade-desktop/src/desktop/control.rs`**: Implements desktop control (mouse, keyboard) functionality.
*   **`crates/cade-desktop/src/desktop/notify.rs`**: Implements desktop notifications.
*   **`crates/cade-desktop/src/desktop/tray.rs`**: Implements the system tray icon.

## Toolsets
* **`crates/cade-core/src/toolsets/mod.rs`**: Defines the `Toolset` enum which determines which family of tools to attach to the agent.
    * `Default`: String-replace editing, optimised for Claude/Anthropic models.
    * `Codex`: Patch-based editing (unified diff), optimised for OpenAI/GPT models.
    * `Gemini`: String-replace variant, optimised for Google Gemini models.

## Tools
* **`crates/cade-agent/src/tools/manager.rs`**: Manages the available tools.
* **`crates/cade-agent/src/tools/bash.rs`**: Implements the `bash` tool.
* **`crates/cade-agent/src/tools/fs.rs`**: Implements file system tools (`read_file`, `write_file`, `edit_file`, etc.).
* **`crates/cade-agent/src/tools/search.rs`**: Implements the `grep` and `glob` tools.
* **`crates/cade-agent/src/tools/desktop.rs`**: Implements the desktop interaction tools (`desktop_screenshot`, `desktop_list_windows`, `desktop_control`, `desktop_notify`).
* **`crates/cade-agent/src/tools/ask.rs`**: Implements the `ask_user_question` tool.
