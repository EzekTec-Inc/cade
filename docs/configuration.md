# Configuration

Everything you can tune, in one place. Higher-priority sources override
lower ones.

```
priority (high → low):
  CLI flags  >  env vars  >  project settings  >  global settings  >  built-in defaults
```

## Files

| File | Scope | Track in VCS? |
|---|---|---|
| `~/.cade/settings.json` | Global (all projects) | No |
| `~/.cade/db.key` | DB encryption key | **No** (path-protected) |
| `~/.cade/pricing.json` | Pricing registry | No |
| `~/.cade/cade.log` | Server log | No |
| `~/.cade/skills/` | Global skills | No |
| `~/.cade/subagents/` | Global subagent definitions | No |
| `.cade/settings.json` | Project | **Yes** |
| `.cade/session.json` | Per-checkout state (last agent, mode) | No |
| `.cade/skills/` | Project skills | Yes |
| `.cade/subagents/` | Project subagents | Yes |
| `.cade/hooks/` | Project hook scripts | Yes |

## Global settings (`~/.cade/settings.json`)

```json
{
  "store_api_key": true,
  "default_model": "anthropic/claude-sonnet-4-5",
  "permission_mode": "default",
  "theme": "dark",
  "last_agent": "...",

  "permissions": {
    "always_allow": ["read_file", "glob"],
    "always_deny":  ["bash:rm -rf"]
  },

  "hooks": { /* see hooks.md */ },

  "mcpServers": { /* see mcp-servers.md */ },

  "execution": {
    "backend": "local",
    "docker_image": "ubuntu:22.04",
    "docker_flags": [],
    "ssh_host": "",
    "ssh_user": "",
    "ssh_key_path": null,
    "ssh_port": 22
  },

  "packages": [],
  "extra_prompt_dirs": []
}
```

| Field | Purpose |
|---|---|
| `store_api_key` | If `false`, ignore `env.api_key` from this file; rely on `CADE_API_KEY` env var only |
| `default_model` | Used when creating a new agent without `--model` |
| `permission_mode` | Default mode for new agents |
| `theme` | Theme name; empty = built-in default |
| `last_agent` | Persisted last-used agent id |
| `packages` | Installed packages (npm, git, or local path) |
| `extra_prompt_dirs` | Additional skill / template lookup directories |

## Project settings (`.cade/settings.json`)

Same shape as global, plus:

```json
{
  "auto_checkpoint_on_destructive": true,
  "hooks": { /* merged with global; project runs first */ },
  "mcpServers": { /* project wins on same key */ }
}
```

`auto_checkpoint_on_destructive` makes CADE create a checkpoint
automatically before destructive edits.

## Session file (`.cade/session.json`)

Per-checkout state, written by the CLI:

```json
{
  "last_agent_id": "agent-12345",
  "execution_backend": "local",
  "permission_mode": "default"
}
```

## Environment variables

### Server / connection

| Variable | Purpose | Default |
|---|---|---|
| `CADE_SERVER_URL` | Where the CLI connects | `http://127.0.0.1:8284` |
| `CADE_SERVER_PORT` | Server bind port | `8284` |
| `CADE_ALLOWED_ORIGIN` | CORS allow-origin | locked |
| `CADE_API_KEY` | CADE auth token (NOT an LLM key) | — |
| `CADE_LEGACY_API_KEY` | Legacy auth | — |
| `CADE_LEGACY_BASE_URL` | Legacy upstream | — |
| `CADE_MACHINE_SECRET` | Per-machine secret used to derive the DB key | auto |

### Storage

| Variable | Purpose |
|---|---|
| `CADE_DB_PATH` | Override SQLite DB location |
| `CADE_DB_KEY` | AES-GCM key (overrides `~/.cade/db.key`) |
| `CADE_AGENT_DIR` | Override `~/.cade/agents/` lookup root |
| `CADE_FS_ROOT` | Sandbox filesystem operations to this root |
| `CADE_FS_NO_SANDBOX` | Disable the FS sandbox (debugging only) |
| `CADE_RAG_EXPORT_DIR` | Where cade-rag writes export blobs |

### LLM / model

| Variable | Purpose |
|---|---|
| `CADE_DEFAULT_MODEL` | Override `default_model` from settings |
| `CADE_LLM_PROVIDER` | Force-pick a provider |
| `ANTHROPIC_API_KEY` | Anthropic key |
| `OPENAI_API_KEY` | OpenAI key |
| `GOOGLE_API_KEY` | Gemini key |

### Cost guardrails (see [cost-and-pricing.md](cost-and-pricing.md))

| Variable | Default |
|---|---|
| `CADE_MAX_SESSION_COST_USD` | unset |
| `CADE_TOOL_TURN_MAX_TOKENS` | unset |
| `CADE_GEMINI_CACHE_TTL_SECS` | 3600 |

### Context window

| Variable | Purpose |
|---|---|
| `CADE_CONTEXT_BUDGET` | Per-turn context budget (chars) |
| `CADE_MAX_CONTEXT_BUDGET` | Hard upper cap regardless of model |

### Subagents (see [subagents.md](subagents.md))

| Variable | Default |
|---|---|
| `CADE_SUBAGENT_MAX_DEPTH` | 3 |
| `CADE_MAX_SUBAGENTS` | 4 |
| `CADE_SUBAGENT_MAX_ITERS` | 10 |

### Execution backends (see [execution-backends.md](execution-backends.md))

| Variable | Purpose |
|---|---|
| `CADE_SSH_ACCEPT_NEW` | Auto-accept unknown SSH host keys |

## CLI flags (selected)

| Flag | Purpose |
|---|---|
| `--server-url <url>` | Override `CADE_SERVER_URL` |
| `--api-key <key>` | Override `CADE_API_KEY` |
| `--agent <id>` | Resume a specific agent |
| `--name <query>` | Match agent by name |
| `--new-agent` | Force creation of a new agent |
| `--model <model>` | Pin model for this session |
| `--prompt "<text>"` | Headless one-shot run (no TUI) |
| `--output-format <fmt>` | `text` (default) / `json` / `stream-json` |
| `--skills <dir>` | Custom skills directory |

Run `cade --help` for the full list.

## Settings hot-reload

`/hooks` re-reads `settings.json` and re-applies hooks, permissions, and
MCP servers without restarting the session. Skill discovery and
subagent discovery also pick up new files on hot-reload.

## Troubleshooting

| Symptom | Where to look |
|---|---|
| API key not being read | `store_api_key: false`? `CADE_API_KEY` set? |
| New global setting ignored | Did you edit project file by mistake? |
| Hooks not firing | Verify exec bit on the script; check `~/.cade/cade.log` |
| Wrong DB | `CADE_DB_PATH` shadowing your default |
| Permission mode resets | `.cade/session.json` overrides global on session start |
