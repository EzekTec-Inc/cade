use super::*;

fn m(role: &str, text: &str) -> (String, String) {
    (role.to_string(), text.to_string())
}

#[test]
fn empty_produces_no_turns() {
    assert!(group_turns(&[], 64_000).is_empty());
}

#[test]
fn single_exchange_produces_one_turn() {
    let turns = group_turns(&[m("user", "hello")], 64_000);
    assert_eq!(turns.len(), 1);
}

#[test]
fn two_exchanges_produce_two_turns() {
    let msgs = vec![
        m("user", "q1"),
        m("assistant", "a1"),
        m("user", "q2"),
        m("assistant", "a2"),
    ];
    let turns = group_turns(&msgs, 64_000);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0][0].0, "user");
    assert_eq!(turns[1][0].0, "user");
}

#[test]
fn tool_result_stays_in_same_turn_as_its_call() {
    let msgs = vec![
        m("user", "do thing"),
        m("assistant", ""), // triggers tool call
        m("tool", "ok"),    // result
        m("assistant", "done"),
        m("user", "next"),
    ];
    let turns = group_turns(&msgs, 64_000);
    assert_eq!(turns.len(), 2);
    // First turn: user + assistant + tool + assistant = 4 messages
    assert_eq!(turns[0].len(), 4);
}

// ── Inflation guard tests ─────────────────────────────────────────────

#[test]
fn inflation_guard_rejects_when_summary_is_large() {
    assert!(is_summary_inflated(900, 1000));
}

#[test]
fn inflation_guard_accepts_when_summary_is_compact() {
    assert!(!is_summary_inflated(200, 1000));
}

#[test]
fn inflation_guard_boundary_at_80_percent() {
    assert!(!is_summary_inflated(800, 1000));
    assert!(is_summary_inflated(801, 1000));
}

#[test]
fn inflation_guard_handles_zero_dropped() {
    assert!(!is_summary_inflated(100, 0));
}

#[test]
fn inflation_guard_handles_empty_summary() {
    assert!(!is_summary_inflated(0, 1000));
}

// ── extract_artifacts tests ───────────────────────────────────────────

#[test]
fn extracts_rust_file_paths() {
    let text = "Modified src/server/consolidation.rs and crates/cade-core/src/lib.rs";
    let arts = extract_artifacts(text);
    assert!(arts.contains(&"src/server/consolidation.rs".to_string()));
    assert!(arts.contains(&"crates/cade-core/src/lib.rs".to_string()));
}

#[test]
fn extracts_function_names() {
    let text = "Called extract_artifacts() and build_context( with args";
    let arts = extract_artifacts(text);
    assert!(arts.iter().any(|a| a.contains("extract_artifacts")));
    assert!(arts.iter().any(|a| a.contains("build_context")));
}

#[test]
fn extracts_error_identifiers() {
    let text = "Found RUSTSEC-2025-0009 and error[E0433] in the build";
    let arts = extract_artifacts(text);
    assert!(arts.iter().any(|a| a.contains("RUSTSEC-2025-0009")));
    assert!(arts.iter().any(|a| a.contains("error[E0433]")));
}

#[test]
fn extracts_error_lines() {
    let text = "output:\nerror: cannot find type `Foo` in this scope\nmore stuff";
    let arts = extract_artifacts(text);
    assert!(arts.iter().any(|a| a.starts_with("error: cannot find")));
}

#[test]
fn caps_at_six_artifacts() {
    let text = "src/a.rs src/b.rs src/c.rs src/d.rs src/e.rs src/f.rs src/g.rs src/h.rs";
    let arts = extract_artifacts(text);
    assert!(arts.len() <= 6);
}

#[test]
fn empty_text_yields_no_artifacts() {
    assert!(extract_artifacts("").is_empty());
}

#[test]
fn plain_text_yields_no_artifacts() {
    assert!(extract_artifacts("hello world this is a normal sentence").is_empty());
}

#[test]
fn deduplicates_artifacts() {
    let text = "src/lib.rs and again src/lib.rs and src/lib.rs";
    let arts = extract_artifacts(text);
    assert_eq!(arts.len(), 1);
}

// ── Phase C: pure helper tests ───────────────────────────────────────

