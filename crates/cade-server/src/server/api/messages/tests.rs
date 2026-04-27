use super::*;

fn user(text: &str) -> LlmMessage {
    LlmMessage {
        role: "user".to_string(),
        content: text.to_string(),
        tool_call_id: None,
        tool_calls: None,
        images: None,
    }
}

fn assistant(text: &str) -> LlmMessage {
    LlmMessage {
        role: "assistant".to_string(),
        content: text.to_string(),
        tool_call_id: None,
        tool_calls: None,
        images: None,
    }
}

fn tool_result(id: &str, content: &str) -> LlmMessage {
    LlmMessage {
        role: "tool".to_string(),
        content: content.to_string(),
        tool_call_id: Some(id.to_string()),
        tool_calls: None,
        images: None,
    }
}

// ── group_into_turns ──────────────────────────────────────────────────────

#[test]
fn empty_input_produces_no_turns() {
    assert!(group_into_turns(&[]).is_empty());
}

#[test]
fn single_user_message_is_one_turn() {
    let turns = group_into_turns(&[user("hello")]);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].len(), 1);
    assert_eq!(turns[0][0].role, "user");
}

#[test]
fn user_assistant_is_one_turn() {
    let msgs = vec![user("q"), assistant("a")];
    let turns = group_into_turns(&msgs);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].len(), 2);
}

#[test]
fn two_user_messages_produce_two_turns() {
    let msgs = vec![user("q1"), assistant("a1"), user("q2"), assistant("a2")];
    let turns = group_into_turns(&msgs);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0][0].content, "q1");
    assert_eq!(turns[1][0].content, "q2");
}

#[test]
fn tool_call_and_result_stay_within_same_turn() {
    // user → assistant (with tool call) → tool result → assistant response
    let msgs = vec![
        user("do the thing"),
        assistant(""), // assistant triggers tool
        tool_result("tc1", "ok"),
        assistant("done"), // assistant responds after tool
    ];
    let turns = group_into_turns(&msgs);
    // All four messages are in one turn (only one user message)
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].len(), 4);
}

#[test]
fn multi_tool_call_turn_stays_intact() {
    let msgs = vec![
        user("q"),
        assistant(""),
        tool_result("tc1", "r1"),
        tool_result("tc2", "r2"),
        assistant("summary"),
        user("next q"),
        assistant("a2"),
    ];
    let turns = group_into_turns(&msgs);
    assert_eq!(turns.len(), 2);
    // First turn has 5 messages (user + assistant + 2 tool results + assistant)
    assert_eq!(turns[0].len(), 5);
    // Second turn has 2 messages (user + assistant)
    assert_eq!(turns[1].len(), 2);
}

#[test]
fn orphaned_assistant_at_start_forms_its_own_turn() {
    // Unusual but must not panic; sanitize_messages cleans it up later.
    let msgs = vec![assistant("orphan"), user("q"), assistant("a")];
    let turns = group_into_turns(&msgs);
    // "orphan" assistant has no preceding user → it starts a turn by itself
    // because current is empty when we encounter it, so the flush branch
    // never triggers.  Then user("q") flushes that turn.
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0][0].role, "assistant");
    assert_eq!(turns[1][0].role, "user");
}

#[test]
fn back_to_back_user_messages_each_start_a_turn() {
    let msgs = vec![user("a"), user("b"), user("c")];
    let turns = group_into_turns(&msgs);
    assert_eq!(turns.len(), 3);
    for (i, t) in turns.iter().enumerate() {
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].role, "user");
        let expected = ["a", "b", "c"][i];
        assert_eq!(t[0].content, expected);
    }
}

// ── Budget-based turn selection (D4) ──────────────────────────────────────

/// Build a minimal user+assistant turn whose TOTAL chars() count is `chars`.
/// Split evenly between user and assistant messages.
fn make_turn_of_size(chars: usize) -> Vec<LlmMessage> {
    let half = chars / 2;
    let rest = chars - half; // absorbs odd remainders
    vec![
        LlmMessage {
            role: "user".to_string(),
            content: "x".repeat(half),
            tool_call_id: None,
            tool_calls: None,
            images: None,
        },
        LlmMessage {
            role: "assistant".to_string(),
            content: "x".repeat(rest),
            tool_call_id: None,
            tool_calls: None,
            images: None,
        },
    ]
}

/// Simulate the turn-selection loop from `build_context`.
fn select_turns(turns: &[Vec<LlmMessage>], budget: usize) -> (usize, usize) {
    let mut included = 0;
    let mut omitted = 0;
    let mut used = 0usize;
    for turn in turns.iter().rev() {
        let chars: usize = turn.iter().map(|m| m.content.chars().count()).sum();
        if included == 0 || used + chars <= budget {
            included += 1;
            used += chars;
        } else {
            omitted += 1;
        }
    }
    (included, omitted)
}

#[test]
fn all_turns_fit_when_budget_is_large() {
    // 5 turns × 100 chars each = 500 chars total; budget = 10 000 → all fit
    let turns: Vec<Vec<LlmMessage>> = (0..5).map(|_| make_turn_of_size(100)).collect();
    let (included, omitted) = select_turns(&turns, 10_000);
    assert_eq!(included, 5);
    assert_eq!(omitted, 0);
}

#[test]
fn oldest_turns_dropped_when_budget_is_tight() {
    // 10 turns × 200 chars = 2 000 chars; budget = 600 → only 3 newest fit
    let turns: Vec<Vec<LlmMessage>> = (0..10).map(|_| make_turn_of_size(200)).collect();
    let (included, omitted) = select_turns(&turns, 600);
    assert_eq!(included, 3, "exactly 3 turns of 200 chars fit in 600");
    assert_eq!(omitted, 7);
}

#[test]
fn most_recent_turn_always_included_even_if_oversized() {
    // One giant turn (5 000 chars) with budget of only 1 000 → still included.
    let turns = vec![make_turn_of_size(5_000)];
    let (included, omitted) = select_turns(&turns, 1_000);
    assert_eq!(included, 1, "most-recent turn must always be included");
    assert_eq!(omitted, 0);
}

#[test]
fn many_turns_long_session_regression() {
    // Simulate a 100-turn session with 300 chars per turn (30 000 chars total).
    // Budget of 9 000 chars → expect ~30 turns included.
    let turns: Vec<Vec<LlmMessage>> = (0..100).map(|_| make_turn_of_size(300)).collect();
    let (included, omitted) = select_turns(&turns, 9_000);
    assert_eq!(included, 30);
    assert_eq!(omitted, 70);
}

