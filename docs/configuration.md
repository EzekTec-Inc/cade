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
  "default_model": "openai/gpt-6-sol",
  "reasoning_effort": "medium",
  "permission_mode": "default",
  "theme": "dark",
  "last_agent": "...",

  "permissions": {
    "allow": [
      "Bash(cargo test)",
      "Bash(cargo build)",
      "read_file",
      "glob"
    ],
    "deny": [
      "Bash(rm:*)",
      "delete_file(*)"
    ],
    "strict_bash": false,
    "allow_agent_mode_changes": false
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
| `DEEPSEEK_API_KEY` | DeepSeek key |

### Cost guardrails (see [cost-and-pricing.md](cost-and-pricing.md))

The session cost cap is also available in settings files as
`max_session_cost_usd` under `.cade/settings.json` (project) or
`~/.cade/settings.json` (global).

| Variable | Default |
|---|---|
| `CADE_MAX_SESSION_COST_USD` | `120.00` (built-in; env var overrides settings) |
| `CADE_TOOL_TURN_MAX_TOKENS` | unset |
| `CADE_MAX_TURNS` | `20` (base adaptive budget) |
| `CADE_MAX_TURNS_CEILING` | `5 × CADE_MAX_TURNS` |
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

## OpenAI Dual-Wire Protocol & Model Capabilities

CADE integrates an authoritative model classifier (`OpenAiModelCapabilities`) to determine endpoint routing, token limits, and reasoning parameter serialization across OpenAI model families:

```mermaid
flowchart TD
    REQ[CompletionRequest: model, tools, reasoning_effort] --> CLASSIFY[OpenAiModelCapabilities::for_model]
    
    CLASSIFY -->|gpt-6* or gpt-5* with tools| RESPONSES_WIRE[/v1/responses Wire Protocol]
    CLASSIFY -->|o1*, o3*, o4* reasoning models| CHAT_O[/v1/chat/completions Wire Protocol]
    CLASSIFY -->|gpt-4o, gpt-4o-mini, legacy| CHAT_STD[/v1/chat/completions Wire Protocol]

    subgraph ResponsesFormatting [Responses API Payload]
        RESPONSES_WIRE --> R1[Endpoint: https://api.openai.com/v1/responses]
        RESPONSES_WIRE --> R2[input: array of input items]
        RESPONSES_WIRE --> R3[tools: flat function definitions]
        RESPONSES_WIRE --> R4[max_output_tokens: integer]
        RESPONSES_WIRE --> R5[reasoning: { effort: 'medium' }]
    end

    subgraph ChatOFormatting [Chat Completions Reasoning Payload]
        CHAT_O --> O1[Endpoint: https://api.openai.com/v1/chat/completions]
        CHAT_O --> O2[messages: standard chat message list]
        CHAT_O --> O3[max_completion_tokens: integer]
        CHAT_O --> O4[reasoning_effort: 'medium']
    end

    subgraph ChatStdFormatting [Standard Chat Payload]
        CHAT_STD --> S1[Endpoint: https://api.openai.com/v1/chat/completions]
        CHAT_STD --> S2[messages: standard chat message list]
        CHAT_STD --> S3[max_tokens: integer]
        CHAT_STD --> S4[tools: nested function declarations]
    end
```

### Model Classification Matrix

| Model Pattern | Protocol | Token Parameter | Reasoning Strategy | Tool Calling Behavior |
|---|---|---|---|---|
| `openai/gpt-6-*` (e.g. `gpt-6-sol`) | `/v1/responses` | `max_output_tokens` | `NestedReasoningObject` | Flat function tools + reasoning |
| `openai/gpt-5*` (e.g. `gpt-5.6-luna`) | `/v1/responses` | `max_output_tokens` | `NestedReasoningObject` | Flat function tools + reasoning |
| `openai/o1*`, `o3*`, `o4*` | `/v1/chat/completions` | `max_completion_tokens` | `TopLevelReasoningEffort` | Standard tools + top-level effort |
| `openai/gpt-4.5*` | `/v1/chat/completions` | `max_completion_tokens` | `None` | Standard chat tools |
| `openai/gpt-4o*` & legacy | `/v1/chat/completions` | `max_tokens` | `None` | Standard chat tools |

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

## Comprehensive Configuration Examples

### Example 1: Enterprise Hardened Project (`.cade/settings.json`)
Ideal for security-conscious teams, enforcing strict permissions, pre-tool safety hooks, transparent Headroom token compression, and core MCP servers:
```json
{
  "permission_mode": "strict",
  "auto_checkpoint_on_destructive": true,
  "max_session_cost_usd": 5.00,
  "default_model": "anthropic/claude-sonnet-4-5",

  "permissions": {
    "always_allow": [
      "read_file",
      "glob",
      "grep",
      "set_plan",
      "UpdatePlan"
    ],
    "always_deny": [
      "bash:rm -rf /",
      "bash:git push --force",
      "bash:docker system prune"
    ]
  },

  "hooks": {
    "PreToolUse": [
      {
        "matcher": "bash",
        "command": ".cade/hooks/block-dangerous-git.sh"
      }
    ],
    "PostToolUse": [
      {
        "matcher": "write_file",
        "command": ".cade/hooks/notify-on-mutation.sh"
      }
    ]
  },

  "mcpServers": {
    "serena": {
      "command": "serena",
      "args": ["start-mcp"],
      "core_server": true
    },
    "cade-rag": {
      "command": "cade-rag-mcp",
      "core_server": true
    }
  }
}
```

### Example 2: Local-First / Zero-Cloud Developer Configuration
Ideal for offline development with zero remote API calls and 100% private data isolation:
```bash
# ~/.bashrc or terminal environment
export CADE_LLM_PROVIDER=ollama
export CADE_DEFAULT_MODEL=ollama/qwen2.5-coder:7b
export OLLAMA_BASE_URL=http://127.0.0.1:11434
export CADE_PERMISSION_MODE=default
export CADE_AUTO_START_SERVER=true
```
With `~/.cade/settings.json`:
```json
{
  "default_model": "ollama/qwen2.5-coder:7b",
  "permission_mode": "default",
  "theme": "cyber-dark",
  "execution": {
    "backend": "local"
  },
  "permissions": {
    "always_allow": ["read_file", "write_file", "edit_file", "glob", "grep"]
  }
}
```

### Example 3: Automated CI/CD Non-Interactive Pipeline
For GitHub Actions, GitLab CI, or headless automation tasks:
```bash
#!/usr/bin/env bash
export CADE_PERMISSION_MODE=yolo
export CADE_DEFAULT_MODEL=anthropic/claude-haiku-4-5
export CADE_MAX_TURNS=15
export CADE_OUTPUT_FORMAT=json

cade --prompt "Audit all dependencies in Cargo.lock and generate a vulnerability markdown table" > report.json
```

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