#[test]
fn truncate_head_to_preserves_tail() {
    let out = truncate_head_to("abcdefghij", 4);
    assert_eq!(out, "ghij");
}

#[test]
fn truncate_head_to_noop_when_under_cap() {
    let out = truncate_head_to("abc", 100);
    assert_eq!(out, "abc");
}

#[test]
fn truncate_head_to_handles_multibyte() {
    // 5 chars, each multi-byte
    let out = truncate_head_to("αβγδε", 3);
    assert_eq!(out.chars().count(), 3);
    assert_eq!(out, "γδε");
}

#[test]
fn first_nonempty_line_skips_blank_lines() {
    let out = first_nonempty_line("\n\n  \nhello world\nnext");
    assert_eq!(out, "hello world");
}

#[test]
fn first_nonempty_line_empty_input() {
    assert_eq!(first_nonempty_line(""), "");
    assert_eq!(first_nonempty_line("\n\n  \n"), "");
}

#[test]
fn first_nonempty_line_caps_at_200() {
    let long = "x".repeat(500);
    let out = first_nonempty_line(&long);
    assert_eq!(out.chars().count(), 200);
}

#[test]
fn sanitize_index_line_collapses_whitespace() {
    let out = sanitize_index_line("hello\n  world\t\tfoo");
    assert_eq!(out, "hello world foo");
}

#[test]
fn sanitize_index_line_caps_at_200() {
    let long = "a ".repeat(200);
    let out = sanitize_index_line(&long);
    assert_eq!(out.chars().count(), 200);
}

// ── M2: per-role preview limits + tighter noisy-tool filter ──────────

#[test]
fn m2_preview_limit_assistant_is_1200() {
    // P5: raised from 1200 → 2000
    assert_eq!(preview_limit_for_role("assistant"), 2_000);
}

#[test]
fn m2_preview_limit_tool_is_800() {
    // P5: raised from 800 → 1200
    assert_eq!(preview_limit_for_role("tool"), 1_200);
}

#[test]
fn m2_preview_limit_user_is_400() {
    // P5: raised from 400 → 600
    assert_eq!(preview_limit_for_role("user"), 600);
}

#[test]
fn m2_preview_limit_unknown_role_falls_back_to_user_limit() {
    // Unknown roles must get the smallest limit so an unexpected role cannot
    // flood the summary prompt.
    assert_eq!(preview_limit_for_role("system"), 400);
    assert_eq!(preview_limit_for_role(""), 400);
}

// ── M3: eager consolidation trigger (turn-count based) ───────────────

#[test]
fn m3_eager_first_time_triggers_when_at_or_above_threshold() {
    // With last_consolidation_turn = 0 and current = threshold, trigger.
    assert!(should_eager_consolidate(
        /* current */ EAGER_CONSOLIDATION_TURN_THRESHOLD,
        /* last    */ 0,
        EAGER_CONSOLIDATION_TURN_THRESHOLD,
    ));
}

#[test]
fn m3_eager_does_not_trigger_before_threshold() {
    // current - last < threshold → no eager consolidation.
    assert!(!should_eager_consolidate(
        /* current */ EAGER_CONSOLIDATION_TURN_THRESHOLD - 1,
        /* last    */ 0,
        EAGER_CONSOLIDATION_TURN_THRESHOLD,
    ));
}

#[test]
fn m3_eager_does_not_double_fire_within_threshold_window() {
    // After a previous eager run stamped last = 25, we must not re-fire at
    // turn 30 if threshold = 10 (gap 5 < 10).
    assert!(!should_eager_consolidate(30, 25, 10));
}

#[test]
fn m3_eager_fires_again_after_threshold_gap() {
    // After a previous eager run stamped last = 25, turn 35 (gap 10) should re-fire.
    assert!(should_eager_consolidate(35, 25, 10));
}

#[test]
fn m3_eager_handles_current_equal_to_last() {
    // Edge case: current == last (shouldn't normally happen but must be safe).
    assert!(!should_eager_consolidate(10, 10, 5));
}

#[test]
fn m3_eager_handles_current_less_than_last() {
    // Defensive: if the counter is ever somehow below last_consolidation_turn,
    // saturating arithmetic must prevent a panic and must not trigger.
    assert!(!should_eager_consolidate(5, 10, 5));
}