#[test]
fn single_turn_session_always_fully_included() {
    let turns = vec![make_turn_of_size(50)];
    let (included, omitted) = select_turns(&turns, 100);
    assert_eq!(included, 1);
    assert_eq!(omitted, 0);
}

#[test]
fn tool_call_and_result_kept_atomically_during_selection() {
    // Turn 1 (old, large): user + assistant(tool_call) + tool_result + assistant = 4 msgs
    // Turn 2 (new, small): user + assistant = 2 msgs
    // Budget only fits turn 2 → turn 1 must be dropped as a whole.
    let big_turn = vec![
        user("big task"),
        assistant(""),
        tool_result("tc1", "x".repeat(300).as_str()),
        assistant("done"),
    ];
    let small_turn = vec![user("small task"), assistant("ok")];
    let turns = vec![big_turn, small_turn];

    let big_chars: usize = turns[0].iter().map(|m| m.content.chars().count()).sum();
    let small_chars: usize = turns[1].iter().map(|m| m.content.chars().count()).sum();
    let budget = small_chars + 10; // fits exactly 1 small turn but not the big one

    let (included, omitted) = select_turns(&turns, budget);
    // Only the most recent (small) turn fits.
    assert_eq!(
        included, 1,
        "big turn must be dropped as a whole (budget {budget}, big={big_chars})"
    );
    assert_eq!(omitted, 1);
}

// ── ALWAYS_INCLUDE_TOOL_NAMES ─────────────────────────────────────────────

#[test]
fn always_include_list_covers_all_retrieval_tools() {
    // Every retrieval/memory tool must be in the always-include list so
    // they are never accidentally pruned on long conversations.
    for name in &[
        "search_memory",
        "conversation_search",
        "archival_memory_insert",
        "archival_memory_search",
        "update_memory",
        "memory_apply_patch",
    ] {
        assert!(
            ALWAYS_INCLUDE_TOOL_NAMES.contains(name),
            "'{name}' missing from ALWAYS_INCLUDE_TOOL_NAMES"
        );
    }
}

// ── tool_output_limit + db_row_to_llm per-tool truncation ────────────────

fn tool_row(tool_name: &str, content: &str) -> cade_store::sqlite::MessageRow {
    cade_store::sqlite::MessageRow {
        id: "t1".into(),
        agent_id: "a1".into(),
        conversation_id: None,
        role: "tool".into(),
        content: serde_json::json!({
            "content": content,
            "tool_call_id": "tc1",
            "tool_name": tool_name
        }),
        char_count: content.len(),
    }
}

#[test]
fn tool_output_limit_bash_is_4k() {
    assert_eq!(tool_output_limit("bash"), 4_096);
    assert_eq!(tool_output_limit("RunShellCommand"), 4_096);
    assert_eq!(tool_output_limit("developer__shell"), 4_096);
}

#[test]
fn tool_output_limit_read_file_is_12k() {
    assert_eq!(tool_output_limit("read_file"), 12_288);
    assert_eq!(tool_output_limit("ReadFileGemini"), 12_288);
    assert_eq!(tool_output_limit("developer__read_file"), 12_288);
}

#[test]
fn tool_output_limit_grep_is_3k() {
    assert_eq!(tool_output_limit("grep"), 3_072);
    assert_eq!(tool_output_limit("SearchFileContent"), 3_072);
    assert_eq!(tool_output_limit("glob"), 3_072);
    assert_eq!(tool_output_limit("GlobGemini"), 3_072);
}

#[test]
fn tool_output_limit_memory_search_is_2k() {
    assert_eq!(tool_output_limit("search_memory"), 2_048);
    assert_eq!(tool_output_limit("archival_memory_search"), 2_048);
    assert_eq!(tool_output_limit("conversation_search"), 2_048);
}

#[test]
fn tool_output_limit_unknown_is_default() {
    let default = tool_output_limit("");
    assert!(default >= 6_000, "default should be at least 6k");
    assert_eq!(tool_output_limit("some_mcp_tool"), default);
}

#[test]
fn db_row_to_llm_applies_bash_cap() {
    // bash cap is 4_096 chars — a 6_000-char output should be truncated
    let big = "x".repeat(6_000);
    let row = tool_row("bash", &big);
    let msgs = db_row_to_llm(&row);
    assert_eq!(msgs.len(), 1);
    assert!(
        msgs[0].content.chars().count() <= tool_output_limit("bash") + 200,
        "bash output should be capped at ~4096 chars"
    );
    assert!(msgs[0].content.contains("[... truncated"));
}

#[test]
fn db_row_to_llm_applies_read_file_cap() {
    // read_file cap is 12_288 chars — a 15_000-char file should be truncated
    let big = "y".repeat(15_000);
    let row = tool_row("read_file", &big);
    let msgs = db_row_to_llm(&row);
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].content.contains("[... truncated"));
}

#[test]
fn db_row_to_llm_no_truncation_when_within_limit() {
    // A 100-char bash output is well within the 4k cap
    let row = tool_row("bash", "short output");
    let msgs = db_row_to_llm(&row);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "short output");
}

#[test]
fn db_row_to_llm_falls_back_to_default_when_no_tool_name() {
    // Old DB rows without tool_name field should use default limit (no crash)
    let row = cade_store::sqlite::MessageRow {
        id: "t1".into(),
        agent_id: "a1".into(),
        conversation_id: None,
        role: "tool".into(),
        content: serde_json::json!({
            "content": "short output",
            "tool_call_id": "tc1"
            // no "tool_name" field
        }),
        char_count: 12,
    };
    let msgs = db_row_to_llm(&row);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].content, "short output");
}

// ── Constants sanity ─────────────────────────────────────────────────────

#[test]
#[allow(clippy::assertions_on_constants)]
fn constants_are_sane() {
    assert!(
        RECENT_WINDOW >= 10,
        "recent window too small for useful tool-usage detection"
    );
    assert!(
        PINNED_BUDGET >= 5_000,
        "pinned budget too small for typical memory blocks"
    );
    assert!(
        SHORT_BUDGET > PINNED_BUDGET,
        "short budget should exceed pinned budget"
    );
    assert!(MIN_CONTEXT_CHARS < MAX_CONTEXT_CHARS);
    assert!(OUTPUT_RESERVE_FRACTION > 0.0 && OUTPUT_RESERVE_FRACTION < 0.5);
}

// ── End-to-end integration logic for P4-C ────────────────────────────────

