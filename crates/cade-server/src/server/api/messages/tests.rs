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