#[test]
fn m3_eager_threshold_constant_is_sane() {
    // The threshold must be > 0 (else eager fires on every turn) and should
    // be well below the 80-turn STALE_THRESHOLD so consolidation wins the
    // race against promote_stale_blocks. A value in 10..=40 is reasonable.
    #[allow(clippy::assertions_on_constants)]
    {
        assert!(EAGER_CONSOLIDATION_TURN_THRESHOLD >= 10);
        assert!(EAGER_CONSOLIDATION_TURN_THRESHOLD <= 40);
    }
}

// ── Phase C: DB-backed ring tests ────────────────────────────────────

use cade_store::sqlite::{self as store_sqlite, AgentRow, Db};

fn setup_db() -> Db {
    let db = store_sqlite::open(":memory:").expect("open in-memory db");
    store_sqlite::create_agent(
        &db,
        &AgentRow {
            id: "a1".into(),
            name: "A".into(),
            model: "m".into(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
            active_plan_json: None,
        },
    )
    .unwrap();
    db
}

fn block_value(db: &Db, label: &str) -> Option<String> {
    store_sqlite::get_memory_blocks(db, "a1")
        .unwrap()
        .into_iter()
        .find(|(l, _, _)| l == label)
        .map(|(_, v, _)| v)
}

#[test]
fn rotate_writes_prev_live_to_slot_1() {
    let db = setup_db();
    rotate_and_archive_session_summary_db(&db, "a1", "FIRST summary content");
    assert_eq!(
        block_value(&db, "session_summary_1").as_deref(),
        Some("FIRST summary content")
    );
    assert!(block_value(&db, "session_summary_2").is_none());
}

#[test]
fn rotate_empty_input_is_noop() {
    let db = setup_db();
    rotate_and_archive_session_summary_db(&db, "a1", "   \n  ");
    assert!(block_value(&db, "session_summary_1").is_none());
}

#[test]
fn rotate_shifts_slots_and_fills_slot_1() {
    let db = setup_db();
    rotate_and_archive_session_summary_db(&db, "a1", "ONE");
    rotate_and_archive_session_summary_db(&db, "a1", "TWO");
    rotate_and_archive_session_summary_db(&db, "a1", "THREE");
    assert_eq!(
        block_value(&db, "session_summary_1").as_deref(),
        Some("THREE")
    );
    assert_eq!(
        block_value(&db, "session_summary_2").as_deref(),
        Some("TWO")
    );
    assert_eq!(
        block_value(&db, "session_summary_3").as_deref(),
        Some("ONE")
    );
    assert!(block_value(&db, "session_summary_4").is_none());
}

#[test]
fn rotate_evicts_to_session_index_when_ring_full() {
    let db = setup_db();
    // Fill RING_CAP slots (P5: raised from 5 → 8).
    rotate_and_archive_session_summary_db(&db, "a1", "Summary ONE first line\nmore");
    rotate_and_archive_session_summary_db(&db, "a1", "Summary TWO first line\nmore");
    rotate_and_archive_session_summary_db(&db, "a1", "Summary THREE first line\nmore");
    rotate_and_archive_session_summary_db(&db, "a1", "Summary FOUR first line\nmore");
    rotate_and_archive_session_summary_db(&db, "a1", "Summary FIVE first line\nmore");
    rotate_and_archive_session_summary_db(&db, "a1", "Summary SIX first line\nmore");
    rotate_and_archive_session_summary_db(&db, "a1", "Summary SEVEN first line\nmore");
    rotate_and_archive_session_summary_db(&db, "a1", "Summary EIGHT first line\nmore");
    // All 8 slots should now be occupied, no index yet.
    assert!(block_value(&db, "session_summary_8").is_some());
    assert!(block_value(&db, "session_index").is_none());

    // One more rotation — "ONE" should be evicted to session_index.
    rotate_and_archive_session_summary_db(&db, "a1", "Summary NINE first line\nmore");
    let index = block_value(&db, "session_index").expect("index block must exist");
    assert!(
        index.contains("Summary ONE first line"),
        "expected ONE's first line in index, got: {index}"
    );
    // Ring still bounded at 8.
    assert!(block_value(&db, "session_summary_8").is_some());
    assert!(block_value(&db, "session_summary_9").is_none());
    // Slot 1 has the newest.
    assert_eq!(
        block_value(&db, "session_summary_1").as_deref(),
        Some("Summary NINE first line\nmore")
    );
}

#[test]
fn session_index_fifo_truncates_when_over_cap() {
    let db = setup_db();
    // Pre-seed session_index near the cap.
    let big = "X".repeat(SESSION_INDEX_MAX_CHARS - 10);
    store_sqlite::upsert_memory_block(
        &db,
        "a1",
        "session_index",
        &big,
        Some("seed"),
        Some(SESSION_INDEX_MAX_CHARS + 1000),
    )
    .unwrap();

    // Append a line long enough to push over cap — should trigger drain.
    append_to_session_index_db(&db, "a1", &"y".repeat(100));
    let v = block_value(&db, "session_index").unwrap();
    assert!(
        v.chars().count() <= SESSION_INDEX_MAX_CHARS,
        "expected ≤ {} chars, got {}",
        SESSION_INDEX_MAX_CHARS,
        v.chars().count()
    );
}

#[test]
fn rotated_slot_capped_at_archived_max_chars() {
    let db = setup_db();
    let huge = "Z".repeat(SESSION_SUMMARY_ARCHIVED_MAX_CHARS * 3);
    rotate_and_archive_session_summary_db(&db, "a1", &huge);
    let v = block_value(&db, "session_summary_1").unwrap();
    assert_eq!(v.chars().count(), SESSION_SUMMARY_ARCHIVED_MAX_CHARS);
    // Tail-preserving truncation: still all Zs.
    assert!(v.chars().all(|c| c == 'Z'));
}

// ─────────────────────────────────────────────────────────────────────
// M4 — End-to-end consolidation round-trip regression test
// ─────────────────────────────────────────────────────────────────────
//
// Protects the full pipeline: many dropped turns → `consolidate_agent`
// → `session_summary` memory block written with LLM output → block is
// `pinned` so the next context build surfaces it even after restart.
//
// This is the first test that exercises the whole round-trip through
// the real consolidation code path using an in-process mock LLM.
//
// Gap this test closes: prior to M4 no test verified that `consolidate_agent`
// actually writes a usable `session_summary` block — only rotation, turn
// grouping, and inflation-guard pieces were covered in isolation.

use crate::server::config::{LlmProviderKind, ServerConfig};
use crate::server::rate_limit::RateLimiter;
use crate::server::state::AppState;
use async_trait::async_trait;
use cade_ai::Result as AiResult;
use cade_ai::{
    AiConfig, CompletionRequest, CompletionResponse, LlmProvider, LlmRouter, StreamChunk,
};
use futures::Stream;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock as AsyncRwLock;

/// Mock LLM provider that returns a fixed summary string and counts calls.
struct MockSummaryLlm {
    summary: String,
    calls: AtomicUsize,
}

impl MockSummaryLlm {
    fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for MockSummaryLlm {
    async fn complete(&self, _req: &CompletionRequest) -> AiResult<CompletionResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(CompletionResponse {
            content: Some(self.summary.clone()),
            tool_calls: Vec::new(),
            finish_reason: "stop".into(),
        })
    }

    async fn stream(
        &self,
        _req: &CompletionRequest,
    ) -> AiResult<Pin<Box<dyn Stream<Item = AiResult<StreamChunk>> + Send>>> {
        // Consolidation only ever calls complete(); stream must exist to satisfy
        // the trait but is never invoked in this test.
        Err(cade_ai::Error::custom("stream not supported in mock"))
    }
}

/// Build a minimal AppState around an in-memory DB and a mock LLM.
fn mk_state(db: cade_store::sqlite::Db, llm: Arc<dyn LlmProvider>) -> AppState {
    let ai_cfg = AiConfig {
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        llm_provider: "ollama".into(),
    };
    let router = Arc::new(AsyncRwLock::new(LlmRouter::build(&ai_cfg)));

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let cfg = ServerConfig {
        addr,
        db_path: ":memory:".into(),
        llm_provider: LlmProviderKind::Ollama,
        default_model: "m".into(),
        max_tokens_per_turn: Some(64_000),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: "http://localhost:11434".into(),
        api_key: None,

        allowed_origin: None,
        max_context_budget: None,
    };

    AppState {
        db,
        llm,
        llm_router: router,
        config: Arc::new(cfg),
        mcp: Arc::new(crate::server::state::McpManager::empty()),
        rate_limiter: RateLimiter::from_env(),
        memory_cache: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        agent_activity: Arc::new(AsyncRwLock::new(std::collections::HashMap::new())),
        agent_metrics: Arc::new(dashmap::DashMap::new()),
        agent_context_telemetry: Arc::new(AsyncRwLock::new(std::collections::HashMap::new())),
        context_cache: Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
            crate::server::state::CONTEXT_CACHE_CAPACITY,
        ))),
        all_skills: Arc::new(AsyncRwLock::new(Vec::new())),
        agent_skills: Arc::new(AsyncRwLock::new(std::collections::HashMap::new())),
        pending_subagent_results: Arc::new(AsyncRwLock::new(std::collections::HashMap::new())),
        subagent_cancellations: Arc::new(AsyncRwLock::new(std::collections::HashMap::new())),
        subagent_semaphore: Arc::new(tokio::sync::Semaphore::new(4)),
        embedder: None,
    }
}