#[tokio::test]
async fn send_message_blocking_triggers_needs_consolidation() {
    let db = cade_store::sqlite::open(":memory:").unwrap();

    // Create agent
    let agent_id = "test_agent_1";
    cade_store::sqlite::create_agent(
        &db,
        &cade_store::sqlite::AgentRow {
            id: agent_id.to_string(),
            name: "A".to_string(),
            model: "m".to_string(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
        },
    )
    .unwrap();

    // Insert 50 messages to trigger P5-B (turns_len > 20)
    for i in 0..50 {
        db.lock()
            .execute(
                "INSERT INTO messages (id, agent_id, role, content, char_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    format!("m{i}"),
                    agent_id,
                    if i % 2 == 0 { "user" } else { "assistant" },
                    serde_json::json!({"content": "test"}).to_string(),
                    4,
                    0i64
                ],
            )
            .unwrap();
    }

    let config = std::sync::Arc::new(crate::server::config::ServerConfig {
        addr: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        llm_provider: crate::server::config::LlmProviderKind::Anthropic,
        default_model: "test".into(),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: String::new(),
        api_key: None,

        allowed_origin: None,
        max_context_budget: None,
    });

    let state = AppState {
        db: db.clone(),
        llm: std::sync::Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            llm_provider: String::new(),
        })),
        llm_router: std::sync::Arc::new(tokio::sync::RwLock::new(cade_ai::LlmRouter::build(
            &cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                ollama_base_url: String::new(),
                llm_provider: String::new(),
            },
        ))),
        config,
        mcp: std::sync::Arc::new(crate::server::state::McpManager::empty()),
        rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
        memory_cache: std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        agent_activity: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        agent_metrics: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        agent_context_telemetry: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        context_cache: std::sync::Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
            crate::server::state::CONTEXT_CACHE_CAPACITY,
        ))),
        all_skills: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        agent_skills: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        pending_subagent_results: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
    };

    // Call blocking endpoint
    let res = super::send_message(
        axum::extract::State(state.clone()),
        axum::extract::Path(agent_id.to_string()),
        axum::extract::Json(serde_json::json!({"input": "test"})),
    )
    .await;

    // Check what was saved in the db
    let count: i64 = db
        .lock()
        .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    println!("Total messages in DB: {}", count);

    let (parts, body) = res.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    println!("Response status: {}", parts.status);
    println!("Response body: {}", String::from_utf8_lossy(&bytes));

    let activity = state.agent_activity.read().await;
    let entry = activity.get(agent_id).unwrap();
    assert!(
        entry.needs_consolidation,
        "blocking endpoint must trigger needs_consolidation when turns_len >= 20"
    );
}

// ── P1-1 per-message truncation ────────────────────────────────────────────

#[test]
fn truncate_oversize_message_passthrough_when_under_cap() {
    let msg = LlmMessage {
        role: "tool".to_string(),
        content: "x".repeat(100),
        tool_call_id: Some("t".to_string()),
        tool_calls: None,
        images: None,
    };
    let out = super::context::truncate_oversize_message(msg.clone(), PER_MESSAGE_CHAR_CAP);
    assert_eq!(
        out.content.chars().count(),
        100,
        "under-cap message must pass through untouched"
    );
}

#[test]
fn truncate_oversize_message_caps_huge_tool_result() {
    let huge = "x".repeat(PER_MESSAGE_CHAR_CAP * 4);
    let msg = LlmMessage {
        role: "tool".to_string(),
        content: huge,
        tool_call_id: Some("t".to_string()),
        tool_calls: None,
        images: None,
    };
    let out = super::context::truncate_oversize_message(msg, PER_MESSAGE_CHAR_CAP);
    let len = out.content.chars().count();
    assert!(
        len <= PER_MESSAGE_CHAR_CAP + TRUNCATION_MARKER.chars().count() + 8,
        "truncated content must fit within cap + marker, got {len}"
    );
    assert!(
        out.content.contains(TRUNCATION_MARKER),
        "must include truncation marker"
    );
}

#[test]
fn truncate_oversize_message_keeps_head_and_tail() {
    let mut content = String::new();
    content.push_str("HEADSTART");
    content.push_str(&"m".repeat(PER_MESSAGE_CHAR_CAP * 3));
    content.push_str("TAILEND");
    let msg = LlmMessage {
        role: "tool".to_string(),
        content,
        tool_call_id: Some("t".to_string()),
        tool_calls: None,
        images: None,
    };
    let out = super::context::truncate_oversize_message(msg, PER_MESSAGE_CHAR_CAP);
    assert!(out.content.starts_with("HEADSTART"), "must preserve head");
    assert!(out.content.contains("TAILEND"), "must preserve tail");
}

#[test]
fn truncate_oversize_message_preserves_role_and_tool_call_id() {
    let msg = LlmMessage {
        role: "tool".to_string(),
        content: "x".repeat(PER_MESSAGE_CHAR_CAP * 2),
        tool_call_id: Some("call_abc".to_string()),
        tool_calls: None,
        images: None,
    };
    let out = super::context::truncate_oversize_message(msg, PER_MESSAGE_CHAR_CAP);
    assert_eq!(out.role, "tool");
    assert_eq!(out.tool_call_id.as_deref(), Some("call_abc"));
}

#[tokio::test]
async fn build_context_caps_oversize_tool_result_messages() {
    let db = cade_store::sqlite::open(":memory:").unwrap();
    let agent_id = "agent_p1_1";
    cade_store::sqlite::create_agent(
        &db,
        &cade_store::sqlite::AgentRow {
            id: agent_id.to_string(),
            name: "A".to_string(),
            model: "anthropic/claude-sonnet-4-5-20250929".to_string(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
        },
    )
    .unwrap();

    // Insert: user, assistant(huge text), assistant(text)
    let huge = "Z".repeat(PER_MESSAGE_CHAR_CAP * 10);
    let rows: Vec<(&str, &str, serde_json::Value)> = vec![
        ("u1", "user", serde_json::json!({"content": "do it"})),
        ("a1", "assistant", serde_json::json!({"content": huge})),
        ("u2", "user", serde_json::json!({"content": "and now this"})),
    ];
    for (id, role, content) in rows {
        db.lock()
            .execute(
                "INSERT INTO messages (id, agent_id, role, content, char_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    id,
                    agent_id,
                    role,
                    content.to_string(),
                    content.to_string().len() as i64,
                    0i64
                ],
            )
            .unwrap();
    }

    let config = std::sync::Arc::new(crate::server::config::ServerConfig {
        addr: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        llm_provider: crate::server::config::LlmProviderKind::Anthropic,
        default_model: "test".into(),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: String::new(),
        api_key: None,
        allowed_origin: None,
        max_context_budget: None,
    });
    let state = AppState {
        db: db.clone(),
        llm: std::sync::Arc::new(cade_ai::LlmRouter::build(&cade_ai::AiConfig {
            anthropic_api_key: None,
            openai_api_key: None,
            google_api_key: None,
            ollama_base_url: String::new(),
            llm_provider: String::new(),
        })),
        llm_router: std::sync::Arc::new(tokio::sync::RwLock::new(cade_ai::LlmRouter::build(
            &cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                ollama_base_url: String::new(),
                llm_provider: String::new(),
            },
        ))),
        config,
        mcp: std::sync::Arc::new(crate::server::state::McpManager::empty()),
        rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
        memory_cache: std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        agent_activity: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        agent_metrics: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        agent_context_telemetry: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        context_cache: std::sync::Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
            crate::server::state::CONTEXT_CACHE_CAPACITY,
        ))),
        all_skills: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        agent_skills: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        pending_subagent_results: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
    };

    let (_model, messages, _tools) = super::context::build_context(&state, agent_id, None, false)
        .await
        .expect("build_context");

    // Find the giant assistant message in the assembled context
    let big_msg = messages
        .iter()
        .find(|m| m.role == "assistant" && m.content.len() > 100)
        .expect("oversized assistant message must be present");
    let len = big_msg.content.chars().count();
    assert!(
        len <= PER_MESSAGE_CHAR_CAP + TRUNCATION_MARKER.chars().count() + 8,
        "oversized message must be truncated by build_context, got {len} chars"
    );
    assert!(
        big_msg.content.contains(TRUNCATION_MARKER),
        "must include truncation marker for agent recovery"
    );
}

