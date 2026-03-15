# CADE Security Model

## Threat Model

CADE is a **local, single-user** coding assistant. Both the CLI and the server
run on the developer's own machine, communicating over `localhost`.

| Component | Runs on | Listens on |
|---|---|---|
| `cade` (CLI/TUI) | User machine | — (client only) |
| `cade-server` | User machine | `127.0.0.1:8284` (configurable) |

CADE is **not** designed to be exposed to the public internet, shared between
untrusted users, or run in a multi-tenant environment. If you require any of
those, additional hardening is needed beyond what is described here.

---

## Capabilities with Elevated Risk

### Shell execution (`bash` tool)

The LLM can request arbitrary shell commands. Defences:

- **Plan mode** blocks all write/mutating commands via `bash_command_is_write()`.
- **Default mode** prompts the user for approval on every bash invocation
  (bash is never auto-approved unless an explicit allow rule is configured).
- **`strict_bash` setting** (`~/.cade/settings.json → permissions.strict_bash`):
  when `true`, bash tools are never auto-approved even if an allow rule matches.
  Every bash call requires explicit user confirmation.
- **`BypassPermissions` mode** auto-approves everything (including bash);
  use only when you trust the LLM fully and want zero-prompt workflows.
- Defence-in-depth: `bash_command_is_suspicious()` logs high-risk patterns
  (nested shells, network tools, obfuscation, critical system files) regardless
  of approval status.

### File tools (`read_file`, `write_file`, `edit_file`, `apply_patch`)

The LLM can read and write files on the host filesystem. Defences:

- **Plan mode** blocks all write tools (`write_file`, `edit_file`, etc.).
- **Default / AcceptEdits modes** require user approval for file operations
  (AcceptEdits auto-approves only file edits, not reads of sensitive files).
- **Path traversal defence** (`apply_patch`): patch paths containing `..`
  segments or absolute paths are rejected before `patch` is invoked.
- **Opt-in filesystem sandboxing** (`CADE_FS_ROOT`): when set, all file-tool
  paths are verified to resolve within the specified directory. Paths that
  escape (via `..`, symlinks, or absolute paths) are rejected. When unset,
  tools operate without path confinement (default for local dev use).

### Desktop control (`desktop_control`)

The LLM can drive keyboard, mouse, and window focus via xdotool/ydotool.
This tool is listed in `WRITE_TOOLS` and is blocked in plan mode. In other
modes it requires explicit user approval.

### MCP servers

External MCP servers run as child processes. Tool calls are routed to them
via JSON-RPC over stdio. Stale MCP tool schemas are cleaned up at startup.
Reconnection attempts are limited and protocol errors are not retried.

---

## Authentication & Authorization

| Layer | Mechanism |
|---|---|
| Server API auth | Optional Bearer token via `CADE_API_KEY` env var. When set, all endpoints (except `/v1/health`) require `Authorization: Bearer <token>`. |
| CORS | Restricted to `localhost` / `127.0.0.1` origins only. |
| Rate limiting | Token-bucket per agent on inference endpoints. Configurable via `CADE_RATE_LIMIT_RPM` / `CADE_RATE_LIMIT_BURST`. |

---

## Secrets Management

| Secret | Storage | Protection |
|---|---|---|
| LLM provider API keys (server-side) | SQLite `providers` table | Encrypted at rest via AES-256-GCM (machine-specific key). |
| LLM provider API keys (CLI-side) | `~/.cade/settings.json` (optional) | Plaintext JSON file. Prefer env vars (`ANTHROPIC_API_KEY`, etc.). Set `"store_api_key": false` in settings to disable file-based key reading entirely. |
| `CADE_API_KEY` (server auth) | Environment variable | Never persisted to disk by CADE. |

**Recommendation:** Use environment variables for all API keys. Avoid writing
keys into `~/.cade/settings.json` unless convenience outweighs risk.

---

## Headless / CI Mode

When running in headless mode (`cade --prompt "..."` or piped stdin), model
and tool output is sanitized before printing to stdout/stderr:

- Control characters (`0x00–0x1F` except `\n`/`\t`, and `0x7F`) are stripped.
- This prevents ANSI escape sequence injection (e.g. cursor manipulation,
  OSC 52 clipboard exfiltration) from a compromised model or upstream server.
- JSON output modes (`--output json`, `--output stream-json`) are inherently
  safe because `serde_json` escapes control characters in string values.

---

## Configuration Reference

```jsonc
// ~/.cade/settings.json
{
  "permissions": {
    "allow": [],          // e.g. ["Bash(cargo test)", "Read(src/**)"]
    "deny":  [],          // e.g. ["Bash(rm -rf:*)"]
    "strict_bash": false  // true → every bash call requires approval
  },
  "store_api_key": true   // false → ignore env.api_key from this file
}
```

```bash
# Environment variables
CADE_API_KEY=...           # Server auth token
CADE_FS_ROOT=/path/to/dir  # Opt-in filesystem sandbox for file tools
CADE_RATE_LIMIT_RPM=60     # Requests per minute per agent (default 60)
CADE_RATE_LIMIT_BURST=10   # Burst capacity (default 10)
```

---

## Reporting

If you discover a security vulnerability in CADE, please open a private
issue or contact the maintainer directly.
