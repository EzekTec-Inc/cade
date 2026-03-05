# CADE Improvement Plan

Tracked implementation plan — ordered Critical → High → Medium → Low.  
Each item includes the exact file(s) affected, the problem, and the implementation approach.

Status legend: `[ ]` = not started · `[~]` = in progress · `[x]` = done

---

## 🔴 Critical

---

### C-01 · LLM retry + exponential backoff
**Status:** `[ ]`  
**Files:** `src/server/llm/anthropic.rs`, `src/server/llm/openai.rs`, `src/server/llm/gemini.rs`, `src/server/llm/ollama.rs`  
**Problem:** Any transient network error, 429 (rate limit), or 500 silently kills the turn. Zero retry logic exists anywhere in the codebase.  
**Fix:**
- Add a `retry_with_backoff(max_attempts, base_delay, f)` helper in `src/server/llm/mod.rs`
- Wrap `complete()` and `stream()` calls in each provider with it
- Retry on: connection errors, 429, 503 · Fail-fast on: 400, 401, 404
- Max 3 attempts, base delay 1 s, multiplier 2× (caps at 8 s)
- Surface attempt count to `tracing::warn!` so the user sees what's happening

---

### C-02 · Dynamic context budget tied to model context length
**Status:** `[ ]`  
**Files:** `src/server/api/messages.rs:32`, `src/server/llm/catalogue.rs`  
**Problem:** `CONTEXT_CHAR_BUDGET` is a hardcoded `200_000` constant. This is correct for Claude 3.x/4.x (200K tokens) but wrong for GPT-4o (128K), Gemini Flash (1M), Llama 3.2 (128K), Groq models (32K). Overflow → silent truncation or API errors.  
**Fix:**
- Add `context_window: u32` field to `ModelEntry` in `catalogue.rs`
- Populate for all known models in the catalogue
- In `messages.rs`, look up the agent's model from the catalogue and use `(context_window * 3) as usize` as the char budget (3 chars ≈ 1 token on average)
- Fall back to `200_000` if model not in catalogue
- Expose `CADE_CONTEXT_BUDGET` env var override for power users

---

### C-03 · Fix unsafe fd duplication in server auto-start
**Status:** `[ ]`  
**Files:** `src/main.rs:403–413`  
**Problem:** `IntoRawFd` + `FromRawFd` is used to duplicate a `File` into stdout and stderr of the child process. The fd is consumed by `into_raw_fd()` then reconstructed twice — second `from_raw_fd` creates a duplicate handle on the same fd, which is undefined behaviour if either handle is closed.  
**Fix:**
```rust
// Before (unsafe):
let fd = log.into_raw_fd();
unsafe {
    cmd.stdout(std::fs::File::from_raw_fd(fd));
    cmd.stderr(std::fs::File::from_raw_fd(fd));
}

// After (safe):
let stderr_log = log.try_clone()?;
cmd.stdout(log);
cmd.stderr(stderr_log);
```
`try_clone()` creates a proper OS-level dup2, which is safe and correct.

---

## 🔴 High

---

### H-01 · Parallel tool dispatch (concurrent tool calls)
**Status:** `[ ]`  
**Files:** `src/cli/headless.rs:143`, `src/cli/repl.rs` (tool call loop)  
**Problem:** When the LLM returns multiple tool calls in one response they are executed sequentially. A response with 4 independent file-read calls takes 4× longer than necessary.  
**Fix:**
- In `process_tool_calls()` (headless) and the equivalent REPL loop, collect all tool calls into a `Vec`
- Use `futures::future::join_all()` to run non-conflicting tool calls concurrently via `tokio::spawn`
- Note: `update_memory` and `load_skill` intercept calls must still run sequentially (they mutate shared state). Classify tool calls as `memory_mutating` vs `regular` and run only the latter in parallel
- Collect all results, then submit them all back to the agent in one batch `stream_tool_return` call

---

### H-02 · MCP server reconnect on crash
**Status:** `[ ]`  
**Files:** `src/mcp/mod.rs`  
**Problem:** If an MCP server crashes mid-session, `McpManager::call_tool()` returns `Err("MCP call failed")` with no recovery. The tool silently vanishes until CADE restarts.  
**Fix:**
- Add `last_error: Option<Instant>` and `reconnect_attempts: u32` to `McpServer`
- On `call_tool()` failure, attempt reconnect via `connect_server()` up to 3 times with 2 s delay
- Wrap `McpServer` in `Arc<Mutex<>>` to allow interior mutability for reconnect state
- Log reconnect attempts via `tracing::warn!`
- After 3 failed reconnects, mark server as `disabled` and surface a TUI warning to the user