// ── P1-2 pre-flight overflow guard ─────────────────────────────────────────

/// The pre-flight guard must drop oldest turns when, after PER_MESSAGE_CHAR_CAP
/// truncation, the cumulative size still exceeds the budget.  In practice this
/// happens when the most-recent turn alone is near the cap (so cannot be
/// reduced further) and earlier turns push the total over.
fn select_with_preflight(turns: Vec<Vec<LlmMessage>>, budget: usize) -> (usize, usize) {
    // Mirror the build_context preflight: walk newest→oldest greedily, then
    // drop oldest selected until total fits (preserving most-recent).
    let mut selected: Vec<Vec<LlmMessage>> = Vec::new();
    let mut used = 0usize;
    let mut omitted = 0usize;
    for turn in turns.into_iter().rev() {
        let chars: usize = turn.iter().map(|m| m.content.chars().count()).sum();
        if selected.is_empty() || used + chars <= budget {
            used += chars;
            selected.push(turn);
        } else {
            omitted += 1;
        }
    }
    while selected.len() > 1 && used > budget {
        if let Some(dropped) = selected.pop() {
            let chars: usize = dropped.iter().map(|m| m.content.chars().count()).sum();
            used = used.saturating_sub(chars);
            omitted += 1;
        }
    }
    (selected.len(), omitted)
}

#[test]
fn preflight_guard_drops_oldest_when_total_exceeds_budget() {
    // Three 400-char turns; budget 500.  Newest fits alone (400 ≤ 500); the
    // greedy walker then can't add more.  But if budget were e.g. exceeded by
    // an oversized newest turn we still need preflight: simulate by
    // constructing turns that each *individually* fit but cumulatively don't,
    // and an over-cap newest turn that would push the total over.
    let huge_recent = make_turn_of_size(800); // > budget alone — always-included
    let medium = make_turn_of_size(300);
    let turns = vec![medium.clone(), medium.clone(), huge_recent];
    let budget = 500;
    let (included, omitted) = select_with_preflight(turns, budget);
    assert_eq!(
        included, 1,
        "only most-recent turn must remain after preflight"
    );
    assert_eq!(omitted, 2);
}

#[test]
fn preflight_guard_keeps_most_recent_even_if_oversized() {
    let huge_recent = make_turn_of_size(10_000);
    let turns = vec![huge_recent];
    let (included, _omitted) = select_with_preflight(turns, 1_000);
    assert_eq!(included, 1, "most-recent turn must never be dropped");
}

#[test]
fn preflight_guard_no_op_when_under_budget() {
    let turns: Vec<Vec<LlmMessage>> = (0..3).map(|_| make_turn_of_size(100)).collect();
    let (included, omitted) = select_with_preflight(turns, 10_000);
    assert_eq!(included, 3);
    assert_eq!(omitted, 0);
}

// ── P1-3 provider-error recovery ──────────────────────────────────────────

/// Fake provider that returns `Provider{400, "context_length_exceeded"}` on the
/// first call and a successful response on the second.  Used to verify that
/// `complete_with_overflow_recovery` retries exactly once on overflow.
struct OverflowThenOk {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl cade_ai::LlmProvider for OverflowThenOk {
    async fn complete(
        &self,
        _req: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<cade_ai::CompletionResponse> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            Err(cade_ai::Error::Provider {
                status: 400,
                msg: "context_length_exceeded".into(),
            })
        } else {
            Ok(cade_ai::CompletionResponse {
                content: Some("recovered".into()),
                tool_calls: vec![],
                finish_reason: "stop".into(),
            })
        }
    }