/// Seed `n` fake user↔assistant turns (content each ~`payload_chars` chars) so
/// that consolidation has enough dropped content to summarise.
fn seed_turns(db: &cade_store::sqlite::Db, agent_id: &str, n: usize, payload_chars: usize) {
    use cade_store::sqlite::MessageRow;
    for i in 0..n {
        let user_body = format!(
            "turn {i}: please edit src/mod_{i}.rs and fix `fn compute_{i}`. {}",
            "x".repeat(payload_chars)
        );
        let asst_body = format!(
            "turn {i}: I edited src/mod_{i}.rs — updated `fn compute_{i}`. error code E{:04}. {}",
            i,
            "y".repeat(payload_chars)
        );
        store_sqlite::insert_message(
            db,
            &MessageRow {
                id: format!("u-{i}"),
                agent_id: agent_id.into(),
                conversation_id: None,
                role: "user".into(),
                content: serde_json::json!({ "content": user_body }),
                char_count: user_body.chars().count(),
            },
        )
        .unwrap();
        store_sqlite::insert_message(
            db,
            &MessageRow {
                id: format!("a-{i}"),
                agent_id: agent_id.into(),
                conversation_id: None,
                role: "assistant".into(),
                content: serde_json::json!({ "content": asst_body }),
                char_count: asst_body.chars().count(),
            },
        )
        .unwrap();
    }
}

