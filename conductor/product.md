# Initial Concept
A stateful, multi-provider AI coding agent built in Rust with desktop extensions.

# Product Guide: CADE

## Vision
CADE is a stateful, multi-provider AI coding agent built in Rust, empowering developers with full access to their local development environment, including desktop extensions. By operating locally with its own server, it removes the need for external platform accounts, ensuring privacy and seamless integration.

## Target Audience
- **Privacy-conscious Developers:** Users who want a powerful local AI assistant without relying on third-party cloud platforms.
- **Power Users & Teams:** Individuals and organizations needing a custom, scriptable agent integrated with their local workflows and desktop environments.
- **Tool Builders:** Contributors looking to extend CADE with custom skills, MCP integrations, and new subagents.

## Primary Goal
The core differentiating goal of CADE is **Code Assistance**—prioritizing fast, robust code editing, intelligent codebase search, and context-aware project assistance, while providing seamless desktop interactions.

## Key Features & Roadmap Priorities
The immediate roadmap will focus heavily on improving the following areas:
- **Desktop Extensions:** Enhancing the suite of tools for capturing screenshots, controlling windows, and integrating with the system tray.
- **CLI & Core Routing:** Improving the interactive terminal UI (REPL), expanding Model Context Protocol (MCP) integrations, and optimizing local LLM routing capabilities.
- **Skills & Subagents:** Expanding the skill loading system, enhancing memory retention across sessions, and supporting complex background task execution via subagents.