    async fn stream(
        &self,
        _req: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<
        std::pin::Pin<
            Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
        >,
    > {
        unreachable!("stream() is not exercised by this mock")
    }
}

struct AlwaysOverflow;

#[async_trait::async_trait]
impl cade_ai::LlmProvider for AlwaysOverflow {
    async fn complete(
        &self,
        _req: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<cade_ai::CompletionResponse> {
        Err(cade_ai::Error::Provider {
            status: 400,
            msg: "prompt is too long".into(),
        })
    }

    async fn stream(
        &self,
        _req: &cade_ai::CompletionRequest,
    ) -> cade_ai::Result<
        std::pin::Pin<
            Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
        >,
    > {
        unreachable!("stream() is not exercised by this mock")
    }
}

fn build_minimal_state(
    db: cade_store::sqlite::Db,
    llm: std::sync::Arc<dyn cade_ai::LlmProvider>,
) -> AppState {
    let config = std::sync::Arc::new(crate::server::config::ServerConfig {
        addr: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        llm_provider: crate::server::config::LlmProviderKind::Anthropic,
        default_model: "test".into(),
        anthropic_api_key: None,
        openai_api_key: None,
        google_api_key: None,
        ollama_base_url: String::new(),
        api_key: None,
        allowed_origin: None,
        max_context_budget: None,
    });
    AppState {
        db,
        llm,
        llm_router: std::sync::Arc::new(tokio::sync::RwLock::new(cade_ai::LlmRouter::build(
            &cade_ai::AiConfig {
                anthropic_api_key: None,
                openai_api_key: None,
                google_api_key: None,
                ollama_base_url: String::new(),
                llm_provider: String::new(),
            },
        ))),
        config,
        mcp: std::sync::Arc::new(crate::server::state::McpManager::empty()),
        rate_limiter: crate::server::rate_limit::RateLimiter::from_env(),
        memory_cache: std::sync::Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        agent_activity: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        agent_metrics: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        agent_context_telemetry: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        context_cache: std::sync::Arc::new(parking_lot::Mutex::new(lru::LruCache::new(
            crate::server::state::CONTEXT_CACHE_CAPACITY,
        ))),
        all_skills: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        agent_skills: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        pending_subagent_results: std::sync::Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        subagent_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
    }
}

fn seed_basic_agent(db: &cade_store::sqlite::Db, agent_id: &str) {
    cade_store::sqlite::create_agent(
        db,
        &cade_store::sqlite::AgentRow {
            id: agent_id.to_string(),
            name: "A".to_string(),
            model: "anthropic/claude-sonnet-4-5-20250929".to_string(),
            description: None,
            system_prompt: None,
            created_at: None,
            compaction_model: None,
            theme: None,
        },
    )
    .unwrap();
    db.lock()
        .execute(
            "INSERT INTO messages (id, agent_id, role, content, char_count, created_at)
         VALUES ('u', ?1, 'user', ?2, 5, 0)",
            rusqlite::params![agent_id, serde_json::json!({"content": "hi"}).to_string()],
        )
        .unwrap();
}

#[tokio::test]
async fn complete_with_overflow_recovery_retries_once_and_succeeds() {
    let db = cade_store::sqlite::open(":memory:").unwrap();
    let agent_id = "agent_p1_3a";
    seed_basic_agent(&db, agent_id);
    let llm = std::sync::Arc::new(OverflowThenOk {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let state = build_minimal_state(db, llm.clone());
    let req = cade_ai::CompletionRequest {
        model: "anthropic/claude-sonnet-4-5-20250929".into(),
        messages: vec![],
        tools: vec![],
        max_tokens: 100,
        reasoning_effort: None,
    };
    let res =
        super::context::complete_with_overflow_recovery(&state, agent_id, None, false, req).await;
    let resp = res.expect("recovery must succeed");
    assert_eq!(resp.content.as_deref(), Some("recovered"));
    assert_eq!(
        llm.calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "must retry exactly once"
    );
}

#[tokio::test]
async fn complete_with_overflow_recovery_surfaces_persistent_overflow() {
    let db = cade_store::sqlite::open(":memory:").unwrap();
    let agent_id = "agent_p1_3b";
    seed_basic_agent(&db, agent_id);
    let llm = std::sync::Arc::new(AlwaysOverflow);
    let state = build_minimal_state(db, llm);
    let req = cade_ai::CompletionRequest {
        model: "anthropic/claude-sonnet-4-5-20250929".into(),
        messages: vec![],
        tools: vec![],
        max_tokens: 100,
        reasoning_effort: None,
    };
    let res =
        super::context::complete_with_overflow_recovery(&state, agent_id, None, false, req).await;
    match res {
        Err(cade_ai::Error::Provider { status, msg }) => {
            assert_eq!(status, 400);
            assert!(msg.contains("prompt is too long"));
        }
        other => panic!("expected persistent overflow to surface, got {other:?}"),
    }
}

#[tokio::test]
async fn complete_with_overflow_recovery_passes_through_non_overflow_errors() {
    struct Always429;
    #[async_trait::async_trait]
    impl cade_ai::LlmProvider for Always429 {
        async fn complete(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<cade_ai::CompletionResponse> {
            Err(cade_ai::Error::Provider {
                status: 429,
                msg: "rate_limited".into(),
            })
        }
        async fn stream(
            &self,
            _r: &cade_ai::CompletionRequest,
        ) -> cade_ai::Result<
            std::pin::Pin<
                Box<dyn tokio_stream::Stream<Item = cade_ai::Result<cade_ai::StreamChunk>> + Send>,
            >,
        > {
            unreachable!("stream() is not exercised by this mock")
        }
    }
    let db = cade_store::sqlite::open(":memory:").unwrap();
    let agent_id = "agent_p1_3c";
    seed_basic_agent(&db, agent_id);
    let llm = std::sync::Arc::new(Always429);
    let state = build_minimal_state(db, llm);
    let req = cade_ai::CompletionRequest {
        model: "anthropic/claude-sonnet-4-5-20250929".into(),
        messages: vec![],
        tools: vec![],
        max_tokens: 100,
        reasoning_effort: None,
    };
    let res =
        super::context::complete_with_overflow_recovery(&state, agent_id, None, false, req).await;
    match res {
        Err(cade_ai::Error::Provider { status, .. }) => assert_eq!(status, 429),
        other => panic!("expected 429 passthrough, got {other:?}"),
    }
}

// ── P2-2 system + memory subtracted from message budget ────────────────────

#[tokio::test]
async fn build_context_subtracts_full_system_prompt_and_memory() {
    let db = cade_store::sqlite::open(":memory:").unwrap();
    let agent_id = "agent_p2_2";
    cade_store::sqlite::create_agent(
        &db,
        &cade_store::sqlite::AgentRow {
            id: agent_id.to_string(),
            name: "A".to_string(),
            // model with a small window so the budget is tight
            model: "anthropic/claude-sonnet-4-5-20250929".to_string(),
            description: None,
            // big system prompt
            system_prompt: Some("S".repeat(10_000)),
            created_at: None,
            compaction_model: None,
            theme: None,
        },
    )
    .unwrap();

    // Add a pinned memory block via direct SQL
    {
        let conn = db.lock();
        conn.execute(
            "INSERT INTO shared_memory_blocks (id, label, value, description, updated_at, last_turn, tier)
             VALUES ('mb1', 'pinned_block', ?1, '', 0, 0, 'pinned')",
            rusqlite::params!["P".repeat(8_000)],
        ).unwrap();
        conn.execute(
            "INSERT INTO agent_memory_blocks (agent_id, block_id) VALUES (?1, 'mb1')",
            rusqlite::params![agent_id],
        )
        .unwrap();
    }

    // Insert two user/assistant turns
    for i in 0..4 {
        db.lock()
            .execute(
                "INSERT INTO messages (id, agent_id, role, content, char_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    format!("m{i}"),
                    agent_id,
                    if i % 2 == 0 { "user" } else { "assistant" },
                    serde_json::json!({"content": "x".repeat(50)}).to_string(),
                    50i64,
                    i as i64
                ],
            )
            .unwrap();
    }

    let llm = std::sync::Arc::new(OverflowThenOk { calls: std::sync::atomic::AtomicUsize::new(2) }) // never used
        as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_minimal_state(db, llm);

    let (_model, messages, _tools) = super::context::build_context(&state, agent_id, None, false)
        .await
        .expect("build_context");

    // Both system messages (static + dynamic) must be present.
    let sys_count = messages.iter().filter(|m| m.role == "system").count();
    assert_eq!(sys_count, 2, "must have static + dynamic system messages");
    let static_sys = &messages[0].content;
    assert!(
        static_sys.contains(&"P".repeat(100)),
        "static system must include the pinned memory block"
    );

    // Sum total chars in the assembled message list (system + history).
    let total: usize = messages.iter().map(|m| m.content.chars().count()).sum();
    let window =
        cade_ai::catalogue::context_window_for_model("anthropic/claude-sonnet-4-5-20250929")
            as usize;
    // input budget at 15% reserve, 3 chars/token
    let max_chars = ((window as f64 * 0.85).round() as usize) * 3;
    assert!(
        total <= max_chars,
        "assembled context ({total} chars) must fit within input budget ({max_chars} chars) \
         after subtracting full system prompt + memory"
    );
}

// ── P2-3 bounded skills injection ─────────────────────────────────────────

fn fake_skill(id: &str, name: &str, body_chars: usize) -> cade_core::skills::Skill {
    cade_core::skills::Skill {
        id: id.into(),
        name: name.into(),
        description: format!("desc for {name}"),
        category: None,
        tags: vec![],
        triggers: vec![],
        rpi_phase: None,
        capabilities: vec![],
        scripts: vec![],
        references: vec![],
        body: "B".repeat(body_chars),
        scope: cade_core::skills::SkillScope::Project,
        path: std::path::PathBuf::new(),
    }
}

#[test]
fn render_skills_section_empty_when_no_loaded() {
    assert_eq!(
        super::context::render_skills_section(&[], 10_000, 5_000),
        ""
    );
}

#[test]
fn render_skills_section_emits_full_body_when_under_caps() {
    let s = fake_skill("a", "Alpha", 100);
    let out = super::context::render_skills_section(&[&s], 10_000, 5_000);
    assert!(out.contains("# Loaded Skills"));
    assert!(out.contains("## Skill: Alpha (a)"));
    assert!(out.contains(&"B".repeat(100)));
    assert!(!out.contains("summary-only"));
}

#[test]
fn render_skills_section_falls_back_to_summary_when_body_exceeds_individual_cap() {
    let big = fake_skill("big", "Big", 20_000);
    let out = super::context::render_skills_section(&[&big], 100_000, 5_000);
    assert!(out.contains("summary-only"));
    assert!(
        !out.contains(&"B".repeat(20_000)),
        "must not include full oversized body"
    );
}

#[test]
fn render_skills_section_respects_total_budget() {
    let a = fake_skill("a", "A", 4_000);
    let b = fake_skill("b", "B", 4_000);
    let c = fake_skill("c", "C", 4_000);
    // Budget: ~6_000 chars — only first skill's body fits in full.
    let out = super::context::render_skills_section(&[&a, &b, &c], 6_000, 5_000);
    let len = out.chars().count();
    assert!(len <= 6_500, "section ({len} chars) must be near budget");
    // First skill full, later ones summarised or omitted with marker.
    assert!(out.contains("## Skill: A (a)"));
    assert!(out.contains(&"B".repeat(3_000)), "first body present");
    let summary_or_omit = out.contains("summary-only") || out.contains("more loaded skill");
    assert!(summary_or_omit, "later skills must summarise or omit");
}

#[test]
fn render_skills_section_emits_omit_marker_when_budget_exhausted_mid_list() {
    // Tiny budget so even summary entries cannot fit beyond the first.
    let a = fake_skill("a", "Alpha", 50);
    let b = fake_skill("b", "Beta", 50);
    let c = fake_skill("c", "Gamma", 50);
    // 200-char budget: header + first full entry only.
    let out = super::context::render_skills_section(&[&a, &b, &c], 220, 5_000);
    assert!(out.contains("## Skill: Alpha (a)"));
    // Either Beta has been added as a summary OR omit marker is present.
    let has_omit = out.contains("more loaded skill");
    let has_beta = out.contains("Beta (b)");
    assert!(has_omit || has_beta, "later skills must surface somehow");
}

// ── P2-1 token-based budget accounting ─────────────────────────────────────

#[test]
fn token_count_used_for_system_overhead_smoke() {
    // The cade_ai::count_tokens function must produce a smaller-or-equal
    // result than the legacy chars/3 estimate for typical English text.
    // (chars/3 over-counts by ~30% vs real tokenizer.)
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(50);
    let real_tokens = cade_ai::count_tokens("openai/gpt-4o", &text);
    let chars = text.chars().count();
    let estimated_tokens = chars / 3;
    assert!(real_tokens > 0);
    // Real tokenizer should report fewer tokens than the conservative chars/3 ratio.
    assert!(
        real_tokens < estimated_tokens,
        "real tokenizer ({real_tokens}) should be more efficient than chars/3 estimate ({estimated_tokens})"
    );
}

#[tokio::test]
async fn build_context_message_budget_reflects_real_token_overhead() {
    // Two agents with identical history but different system prompt sizes.
    // The agent with the larger system prompt must end up with a strictly
    // smaller message budget after the token-based deduction.
    use cade_store::sqlite as ssq;
    let db = ssq::open(":memory:").unwrap();

    let small_id = "agent_p2_1_small";
    let big_id = "agent_p2_1_big";
    for (id, sys_size) in &[(small_id, 200usize), (big_id, 50_000usize)] {
        ssq::create_agent(
            &db,
            &ssq::AgentRow {
                id: (*id).to_string(),
                name: "A".to_string(),
                model: "anthropic/claude-sonnet-4-5-20250929".to_string(),
                description: None,
                system_prompt: Some("S".repeat(*sys_size)),
                created_at: None,
                compaction_model: None,
                theme: None,
            },
        )
        .unwrap();
        // One short user message so build_context returns
        db.lock()
            .execute(
                "INSERT INTO messages (id, agent_id, role, content, char_count, created_at)
             VALUES (?1, ?2, 'user', ?3, 5, 0)",
                rusqlite::params![
                    format!("u_{id}"),
                    id,
                    serde_json::json!({"content": "hi"}).to_string()
                ],
            )
            .unwrap();
    }

    let llm = std::sync::Arc::new(AlwaysOverflow) as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_minimal_state(db, llm);

    let (_m1, msgs_small, _) = super::context::build_context(&state, small_id, None, false)
        .await
        .unwrap();
    let (_m2, msgs_big, _) = super::context::build_context(&state, big_id, None, false)
        .await
        .unwrap();

    let sum_chars = |v: &Vec<cade_ai::LlmMessage>| -> usize {
        v.iter().map(|m| m.content.chars().count()).sum()
    };
    let small_total = sum_chars(&msgs_small);
    let big_total = sum_chars(&msgs_big);
    assert!(
        big_total > small_total,
        "agent with larger system prompt ({big_total}) must have a larger total assembled"
    );
    // And the per-call accounting must not allow the assembled output to
    // exceed (window × 0.85 × 3) chars regardless of system size.
    let window =
        cade_ai::catalogue::context_window_for_model("anthropic/claude-sonnet-4-5-20250929")
            as usize;
    let max_chars = ((window as f64 * 0.85).round() as usize) * 3;
    assert!(
        big_total <= max_chars,
        "big agent total {big_total} must fit in {max_chars}"
    );
}

#[tokio::test]
async fn build_context_token_overhead_frees_more_budget_than_chars() {
    // English-prose system prompt: real token count is ~chars/3.7, so the
    // token-based deduction must reserve fewer chars (=more available for
    // history) than the legacy raw-char deduction would have.
    use cade_store::sqlite as ssq;
    let db = ssq::open(":memory:").unwrap();
    let agent_id = "agent_p2_1_tokens";
    let prose = "The quick brown fox jumps over the lazy dog. ".repeat(300);
    let prose_chars = prose.chars().count();
    let prose_tokens = cade_ai::count_tokens("openai/gpt-4o", &prose);
    // Sanity: real tokenizer is more efficient than chars/3 for English.
    assert!(prose_tokens > 0);
    assert!(
        cade_ai::chars_for_tokens(prose_tokens) < prose_chars,
        "token-based deduction must reserve fewer chars than raw chars"
    );

    ssq::create_agent(
        &db,
        &ssq::AgentRow {
            id: agent_id.to_string(),
            name: "A".to_string(),
            model: "openai/gpt-4o".to_string(),
            description: None,
            system_prompt: Some(prose),
            created_at: None,
            compaction_model: None,
            theme: None,
        },
    )
    .unwrap();

    // Insert many short messages so the budget actually packs them.
    for i in 0..40 {
        db.lock()
            .execute(
                "INSERT INTO messages (id, agent_id, role, content, char_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    format!("m{i}"),
                    agent_id,
                    if i % 2 == 0 { "user" } else { "assistant" },
                    serde_json::json!({"content": "x".repeat(30)}).to_string(),
                    30i64,
                    i as i64
                ],
            )
            .unwrap();
    }

    let llm = std::sync::Arc::new(AlwaysOverflow) as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_minimal_state(db, llm);

    let (_m, messages, _t) = super::context::build_context(&state, agent_id, None, false)
        .await
        .unwrap();
    let history_msgs = messages.iter().filter(|m| m.role != "system").count();
    // With token-based accounting the system prompt costs ~prose_tokens × 3
    // chars instead of prose_chars; this should leave room for *all* 40
    // short messages on a 128k window.
    assert_eq!(
        history_msgs, 40,
        "all 40 short history messages must fit (token-based accounting freed budget)"
    );
}

// ── Phase 4: build_context telemetry capture ─────────────────────────────

#[tokio::test]
async fn build_context_records_telemetry_with_fits_budget_true() {
    use cade_store::sqlite as ssq;
    let db = ssq::open(":memory:").unwrap();
    let agent_id = "agent_telemetry_ok";
    ssq::create_agent(
        &db,
        &ssq::AgentRow {
            id: agent_id.to_string(),
            name: "A".to_string(),
            model: "anthropic/claude-sonnet-4-5-20250929".to_string(),
            description: None,
            system_prompt: Some("short prompt".to_string()),
            created_at: None,
            compaction_model: None,
            theme: None,
        },
    )
    .unwrap();
    db.lock()
        .execute(
            "INSERT INTO messages (id, agent_id, role, content, char_count, created_at)
         VALUES ('u1', ?1, 'user', ?2, 5, 0)",
            rusqlite::params![agent_id, serde_json::json!({"content": "hi"}).to_string()],
        )
        .unwrap();

    let llm = std::sync::Arc::new(AlwaysOverflow) as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_minimal_state(db, llm);

    let _ = super::context::build_context(&state, agent_id, None, false)
        .await
        .unwrap();

    let telem = state.agent_context_telemetry.read().await;
    let t = telem.get(agent_id).expect("telemetry must be recorded");
    assert!(t.fits_budget, "small agent must fit");
    assert!(t.window_tokens > 0);
    assert!(t.input_budget_chars > 0);
    assert!(t.system_msg_count >= 1, "static system message at minimum");
    assert!(t.build_micros > 0, "build_micros must be measured");
    assert_eq!(t.skills_full, 0);
    assert_eq!(t.skills_summary, 0);
}

#[tokio::test]
async fn build_context_records_native_token_counts() {
    use cade_store::sqlite as ssq;
    let db = ssq::open(":memory:").unwrap();
    let agent_id = "agent_native_tokens";
    ssq::create_agent(
        &db,
        &ssq::AgentRow {
            id: agent_id.to_string(),
            name: "A".to_string(),
            model: "openai/gpt-4o".to_string(),
            description: None,
            system_prompt: Some("You are helpful.".to_string()),
            created_at: None,
            compaction_model: None,
            theme: None,
        },
    )
    .unwrap();
    let user_msg =
        serde_json::json!({"content": "The quick brown fox jumps over the lazy dog. ".repeat(20)})
            .to_string();
    db.lock()
        .execute(
            "INSERT INTO messages (id, agent_id, role, content, char_count, created_at)
         VALUES ('u1', ?1, 'user', ?2, 1000, 0)",
            rusqlite::params![agent_id, user_msg],
        )
        .unwrap();

    let llm = std::sync::Arc::new(AlwaysOverflow) as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_minimal_state(db, llm);

    let _ = super::context::build_context(&state, agent_id, None, false)
        .await
        .unwrap();

    let telem = state.agent_context_telemetry.read().await;
    let t = telem.get(agent_id).expect("telemetry recorded");
    assert!(t.history_tokens > 0, "history must produce non-zero tokens");
    assert!(t.total_tokens >= t.system_tokens, "total >= system");
    assert!(t.total_tokens >= t.history_tokens, "total >= history");
    assert!(
        t.history_tokens < t.history_chars,
        "tokens ({}) must be less than chars ({}) for English text",
        t.history_tokens,
        t.history_chars
    );
    assert!(t.fits_budget, "small request must fit");
}

// ── P1: skills section must live in system_static (cache-anchored) ─────────
//
// Anthropic's prompt cache only marks the *first* system block with
// `cache_control: ephemeral`.  Skills bodies are stable across many turns
// (they only change on /skills load|unload), so they MUST live in the
// cache-anchored block (system_static, messages[0]) and NOT in the volatile
// dynamic block (system_dynamic, messages[1]).  This unlocks ~90% input-cost
// reduction on the skills payload across a session.
#[tokio::test]
async fn skills_section_lives_in_static_system_block() {
    let db = cade_store::sqlite::open(":memory:").unwrap();
    let agent_id = "agent_p1_skills";
    cade_store::sqlite::create_agent(
        &db,
        &cade_store::sqlite::AgentRow {
            id: agent_id.to_string(),
            name: "A".to_string(),
            model: "anthropic/claude-sonnet-4-5-20250929".to_string(),
            description: None,
            system_prompt: Some("base prompt".to_string()),
            created_at: None,
            compaction_model: None,
            theme: None,
        },
    )
    .unwrap();

    let llm = std::sync::Arc::new(OverflowThenOk {
        calls: std::sync::atomic::AtomicUsize::new(2),
    }) as std::sync::Arc<dyn cade_ai::LlmProvider>;
    let state = build_minimal_state(db, llm);

    // Register a skill in the global pool and load it for the agent.
    let s = fake_skill("kpx-marker", "KpxMarkerSkill", 200);
    {
        let mut all = state.all_skills.write().await;
        all.push(s);
        let mut loaded = state.agent_skills.write().await;
        loaded.insert(agent_id.to_string(), vec!["kpx-marker".to_string()]);
    }

    let (_model, messages, _tools) = super::context::build_context(&state, agent_id, None, false)
        .await
        .expect("build_context");

    assert!(messages.len() >= 2, "must have 2 system messages");
    let static_sys = &messages[0].content;
    let dynamic_sys = &messages[1].content;

    let marker = "## Skill: KpxMarkerSkill (kpx-marker)";
    assert!(
        static_sys.contains(marker),
        "skills section MUST appear in system_static (messages[0]) for cache anchoring;\n\
         static_sys (first 400 chars):\n{}\n",
        &static_sys.chars().take(400).collect::<String>()
    );
    assert!(
        !dynamic_sys.contains(marker),
        "skills section MUST NOT appear in system_dynamic (messages[1]); \
         that block is volatile and gets re-tokenised every turn"
    );
}

// ── P5: compress_tool_schema ─────────────────────────────────────────────────

#[test]
fn compress_tool_schema_preserves_name_and_parameters_shape() {
    use serde_json::json;
    let s = json!({
        "name": "my_tool",
        "description": "A long detailed description of what my_tool does. ".repeat(10),
        "parameters": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "filesystem path" },
                "limit": { "type": "integer", "description": "max items", "examples": [10, 50] }
            },
            "required": ["path"]
        }
    });
    let c = super::context::compress_tool_schema(s);
    assert_eq!(c["name"].as_str(), Some("my_tool"));
    let desc_len = c["description"].as_str().unwrap().chars().count();
    assert!(
        desc_len <= super::context::COMPRESSED_DESCRIPTION_CHAR_CAP,
        "compressed top-level description too long: {desc_len}"
    );
    // Shape preserved
    assert_eq!(c["parameters"]["type"].as_str(), Some("object"));
    assert_eq!(c["parameters"]["required"][0].as_str(), Some("path"));
    assert_eq!(
        c["parameters"]["properties"]["path"]["type"].as_str(),
        Some("string")
    );
    assert_eq!(
        c["parameters"]["properties"]["limit"]["type"].as_str(),
        Some("integer")
    );
    // Per-property descriptions stripped
    assert!(
        c["parameters"]["properties"]["path"]
            .get("description")
            .is_none()
    );
    assert!(
        c["parameters"]["properties"]["limit"]
            .get("description")
            .is_none()
    );
    // Examples stripped
    assert!(
        c["parameters"]["properties"]["limit"]
            .get("examples")
            .is_none()
    );
}