---

### H-03 · Headless mode global timeout
**Status:** `[ ]`  
**Files:** `src/cli/headless.rs`, `src/cli/args.rs`  
**Problem:** `run_headless()` has no overall timeout. A runaway agent in CI can block a pipeline indefinitely. There is no `--timeout-secs` flag.  
**Fix:**
- Add `--timeout-secs <N>` to `Args` in `args.rs` (default: `0` = no timeout)
- Wrap `run_headless()` call in `tokio::time::timeout(Duration::from_secs(n), ...)` in `main.rs`
- On timeout: print a clear error message to stderr, exit with code `124` (standard timeout exit code)
- Also apply timeout to `run_headless_stream_json()`

---

### H-04 · Configurable server port propagated to client
**Status:** `[ ]`  
**Files:** `src/server/config.rs:101–104`, `src/settings/manager.rs:253`, `src/main.rs`  
**Problem:** `CADE_SERVER_PORT` is already read in `config.rs` for the server, but the client hardcodes `http://localhost:8284` as its fallback in `settings/manager.rs`. If a user sets `CADE_SERVER_PORT=9000` for the server, the client still connects to 8284.  
**Fix:**
- In `settings/manager.rs::base_url()`, also check `CADE_SERVER_PORT` to construct the URL:
```rust
let port = std::env::var("CADE_SERVER_PORT").ok()
    .and_then(|p| p.parse::<u16>().ok())
    .unwrap_or(8284);
format!("http://localhost:{port}")
```
- Add `--port` flag to `cade-server` CLI args as an alternative to the env var
- Document both in README

---

## 🟡 Medium

---

### M-01 · Subagent concurrency cap
**Status:** `[ ]`  
**Files:** `src/subagents/mod.rs`  
**Problem:** No limit on how many subagents can run in parallel. A single prompt could spawn unbounded concurrent LLM calls, exhausting API rate limits or OOM.  
**Fix:**
- Add `Arc<Semaphore>` with a configurable permit count (default: 4, env var `CADE_MAX_SUBAGENTS`)
- Each subagent task acquires a permit before spawning and releases on completion
- Queue waiting tasks rather than rejecting — communicate wait status to user via TUI

---

### M-02 · Live skill file watcher
**Status:** `[ ]`  
**Files:** `src/skills/mod.rs`, `src/cli/repl.rs` (`/skills reload` handler)  
**Problem:** Skills are discovered once at startup via `discover_all_skills()`. Adding or editing a SKILL.MD requires restarting CADE. `/skills reload` exists in the UI but doesn't auto-trigger.  
**Fix:**
- Add `notify` crate to `Cargo.toml`
- Spawn a background watcher task on `.skills/`, `~/.cade/skills/`, and agent skills dir
- On any `Create`/`Modify`/`Remove` event: re-run `discover_all_skills()`, update the in-memory skill list, and push a TUI notification `"Skills reloaded (N skills)"`

---

### M-03 · Agent export / import
**Status:** `[ ]`  
**Files:** `src/cli/args.rs`, `src/cli/repl.rs`, `src/server/api/agents.rs`, `src/server/storage/sqlite.rs`  
**Problem:** Agents exist only in `~/.cade/cade.db`. No portability — can't share an agent, move it to another machine, or back it up without copying the whole DB.  
**Fix:**
- Add `cade export-agent <name-or-id> [--output file.json]` CLI subcommand
- Export payload: `{ agent, memory_blocks, conversations: [{ messages }] }`
- Add `cade import-agent <file.json>` CLI subcommand — creates a new agent from the JSON
- Add `/export` and `/import` REPL slash commands
- Format: pretty-printed JSON (human-readable, diffable, committable)

---

### M-04 · Semantic memory / conversation search
**Status:** `[ ]`  
**Files:** `src/server/storage/sqlite.rs`, `src/cli/repl.rs` (`/search` handler), `src/server/api/messages.rs`  
**Problem:** `/search` does a simple `LIKE '%query%'` SQL query on message content. No ranking, no relevance, no semantic understanding. Long-running agents lose practical access to older context.  
**Fix (pragmatic — no external service):**
- Integrate `sqlite-vss` (SQLite vector search extension) or use BM25 via SQLite FTS5 (already available in `rusqlite`)
- Add FTS5 virtual table mirroring the `messages` table
- Re-implement `/search` handler to use FTS5 `MATCH` queries with `rank` ordering
- Return top-N results with context snippets
- Phase 2 (optional): embed via a local model (e.g. `fastembed-rs`) for true semantic search