#[tokio::test]
async fn m4_consolidation_round_trip_writes_pinned_session_summary() {
    // ── arrange ─────────────────────────────────────────────────────
    let db = setup_db(); // agent "a1", model "m" (unknown → 32 000 token window)
    let agent_id = "a1";

    // Seed enough turns that the older ones will not fit in HISTORY_BUDGET_FRACTION (40%)
    // of the estimated char budget. With model "m" → 32 000 tokens → ~81 600 char budget
    // → ~32 640 char history budget. 40 turns × ~4200 chars/turn ≈ 168 000 chars ⇒ most
    // turns must be classified as dropped, guaranteeing consolidate_agent reaches the
    // "write session_summary" branch.
    seed_turns(&db, agent_id, 40, 2_000);

    let mock_summary =
        "MOCK_ROUND_TRIP_SUMMARY: rewrote src/mod_3.rs, fixed fn compute_7, error E0042 resolved.";
    let llm = Arc::new(MockSummaryLlm::new(mock_summary));
    let llm_trait: Arc<dyn LlmProvider> = llm.clone();
    let state = mk_state(db.clone(), llm_trait);

    // ── act ─────────────────────────────────────────────────────────
    consolidate_agent(&state, agent_id, None, None).await;

    // ── assert ──────────────────────────────────────────────────────

    // 1. The mock LLM's complete() was invoked exactly three times:
    //    (a) session_summary consolidation
    //    (b) P7 auto_update_active_goal
    //    (c) Phase B auto_extract_facts (added during memory architecture rework)
    assert_eq!(
        llm.calls.load(Ordering::SeqCst),
        2,
        "consolidate_agent must call LLM.complete two times (summary + auto_extract_facts)"
    );

    // 2. `session_summary` block exists and contains the mock output verbatim.
    let blocks = store_sqlite::get_memory_blocks(&db, agent_id).unwrap();
    let summary_block = blocks
        .iter()
        .find(|(l, _, _)| l == "session_summary")
        .expect("session_summary block must be written after consolidation");
    assert!(
        summary_block.1.contains("MOCK_ROUND_TRIP_SUMMARY"),
        "session_summary must contain LLM's summary text; got: {}",
        summary_block.1
    );

    // 3. `session_summary` is `pinned` tier so it is not subject to
    //    promote_stale_blocks demotion on future context builds.
    let active = store_sqlite::get_active_blocks(&db, agent_id).unwrap();
    let (_, _, _, tier, _) = active
        .iter()
        .find(|(l, _, _, _, _)| l == "session_summary")
        .expect("session_summary must appear in active (pinned+short) blocks");
    assert_eq!(
        tier, "pinned",
        "session_summary must be pinned so next build_context always injects it"
    );
}

