# Security Policy

## Security Model

CADE is a local-first coding agent that runs on your machine with full filesystem access. Understanding its security model is essential.

### Trust Boundaries

```
┌─────────────────────────────────────────┐
│  User's Machine (trusted boundary)      │
│                                         │
│  cade (CLI)  ←──HTTP──►  cade-server    │
│    │                         │          │
│    ├── Local filesystem      ├── SQLite │
│    ├── MCP servers           ├── Crypto │
│    └── Desktop extensions    └── API    │
│                                         │
└─────────────┬───────────────────────────┘
              │ HTTPS (outbound only)
              ▼
       LLM Provider APIs
       (Anthropic, OpenAI, Google, Ollama)
```

- **Trusted**: The local machine, CADE server, and SQLite database
- **Semi-trusted**: LLM providers (receive your prompts and code context)
- **Untrusted**: External MCP servers, skill URLs, web content

### Data at Rest

| Data | Storage | Protection |
|------|---------|------------|
| API keys | SQLite (`cade-store`) | AES-GCM encryption with machine-derived key |
| Conversation history | SQLite | Plaintext (local only) |
| Memory blocks | SQLite | Plaintext (local only) |
| Session state | `.cade/settings.local.json` | Plaintext (gitignored) |

### Data in Transit

- All LLM API calls use HTTPS
- Local CLI ↔ server communication is HTTP on `127.0.0.1` (localhost only by default)
- Optional `CADE_API_KEY` for server authentication in remote deployments

---

## Permission Modes

CADE enforces tool execution permissions through four modes:

| Mode | Behaviour | Risk |
|------|-----------|------|
| `default` | Prompts for approval on write/execute | Low |
| `acceptEdits` | Auto-approves file write/edit only | Medium |
| `plan` | Read-only — blocks bash/write/edit | Minimal |
| `bypassPermissions` | Auto-approves everything (`--yolo`) | High |

### Hooks for Auditing

User-defined hooks can intercept tool calls at lifecycle events:

- **`PreToolUse`** — audit or block before execution
- **`PostToolUse`** — log or inject context after execution
- **`PermissionRequest`** — custom approval logic

Hook exit codes:
| Code | Meaning |
|------|---------|
| `0` | Allow |
| `1` | Log and continue |
| `2` | Block (stderr fed back to agent) |

---

## Hardening Recommendations

### For Local Development

1. **Use `default` permission mode** — review tool calls before execution
2. **Set up `PreToolUse` hooks** for sensitive commands (e.g., `rm -rf`, `git push --force`)
3. **Use `--tools` flag** to restrict available tools for specific tasks
4. **Review `.cade/settings.local.json`** — contains your agent ID (gitignored by default)

### For Remote/Shared Deployments

1. **Set `CADE_API_KEY`** on both server and client for authentication
2. **Bind to localhost only** (`127.0.0.1`) — do not expose the server to public networks without a reverse proxy with TLS
3. **Use `plan` mode** for read-only access in shared environments
4. **Rotate API keys** regularly — stored keys are encrypted but the machine-derived key is static

### For CI/CD

1. **Use headless mode** (`cade -p "..." --output-format json`)
2. **Set `--permission-mode plan`** for analysis-only tasks
3. **Provide API keys via environment variables** — never commit them
4. **Use `--tools` to restrict** to only the tools needed for the pipeline

---

## Known Limitations

- **No network isolation**: The agent can make arbitrary HTTP requests via `bash` or web tools
- **No sandboxing**: File operations execute directly on the host filesystem
- **LLM prompt injection**: Code context sent to LLM providers could theoretically be crafted to manipulate agent behaviour — use trusted codebases
- **MCP server trust**: External MCP servers are granted tool-level access; only connect to trusted servers

---

## Reporting Vulnerabilities

If you discover a security vulnerability in CADE:

1. **Do NOT open a public issue**
2. Email: **security@ezektec.com**
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

We aim to respond within 48 hours and will coordinate disclosure timelines with you.

---

## Dependencies

CADE uses well-maintained Rust crates for security-sensitive operations:

| Operation | Crate | Notes |
|-----------|-------|-------|
| Encryption | `aes-gcm` | AES-256-GCM for API key storage |
| Key derivation | `pbkdf2` | PBKDF2-HMAC-SHA256 |
| Hashing | `sha2`, `hmac` | SHA-256 for integrity checks |
| Random | `getrandom` | OS-level entropy source |
| Machine ID | `machine-uid` | Derives encryption key from hardware |
| SQLite | `rusqlite` (bundled) | WAL mode for concurrent integrity |
| TLS | `reqwest` + `rustls` | All outbound HTTPS |

---

Built by [EzekTec Inc.](https://github.com/EzekTec-Inc) · Apache-2.0 / MIT
