# Technology Stack: CADE

## Programming Language
- **Rust:** The primary language for the entire codebase, chosen for its safety, performance, and robust ecosystem for systems programming and CLI tools.

## Backend (cade-server)
- **Axum:** Used for handling REST API routing and HTTP server interactions.
- **Tokio:** The primary asynchronous runtime powering network I/O and task execution.
- **Rusqlite:** Employed for local SQLite database persistence (agents, memory, messages).

## CLI & Frontend (cade)
- **Clap:** Handling command-line argument parsing and configuration.
- **Ratatui & Crossterm:** Providing the rich, interactive terminal user interface (REPL).
- **Reqwest:** Used for HTTP client operations and Server-Sent Events (SSE) streaming.

## Core Utilities & Extensions
- **rmcp:** Client library for spawning and communicating with local Model Context Protocol (MCP) servers.
- **xcap, notify-rust, ksni:** Libraries providing desktop extension capabilities (screen capture, notifications, system tray).