#[test]
fn compress_tool_schema_truncates_at_first_newline() {
    use serde_json::json;
    let s = json!({
        "name": "t",
        "description": "First line summary.\nSecond paragraph with extra detail.\n\nMore."
    });
    let c = super::context::compress_tool_schema(s);
    assert_eq!(c["description"].as_str(), Some("First line summary."));
}

#[test]
fn compress_tool_schema_handles_input_schema_variant() {
    use serde_json::json;
    let s = json!({
        "name": "anth_tool",
        "description": "x",
        "input_schema": {
            "type": "object",
            "properties": {
                "q": { "type": "string", "description": "query" }
            }
        }
    });
    let c = super::context::compress_tool_schema(s);
    assert!(
        c["input_schema"]["properties"]["q"]
            .get("description")
            .is_none()
    );
    assert_eq!(
        c["input_schema"]["properties"]["q"]["type"].as_str(),
        Some("string")
    );
}

#[test]
fn compress_tool_schema_idempotent() {
    use serde_json::json;
    let s = json!({
        "name": "t",
        "description": "short",
        "parameters": { "type": "object", "properties": {} }
    });
    let once = super::context::compress_tool_schema(s.clone());
    let twice = super::context::compress_tool_schema(once.clone());
    assert_eq!(once, twice);
}

#[test]
fn compress_tool_schema_no_description_is_safe() {
    use serde_json::json;
    let s = json!({ "name": "bare", "parameters": { "type": "object", "properties": {} } });
    let c = super::context::compress_tool_schema(s);
    assert!(c.get("description").is_none());
    assert_eq!(c["name"].as_str(), Some("bare"));
}

#[test]
fn compress_tool_schema_reduces_byte_size_substantially() {
    use serde_json::json;
    let big_desc = "Long detailed description. ".repeat(50); // ~1.4 KB
    let s = json!({
        "name": "t",
        "description": big_desc,
        "parameters": {
            "type": "object",
            "properties": {
                "p": { "type": "string", "description": "very long property description ".repeat(30) }
            }
        }
    });
    let before = serde_json::to_string(&s).unwrap().len();
    let after = serde_json::to_string(&super::context::compress_tool_schema(s))
        .unwrap()
        .len();
    assert!(
        after < before / 4,
        "compression must reduce schema by ≥ 4x; before={before} after={after}"
    );
}
