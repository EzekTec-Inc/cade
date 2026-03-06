# CADE Architecture

> **CADE** — Coding AI Assistant with Desktop Extensions
> A stateful, multi-provider AI coding agent built in Rust.

---

## CADE's Architectural Pattern

### Core Pattern: **Persistent HTTP-Driven Agentic Loop**

```
┌─────────────────────────────────────────────────────────────┐
│                        CLI / Client                         │
│         REPL  ·  Headless  ·  Stream-JSON  ·  Subagent      │
└────────────────────────┬────────────────────────────────────┘
                         │  HTTP + SSE
┌────────────────────────▼────────────────────────────────────┐
│                    Axum HTTP Server                         │
│   /agents  /messages  /conversations  /tools  /memory       │
└────────────────────────┬────────────────────────────────────┘
                         │
          ┌──────────────▼──────────────┐
          │         Agent Turn          │  ← The core loop
          │                             │
          │  1. Load history (SQLite)   │
          │  2. Inject memory blocks    │
          │  3. Trim to context budget  │
          │  4. Stream LLM response     │
          │  5. Persist messages        │
          │  6. Emit SSE events         │
          └──────────────┬──────────────┘
                         │ tool_calls in response?
          ┌──────────────▼──────────────┐
          │      Tool Dispatch Loop     │
          │                             │
          │  classify → sequential?     │
          │      yes → run in order     │
          │      no  → join_all()       │
          │  collect results            │
          │  → POST back to /messages   │
          │  → repeat from step 1       │
          └─────────────────────────────┘
```

---

## The 5 Interlocking Patterns

### 1. 🔁 Agentic Loop (not a static graph)

Unlike graph-based frameworks with fixed node edges, CADE's loop is **LLM-driven and dynamic**:

- The LLM decides at runtime which tools to call, in what order, how many times
- The loop runs until `finish_reason == "end_turn"` or max turns reached
- No pre-wired edges — the LLM is the router

```
stream → tool_calls? → dispatch → results → stream → tool_calls? → ...
```

### 2. 🗄️ Persistent Conversation State

Every message, tool call, tool result, and memory block is **stored in SQLite** and reloaded on every turn. This makes CADE stateful across sessions — completely unlike ephemeral in-memory frameworks.

```
SQLite
  ├── agents         (id, name, model, system_prompt)
  ├── conversations  (id, agent_id, title)
  ├── messages       (id, conv_id, role, content, tool_calls, tool_call_id)
  ├── memory_blocks  (id, agent_id, label, value)
  └── tools          (id, name, description, json_schema, executor)
```

### 3. 📡 Server-Sent Events (SSE) Streaming

The server streams LLM output token-by-token to the client via SSE. The client reacts to each chunk as it arrives — not after the full response completes.

```
Server                         Client
  │── text chunk ────────────►  │  print to terminal
  │── text chunk ────────────►  │  print to terminal
  │── tool_call_message ──────► │  dispatch tool
  │── tool_result_message ────► │  (echoed back)
  │── usage_message ──────────► │  track tokens
  │── stop_reason ────────────► │  end turn
```

### 4. ⚡ Classified Parallel Tool Dispatch

Tool calls in a single LLM response are classified then dispatched:

```rust
sequential tools  → run one-by-one  (update_memory, load_skill, install_skill)
regular tools     → join_all()      (bash, read_file, grep, screenshot, …)
```

Sequential tools mutate shared agent state and cannot be safely parallelised.
Regular tools are independent and run concurrently via `futures::future::join_all()`.

### 5. 🧠 Memory + Skills Injection

Before every LLM call, CADE dynamically assembles the system prompt by:

- Loading memory blocks from SQLite (ordered by recency, capped by `MEMORY_CHAR_BUDGET`)
- Injecting loaded skills (markdown documents with embedded instructions)
- Trimming full message history to fit the model's context window

```
system_prompt = base_prompt + memory_blocks + skills
messages      = sanitize(history[-N:]) trimmed to context_char_budget
context_char_budget = (model_context_window_tokens × 3).clamp(8_000, 600_000)
```

---

## Layer Map

