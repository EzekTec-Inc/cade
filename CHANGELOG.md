# Changelog

All notable changes to CADE are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versions follow [Semantic Versioning](https://semver.org/).

---

## [0.2.0] — 2026-03-07

### Added
- `X-Cade-Version` response header on every server response (version middleware in `cade-server`)
- Client-side version mismatch warning at startup — logs a warning when client and server versions differ
- `GET /v1/health` and `GET /v1/config` responses now include a `version` field (`CARGO_PKG_VERSION`)
- `created_at` (ISO-8601) field in `AgentResponse` — clients can now sort agents by creation time
- `server_version()` method on `CadeClient` for programmatic version queries

### Changed
- Hook pattern matching now uses the `regex` crate — full regex syntax supported (anchoring, character classes, groups, alternation with `|`, etc.). Invalid patterns fall back to case-insensitive substring match for backwards compatibility.
- Startup log messages (`"cade-server not running — starting…"` and `"cade-server ready."`) converted from `eprintln!` to `tracing::info!` for consistent log filtering via `RUST_LOG`

### Fixed
- `AgentRow` and all SQL queries now correctly read `created_at` from the database (previously the field was written but never read back)

---

## [0.1.0] — 2026-03-04

### Added
- Initial release of CADE (Coding AI assistant with Desktop Extensions)
- Multi-provider LLM support: Anthropic Claude, OpenAI GPT/o-series, Google Gemini, Ollama (local)
- Persistent agent state with SQLite backend
- TUI REPL with streaming responses, tool call display, reasoning collapse
- Tool suite: `bash`, `read_file`, `write_file`, `edit_file`, `grep`, `glob`, `apply_patch`, desktop tools
- Skills system: scoped SKILL.MD files with frontmatter, live file watcher, trigger-based auto-activation
- Subagent support with configurable concurrency cap (`CADE_MAX_SUBAGENTS`)
- MCP server integration with automatic reconnect on crash
- Agent export/import (`--export-agent` / `--import-agent`, `/export`, `/import`)
- FTS5-backed semantic message search (`/search`)
- Per-agent token-bucket rate limiter on inference endpoints
- LLM retry with exponential backoff (3 attempts, 2× multiplier, 8 s cap)
- Dynamic context budget per model from catalogue (`context_window_for_model`)
- Headless mode with `--timeout-secs` global timeout and parallel tool dispatch
- Configurable server port via `CADE_SERVER_PORT` env var, respected by both client and server
- Approval modal for tool calls with session-allow rules
- `/stats` command with per-model token usage breakdown
- `/skills` command: list, show, create, edit, delete, reload
