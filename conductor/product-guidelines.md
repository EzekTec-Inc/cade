# Product Guidelines: CADE

## Documentation & Tone
- **Developer-Friendly:** All documentation, guides, and terminal output should employ conversational, developer-friendly language enriched with practical, real-world examples. Aim for a tone that is helpful, approachable, and technically precise.

## CLI UX Principles
- **Minimal & Terse Output:** The CLI should focus on delivering high-signal outputs. Avoid conversational filler or unnecessary text. Tools should act rather than narrate.
- **Rich Terminal UI:** Leverage `ratatui` and `crossterm` to create a clear visual hierarchy, utilizing spinners, markdown rendering, and structured layouts to enhance readability.
- **Speed & Responsiveness:** Ensure operations feel instantaneous. Utilize Server-Sent Events (SSE) for streaming outputs and provide immediate visual feedback (e.g., spinners) for background tasks.

## Error Handling & Feedback
- **Auto-Recovery First:** The system should aggressively attempt to auto-recover from failures before halting. When presenting an error, provide actionable feedback or attempt an automated fallback strategy.

## Architectural Philosophy
- **Client-Server Separation:** Maintain a strict separation between the `cade-server` (stateful agent backend) and the `cade` CLI (interactive frontend), ensuring the API remains clean and Letta-compatible.
- **Extensibility & Pluggability:** Design core systems to be easily extended via custom markdown Skills, Model Context Protocol (MCP) servers, and independent subagents.
- **Local-First & Independent:** Favor built-in, local-first solutions over external dependencies. The agent must function autonomously on the user's local machine without requiring external platform accounts.