# Plan: cade-server Test Coverage Gap Resolution

## Problem Statement

`cade-server` has **37 tests** covering 5 of 37 source files (5,911 lines untested).
The biggest real gaps are:

1. **7 SQLite storage modules** — 53 pure functions, 1,411 lines, zero tests
2. **14 HTTP handler modules** — 62 axum handlers, 2,145 lines, zero tests

---

## Phase 1: SQLite Storage Tests (HIGH PRIORITY)

**Why first:** These are pure `fn(db, ...) → Result<...>` functions — fully testable
with the existing `setup_mem_db()` pattern from `evidence.rs`. No new infrastructure
needed. This is the highest bang-for-buck work.

**Shared test helper:** Extract the existing `setup_mem_db()` into a common
`#[cfg(test)]` helper module in `sqlite/mod.rs` so all sqlite test modules can
reuse it without duplication.

### Phase 1A: `sqlite/agents.rs` (11 functions)

| Function | Test |
|----------|------|
| `create_agent` | create + verify roundtrip |
| `get_agent` | existing agent, missing agent (None) |
| `list_agents` | empty, one, many |
| `delete_agent` | existing → true, missing → false |
| `update_agent_model` | update + verify |
| `update_agent_name` | update + verify |
| `update_agent_system_prompt` | update + verify |
| `attach_tools_to_agent` | attach + verify via get_agent_tool_ids |
| `get_agent_tool_ids` | with tools, without tools |
| `get_agent_tools_with_names` | joined query returns (id, name) |
| `detach_all_tools_from_agent` | detach + verify count returned |

**Estimated tests: ~14**

### Phase 1B: `sqlite/memory.rs` (14 functions)

| Function | Test |
|----------|------|
| `upsert_memory_block` | insert new, update existing |
| `link_shared_memory_block` | link + verify cross-agent access |
| `delete_memory_block` | existing → true, missing → false |
| `get_memory_blocks` | empty, populated |
| `get_memory_blocks_with_ts` | verify timestamps returned |
| `increment_turn_counter` | increment + read back |
| `get_turn_counter` | initial (0), after increments |
| `promote_stale_blocks` | tier promotion logic |
| `get_active_blocks` | pinned + short-term blocks |
| `get_long_term_excerpts` | archived blocks with excerpts |
| `set_memory_tier` | tier transitions |
| `get_memory_blocks_full` | full metadata including tier |
| `get_memory_history` | history tracking |
| `restore_memory_from_history` | rollback to prior version |

**Estimated tests: ~18**

### Phase 1C: `sqlite/conversations.rs` (6 functions)

| Function | Test |
|----------|------|
| `create_conversation` | create + verify |
| `get_conversation` | existing, missing |
| `list_conversations` | empty, multiple, filtered by agent |
| `delete_conversation` | existing → true, missing → false |
| `update_conversation_title` | update + verify |
| `touch_conversation` | updated_at changes |

**Estimated tests: ~8**

### Phase 1D: `sqlite/messages.rs` (5 functions)

| Function | Test |
|----------|------|
| `last_assistant_message` | found, not found |
| `insert_message` | insert + roundtrip |
| `list_messages` | ordered by created_at |
| `list_messages_page` | pagination (limit, offset) |
| `get_context_window` | budget trimming |

**Estimated tests: ~8**

### Phase 1E: `sqlite/tools.rs` (9 functions)

| Function | Test |
|----------|------|
| `upsert_tool` | insert, update existing |
| `get_tool_id_by_name` | found, not found |
| `clear_messages` | clear all, clear by conversation |
| `search_messages` | FTS match, no match |
| `search_memory` | FTS on memory blocks |
| `insert_archival_memory` | insert + search roundtrip |
| `search_archival_memory` | FTS search with limit |
| `pending_tool_results` | with pending, without pending |
| `list_tools` | empty, populated |

**Estimated tests: ~14**

### Phase 1F: `sqlite/providers.rs` (3 functions)

| Function | Test |
|----------|------|
| `upsert_provider` | insert, update |
| `list_providers` | empty, populated |
| `delete_provider` | existing → true, missing → false |

**Estimated tests: ~5**

### Phase 1G: `sqlite/runs.rs` (5 functions)

| Function | Test |
|----------|------|
| `create_run` | create + verify |
| `get_run` | existing, missing |
| `finish_run` | status update |
| `append_run_event` | append + sequence numbering |
| `run_events_after` | filtering by sequence |

**Estimated tests: ~7**

### Phase 1 Summary

| Module | Functions | Est. Tests |
|--------|-----------|------------|
| agents.rs | 11 | 14 |
| memory.rs | 14 | 18 |
| conversations.rs | 6 | 8 |
| messages.rs | 5 | 8 |
| tools.rs | 9 | 14 |
| providers.rs | 3 | 5 |
| runs.rs | 5 | 7 |
| **Total** | **53** | **~74** |

**Effort estimate:** 2-3 sessions  
**Risk reduction:** HIGH — covers all data persistence layer

---

## Phase 2: HTTP Handler Integration Tests (MEDIUM PRIORITY)

**Why second:** These need an integration test harness that doesn't exist yet.
The harness is reusable once built, so the investment pays off across all 62 handlers.

### Phase 2A: Build test harness (`tests/api_harness.rs` or `api/test_helpers.rs`)