// ── F2: cache full dropped turns to archival before compaction ──────────

/// F2: when consolidate_agent runs, the full text of dropped turns must
/// be written to archival memory BEFORE the LLM summarisation lossy
/// step.  This guarantees the raw dialogue is recoverable later via
/// `archival_memory_search` even after `session_summary` is rotated or
/// overwritten.
#[tokio::test]
async fn f2_consolidation_caches_dropped_turns_to_archival() {
    let db = setup_db();
    let agent_id = "a1";

    // Seed enough turns to force a non-trivial number of dropped turns.
    // The user/assistant bodies contain unique tokens we can search for
    // afterwards to confirm the raw dialogue made it into archival.
    seed_turns(&db, agent_id, 40, 2_000);

    // Sanity: archival is empty before the consolidation pass.
    let pre_hits = store_sqlite::search_archival_memory(&db, agent_id, "compute_5", 10).unwrap();
    assert!(
        pre_hits.is_empty(),
        "archival must be empty before consolidate_agent runs; got {} hits",
        pre_hits.len()
    );

    let llm = Arc::new(MockSummaryLlm::new("F2 mock summary"));
    let llm_trait: Arc<dyn LlmProvider> = llm.clone();
    let state = mk_state(db.clone(), llm_trait);

    consolidate_agent(&state, agent_id, None, None).await;

    // The raw seed text included `compute_<n>` tokens for each turn —
    // every dropped turn's user message contains one. Searching archival
    // for that token must hit the cached payload.
    let hits = store_sqlite::search_archival_memory(&db, agent_id, "compute_5", 10).unwrap();
    assert!(
        !hits.is_empty(),
        "F2: archival must contain the dropped turns after consolidation"
    );

    // The cache row carries the F2 tags so it can be filtered later.
    let combined_tags: Vec<String> = hits.iter().flat_map(|r| r.tags.clone()).collect();
    assert!(
        combined_tags.iter().any(|t| t == "consolidation"),
        "F2 archival entry must be tagged 'consolidation', got {combined_tags:?}"
    );
    assert!(
        combined_tags.iter().any(|t| t == "dropped-turns"),
        "F2 archival entry must be tagged 'dropped-turns', got {combined_tags:?}"
    );
    assert!(
        combined_tags
            .iter()
            .any(|t| t == &format!("agent:{agent_id}")),
        "F2 archival entry must be tagged with agent id, got {combined_tags:?}"
    );
}

/// F2: when the LLM call later fails, the archival cache must already be
/// in place — the raw turns are preserved even though no session_summary
/// was written.
#[tokio::test]
async fn f2_archival_cache_survives_llm_failure() {
    // Re-use the round-trip seed setup, but swap the LLM for one that
    // always errors.  consolidate_agent should still write to archival
    // BEFORE attempting the LLM call.
    struct FailingLlm;
    #[async_trait::async_trait]
    impl LlmProvider for FailingLlm {
        async fn complete(
            &self,
            _req: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            Err(cade_ai::Error::custom("forced LLM failure for F2 test"))
        }
        async fn stream(
            &self,
            _req: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            Err(cade_ai::Error::custom(
                "forced LLM stream failure for F2 test",
            ))
        }
    }

    let db = setup_db();
    let agent_id = "a1";
    seed_turns(&db, agent_id, 40, 2_000);

    let llm: Arc<dyn LlmProvider> = Arc::new(FailingLlm);
    let state = mk_state(db.clone(), llm);

    consolidate_agent(&state, agent_id, None, None).await;

    // No session_summary was written (LLM failed) — but the archival cache
    // must still hold the raw dropped turns.
    let hits = store_sqlite::search_archival_memory(&db, agent_id, "compute_3", 10).unwrap();
    assert!(
        !hits.is_empty(),
        "F2: archival cache must persist even when the LLM call fails"
    );

    let blocks = store_sqlite::get_memory_blocks(&db, agent_id).unwrap();
    let summary_block = blocks.iter().find(|(l, _, _)| l == "session_summary");
    assert!(
        summary_block.is_none(),
        "session_summary should NOT exist when the LLM failed; got: {:?}",
        summary_block
    );
}