---

### M-05 · Rate limiting on API endpoints
**Status:** `[ ]`  
**Files:** `src/server/api/mod.rs`, `src/server/mod.rs`  
**Problem:** No rate limiting on any REST endpoint. A buggy client or runaway script could flood the server with LLM calls.  
**Fix:**
- Add `tower_governor` or a custom `tower::Layer` middleware
- Apply to `/v1/messages` (the most expensive endpoint): max 10 req/s per IP
- Apply to `/v1/runs` stream: max 5 concurrent streams
- Return `429 Too Many Requests` with `Retry-After` header on excess

---

## 🟢 Low / Polish

---

### L-01 · Replace hand-rolled regex in hook matcher
**Status:** `[ ]`  
**Files:** `src/hooks/mod.rs:292–306`  
**Problem:** `regex_match()` implements its own `|` alternation and `.*` wildcard. Can't express `Bash(git *)` or `Read(src/**/*.rs)`. Power users will hit this ceiling quickly.  
**Fix:**
- Add `regex` crate to `Cargo.toml`
- Replace `regex_match()` with `regex::Regex::new(pattern)?.is_match(text)`
- Cache compiled `Regex` objects in `HookEntry` at parse time (not per-call)
- Keep backward compatibility: plain `*` and `""` still match all

---

### L-02 · Consistent startup logging via tracing
**Status:** `[ ]`  
**Files:** `src/main.rs:400,435,464`  
**Problem:** `eprintln!("cade-server not running — starting…")`, `"cade-server ready."`, `"Connected to cade-server…"` go to raw stderr, bypassing the `tracing` subscriber. Can't be filtered or redirected by `RUST_LOG`.  
**Fix:**
- Replace all startup `eprintln!` with `tracing::info!`
- Add `println!` only for the interactive TUI banner (user-facing, intentional)
- Ensure `tracing_subscriber` is initialised before these calls in `main.rs`

---

### L-03 · Timestamps in AgentResponse API
**Status:** `[ ]`  
**Files:** `src/server/api/agents.rs:39–46`, `src/server/storage/sqlite.rs`  
**Problem:** `AgentResponse` omits `created_at` and `updated_at`. Clients can't sort agents by recency. The fields exist in `AgentRow` but are not serialised.  
**Fix:**
- Add `created_at: Option<String>` and `updated_at: Option<String>` to `AgentResponse`
- Populate from `AgentRow` in the `From<AgentRow>` impl
- Update all agent list/get endpoints to include these fields

---

### L-04 · Versioning + CHANGELOG
**Status:** `[ ]`  
**Files:** `Cargo.toml`, `CHANGELOG.md` (new)  
**Problem:** `Cargo.toml` is at `0.1.0` with no version strategy. The client binary and server binary must be compatible — no mechanism to detect or enforce this.  
**Fix:**
- Adopt semver: `MAJOR.MINOR.PATCH`
- Add `X-Cade-Version` header to all server responses
- Client checks header on startup — warns if client/server versions differ
- Create `CHANGELOG.md` (Keep a Changelog format)
- Add `version` field to `GET /v1/health` response

---

## Progress Summary

| ID | Title | Priority | Status |
|----|-------|----------|--------|
| C-01 | LLM retry + exponential backoff | 🔴 Critical | `[x]` |
| C-02 | Dynamic context budget | 🔴 Critical | `[x]` |
| C-03 | Fix unsafe fd in server auto-start | 🔴 Critical | `[x]` |
| H-01 | Parallel tool dispatch | 🔴 High | `[x]` |
| H-02 | MCP server reconnect on crash | 🔴 High | `[x]` |
| H-03 | Headless mode global timeout | 🔴 High | `[x]` |
| H-04 | Configurable port client/server sync | 🔴 High | `[x]` |
| M-01 | Subagent concurrency cap | 🟡 Medium | `[x]` |
| M-02 | Live skill file watcher | 🟡 Medium | `[x]` |
| M-03 | Agent export / import | 🟡 Medium | `[ ]` |
| M-04 | Semantic memory / search | 🟡 Medium | `[ ]` |
| M-05 | API rate limiting | 🟡 Medium | `[ ]` |
| L-01 | Real regex in hook matcher | 🟢 Low | `[ ]` |
| L-02 | Consistent startup logging | 🟢 Low | `[ ]` |
| L-03 | Timestamps in AgentResponse | 🟢 Low | `[ ]` |
| L-04 | Versioning + CHANGELOG | 🟢 Low | `[ ]` |

---

*Last updated: 2026-03-04*