Create a shared helper that constructs a test `AppState`:

```rust
/// Build a fully functional AppState with an in-memory SQLite database
/// and a mock/noop LLM provider for handler testing.
pub fn test_app_state() -> AppState {
    let db = setup_mem_db();
    let config = ServerConfig { /* test defaults */ };
    let router = LlmRouter::new(); // empty, no real providers
    AppState {
        db,
        llm: Arc::new(NoopLlmProvider),
        llm_router: Arc::new(RwLock::new(router)),
        config: Arc::new(config),
        rate_limiter: RateLimiter::from_env(),
        memory_cache: Arc::new(Mutex::new(HashMap::new())),
        agent_activity: Arc::new(RwLock::new(HashMap::new())),
        #[cfg(feature = "reranker")]
        tool_reranker: None,
    }
}

/// Build a full axum Router with test state for use with axum::test.
pub fn test_router() -> Router {
    let state = test_app_state();
    crate::server::api::router(state)
}
```

This requires:
- A `NoopLlmProvider` that implements `LlmProvider` (returns empty responses)
- A test `ServerConfig` with sensible defaults
- Re-using the existing `setup_mem_db()` for the DB

**Effort estimate:** 1 session

### Phase 2B: Health + Config endpoints (2 handlers)

Simplest handlers — validate the harness works:

| Handler | Test |
|---------|------|
| `GET /v1/health` | Returns `{"status":"ok"}` |
| `GET /v1/config` | Returns provider + model info |

**Estimated tests: 2**

### Phase 2C: Agent CRUD (23 handlers — largest module)

| Handler | Tests |
|---------|-------|
| `POST /v1/agents` | Create agent, verify 201 |
| `GET /v1/agents/:id` | Existing → 200, missing → 404 |
| `GET /v1/agents` | List returns array |
| `DELETE /v1/agents/:id` | Existing → 200, missing → 404 |
| `PATCH /v1/agents/:id` | Update model, name, system prompt |
| `GET /v1/agents/:id/memory` | Returns memory blocks |
| `PUT /v1/agents/:id/memory/:label` | Set memory block |
| `DELETE /v1/agents/:id/memory/:label` | Delete block |
| `POST /v1/agents/:id/tools` | Attach tools |
| `GET /v1/agents/:id/conversations` | List conversations |
| ...remaining agent endpoints... | |

**Estimated tests: ~20**

### Phase 2D: Tools endpoints (2 handlers)

| Handler | Test |
|---------|------|
| `POST /v1/tools` | Register tool schema |
| `GET /v1/tools` | List registered tools |

**Estimated tests: 3**

### Phase 2E: Provider endpoints (3 handlers)

| Handler | Test |
|---------|------|
| `POST /v1/providers` | Add provider |
| `GET /v1/providers` | List providers |
| `DELETE /v1/providers/:name` | Remove provider |

**Estimated tests: 4**

### Phase 2F: Remaining endpoints

| Module | Handlers | Est. Tests |
|--------|----------|------------|
| artifacts | 4 | 5 |
| checkpoints | 5 | 6 |
| evals | 6 | 8 |
| memory_evidence | 5 | 6 |
| runs | 2 | 3 |
| auth middleware | 1 | 3 |
| proxy | 1 | 1 |
| models | 1 | 1 |
| tool_executions | 1 | 2 |

**Estimated tests: ~35**

### Phase 2 Summary

| Sub-phase | Tests |
|-----------|-------|
| 2A: Harness | 0 (infrastructure) |
| 2B: Health | 2 |
| 2C: Agents | 20 |
| 2D: Tools | 3 |
| 2E: Providers | 4 |
| 2F: Remaining | 35 |
| **Total** | **~64** |

**Effort estimate:** 3-4 sessions  
**Risk reduction:** MEDIUM — covers full API surface

---

## Phase 3: Business Logic Tests (LOW PRIORITY)

### Phase 3A: `config.rs` (5 functions)

| Function | Test |
|----------|------|
| `default_model_for` | Each provider returns expected model |
| `detect_provider` | Auto-detection priority logic |
| `from_env` | Default construction |
| `from_env_with_port` | Port override |
| `to_ai_config` | Conversion correctness |

**Estimated tests: 6**  
**Note:** `detect_provider` tests are env-dependent (Rust 2024 `unsafe` issue).
Test via manual construction instead.

### Phase 3B: `messages/persist.rs`

Depends on DB — covered by Phase 1 SQLite tests indirectly.

### Phase 3C: `reflection.rs`

Requires LLM call — not unit-testable without mocking. Skip or test with
the noop provider from Phase 2.

---

## Execution Order

```
Phase 1A → 1B → 1C → 1D → 1E → 1F → 1G   (SQLite — ~74 tests)
     ↓
Phase 2A                                      (Harness)
     ↓
Phase 2B → 2C → 2D → 2E → 2F               (HTTP — ~64 tests)
     ↓
Phase 3A                                      (Config — ~6 tests)
```

## Expected Outcome

| Metric | Before | After |
|--------|--------|-------|
| cade-server tests | 37 | **~181** |
| Files with tests | 5 / 37 | **~25 / 37** |
| SQLite functions tested | 0 / 53 | **53 / 53** |
| HTTP handlers tested | 0 / 62 | **~62 / 62** |

**Total workspace tests: 436 → ~580+**