// ── default_compaction_model ─────────────────────────────────────────────

#[test]
fn default_compaction_anthropic_uses_haiku() {
    assert_eq!(
        default_compaction_model("anthropic/claude-sonnet-4-5-20250929"),
        "anthropic/claude-haiku-4-5"
    );
    assert_eq!(
        default_compaction_model("anthropic/claude-opus-4-20250514"),
        "anthropic/claude-haiku-4-5"
    );
}

#[test]
fn default_compaction_openai_uses_4o_mini() {
    assert_eq!(
        default_compaction_model("openai/gpt-4o"),
        "openai/gpt-4o-mini"
    );
    assert_eq!(
        default_compaction_model("openai/gpt-4.1"),
        "openai/gpt-4o-mini"
    );
}

#[test]
fn default_compaction_gemini_uses_flash() {
    assert_eq!(
        default_compaction_model("gemini/gemini-2.5-pro"),
        "gemini/gemini-2.5-flash"
    );
}

#[test]
fn default_compaction_openrouter_uses_free_glm() {
    // Paid-tier OpenRouter → cheap compaction model
    assert_eq!(
        default_compaction_model("openrouter/anthropic/claude-sonnet-4"),
        "openrouter/z-ai/glm-4.5-air:free"
    );
}

#[test]
fn default_compaction_openrouter_free_tier_passthrough() {
    // Free-tier OpenRouter models share the same rate-limit quota.
    // Using a different free model would just compete for that quota.
    assert_eq!(
        default_compaction_model("openrouter/google/gemma-4-26b-a4b-it:free"),
        "openrouter/google/gemma-4-26b-a4b-it:free"
    );
    assert_eq!(
        default_compaction_model("openrouter/mistral/mistral-small-3.1-24b-instruct:free"),
        "openrouter/mistral/mistral-small-3.1-24b-instruct:free"
    );
}

#[test]
fn default_compaction_ollama_passthrough() {
    // Local models cost nothing; reuse same model.
    assert_eq!(
        default_compaction_model("ollama/llama3:70b"),
        "ollama/llama3:70b"
    );
}

#[test]
fn default_compaction_unknown_provider_passthrough() {
    // Unknown / custom providers: do not assume a cheaper variant exists.
    assert_eq!(
        default_compaction_model("some-custom/foo"),
        "some-custom/foo"
    );
}

#[test]
fn default_compaction_already_cheap_anthropic_idempotent() {
    // Already-haiku must not loop or recurse.
    assert_eq!(
        default_compaction_model("anthropic/claude-haiku-4-5"),
        "anthropic/claude-haiku-4-5"
    );
}

#[test]
fn default_compaction_already_cheap_openai_idempotent() {
    assert_eq!(
        default_compaction_model("openai/gpt-4o-mini"),
        "openai/gpt-4o-mini"
    );
}

#[test]
fn default_compaction_already_cheap_gemini_idempotent() {
    assert_eq!(
        default_compaction_model("gemini/gemini-2.5-flash"),
        "gemini/gemini-2.5-flash"
    );
}

#[test]
fn default_compaction_anthropic_uses_current_haiku_model_id() {
    // claude-3-5-haiku-latest was retired by Anthropic (404).
    // Must resolve to claude-haiku-4-5.
    assert_eq!(
        default_compaction_model("anthropic/claude-opus-4-6"),
        "anthropic/claude-haiku-4-5"
    );
}