```
src/
├── bin/
│   └── cade-server.rs          # Server binary entrypoint
├── main.rs                     # CLI entrypoint + server auto-start
│
├── server/                     # HTTP server (Axum)
│   ├── api/
│   │   ├── agents.rs           # CRUD: agents, memory blocks
│   │   ├── conversations.rs    # CRUD: conversations
│   │   ├── messages.rs         # POST /messages → agentic loop + SSE
│   │   ├── models.rs           # GET /v1/models (live + catalogue)
│   │   ├── runs.rs             # Background runs (headless via API)
│   │   └── tools.rs            # CRUD: registered tools
│   ├── config.rs               # ServerConfig (env vars + defaults)
│   ├── llm/
│   │   ├── mod.rs              # LlmRouter, retry_with_backoff, provider_error
│   │   ├── anthropic.rs        # Anthropic Claude provider
│   │   ├── openai.rs           # OpenAI + OpenAI-compat providers
│   │   ├── gemini.rs           # Google Gemini provider
│   │   ├── ollama.rs           # Ollama local provider (delegates to OpenAI)
│   │   └── catalogue.rs        # Static model catalogue + context_window_for_model
│   ├── mcp/                    # MCP (Model Context Protocol) server
│   ├── rate_limit.rs           # Per-agent token-bucket rate limiter
│   ├── state.rs                # AppState (router, db, rate_limiter)
│   └── storage/
│       └── sqlite.rs           # All DB queries (agents, convos, messages, tools, memory)
│
├── agent/
│   ├── client.rs               # CadeClient — HTTP client for the CADE API
│   ├── session.rs              # Session persistence (.cade/settings.local.json)
│   ├── tools.rs                # Tool schema registration helpers
│   └── mod.rs
│
├── cli/
│   ├── repl.rs                 # Interactive REPL (3628 lines — full UI layer)
│   └── headless.rs             # Headless + stream-json modes
│
├── subagents/
│   └── mod.rs                  # SubagentDef, discovery, builtin subagents
│
├── skills/
│   └── mod.rs                  # Skill discovery, parsing, file watcher
│
├── tools/
│   └── dispatch.rs             # Tool execution dispatch (bash, files, grep, …)
│
├── mcp/
│   └── mod.rs                  # MCP client manager (external tool servers)
│
└── permissions/
    └── mod.rs                  # PermissionManager (block/allow rules)
```

---

## LLM Provider Architecture

```
LlmRouter
  ├── anthropic   → AnthropicProvider  (native Anthropic API)
  ├── openai      → OpenAiProvider     (OpenAI API)
  ├── gemini      → GeminiProvider     (Google Gemini API)
  ├── ollama      → OllamaProvider     (delegates to OpenAiProvider)
  ├── openrouter  → OpenAiProvider     (OpenRouter compat, auto-detected from env)
  ├── groq        → OpenAiProvider     (Groq compat, auto-detected from env)
  ├── together    → OpenAiProvider     (Together AI compat, auto-detected from env)
  ├── fireworks   → OpenAiProvider     (Fireworks compat, auto-detected from env)
  └── deepinfra   → OpenAiProvider     (DeepInfra compat, auto-detected from env)

All providers implement:
  async fn complete(req) → CompletionResponse
  async fn stream(req)   → Pin<Box<dyn Stream<Item = Result<StreamChunk>>>>

Wrapped with retry_with_backoff():
  - 3 attempts, 1s base delay, 2× multiplier, 8s cap
  - Retries:   429, 500, 502, 503, 504, connection errors, timeouts
  - Fail-fast: 400, 401, 403, 404
```

---

## Subagent Architecture

```
Main Agent (REPL / Headless)
  │
  ├── detects @subagent or /agent command
  │
  ├── resolves SubagentDef
  │   ├── Builtin:  explore, general-purpose, coder, reviewer, reflection, recall
  │   ├── Global:   ~/.cade/subagents/*.md
  │   └── Project:  .cade/subagents/*.md
  │
  ├── creates ephemeral CADE agent (HTTP POST /agents)
  │
  ├── dispatches via Arc<Semaphore> (concurrency cap)
  │
  └── streams subagent turn → collects result → injects as tool result
```

---

## Memory System

```
Memory Blocks (SQLite: memory_blocks)
  ├── label     — unique key per agent (e.g. "project", "persona", "human")
  ├── value     — free-text content
  └── updated_at — used for recency ordering

Injection at turn time:
  1. Load all blocks for agent, order by updated_at DESC
  2. Accumulate into system prompt until MEMORY_CHAR_BUDGET (8,000 chars) reached
  3. Empty blocks always skipped

Update via tool:
  update_memory(label, value, operation="set"|"append")
  → intercepted by client before server round-trip
  → written directly to SQLite
```

---

## Context Budget System

```
Per-turn context window management:

model_context_window  (tokens, from catalogue or provider-prefix heuristic)
         ×  3         (chars per token — conservative, ~25% headroom)
  .clamp(8_000, 600_000)  (min: tiny local models, max: Gemini 2M token window)
         = context_char_budget

History trimming:
  while total_chars(messages) > context_char_budget AND len(messages) > 3:
      remove messages[1]   ← oldest non-system message

Always preserved:
  messages[0]             ← system prompt
  messages[-2..]          ← last user + assistant turn
```

---

## How It Compares to Other Frameworks

| Dimension | AgentFlow | LangChain | CADE |
|---|---|---|---|
| Routing | Static graph | Chain / DAG | LLM-driven, dynamic |
| State | Ephemeral HashMap | Ephemeral | Persistent SQLite |
| Multi-turn | No | Optional | Native (core feature) |
| Streaming | No | Partial | Full SSE |
| Tool dispatch | No | Sequential | Parallel + classified |
| Multi-agent | Fixed set | Predefined | Dynamic, scoped, semaphore-guarded |
| LLM providers | Via `rig` crate | Via LangChain | Own layer, hot-reload |
| Memory | No | Vector store | Structured blocks + FTS5 search |
| Scale | ~730 lines | Massive Python | ~15,000 lines Rust |

---

## One-Line Summary

> **CADE is a Persistent Multi-turn SSE-Streaming Agentic Server — the LLM is the router, SQLite is the brain, and the tool loop runs until the LLM says it's done.**

This is the architecture that production AI coding assistants (Claude Code, Cursor, Windsurf) converged on independently. CADE is a Rust-native implementation of that same pattern.
