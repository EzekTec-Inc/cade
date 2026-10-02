#[allow(unused)]
type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

use super::render::count_wrapped_segment;
use super::*;

#[test]
fn question_other_accepts_digits_and_details_scroll_without_changing_focus() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let (tx, rx) = tokio::sync::oneshot::channel();
    let question = crate::question::Question {
        header: "Question".into(),
        text: "Choose a value".into(),
        options: vec![crate::question::QuestionOption {
            label: "First".into(),
            description: String::new(),
        }],
        multi_select: false,
        allow_other: true,
        progress: None,
    };
    let mut draw_state = ActiveQuestionDrawState::new(question);
    draw_state.cursor_pos = draw_state.other_idx;
    let mut overlay = ActiveQuestionState {
        approval_id: None,
        draw_state,
        tx: Some(tx),
        result: None,
    };
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        overlay.handle_input(key(KeyCode::PageDown)),
        OverlayInputResult::Consumed
    );
    assert_eq!(overlay.draw_state.detail_scroll, 3);
    assert_eq!(
        overlay.handle_input(key(KeyCode::Char('1'))),
        OverlayInputResult::Consumed
    );
    assert_eq!(overlay.draw_state.custom_text, "1");
    assert_eq!(
        overlay.handle_input(key(KeyCode::Enter)),
        OverlayInputResult::Dismiss
    );
    assert!(
        matches!(rx.blocking_recv(), Ok(Some(crate::question::QuestionAnswer::Custom(answer))) if answer == "1")
    );
}

#[test]
fn test_app_question_result_formatting() {
    // -- Setup & Fixtures
    let line = RenderLine::QuestionResult {
        header: "Decision".to_string(),
        answer: "Yes".to_string(),
    };

    // -- Check
    match line {
        RenderLine::QuestionResult { header, answer } => {
            assert_eq!(header, "Decision");
            assert_eq!(answer, "Yes");
        }
        _ => panic!("Expected QuestionResult"),
    }
}

#[test]
fn test_app_count_wrapped_segment() {
    // -- Exec & Check
    assert_eq!(count_wrapped_segment("a", 10), 1);
    assert_eq!(count_wrapped_segment("1234567890", 10), 1);
    assert_eq!(count_wrapped_segment("12345678901", 10), 2);
    assert_eq!(count_wrapped_segment("123456789012345678901", 10), 3);
    assert_eq!(count_wrapped_segment("a 12345678901", 10), 3);
    assert_eq!(count_wrapped_segment("a 12345678901 ", 10), 3);
}

#[test]
fn test_timeline_item_tool_call_measurement_smoke() {
    let line = RenderLine::ToolCall {
        name: "bash".to_string(),
        preview: "cargo test --workspace".to_string(),
    };
    let item = TimelineItem::from_render_line(&line);
    assert_eq!(item.kind(), TimelineItemKind::ToolCall);
    assert!(item.visual_rows(80, false, &ThemeColors::default(), true) >= 1);
}

#[test]
fn test_timeline_item_maps_assistant_variant() {
    let line = RenderLine::AssistantText("hello".to_string());
    let item = TimelineItem::from_render_line(&line);
    assert!(matches!(item, TimelineItem::Assistant("hello")));
}

#[test]
fn test_timeline_item_maps_system_variant() {
    let line = RenderLine::SystemMsg("info".to_string());
    let item = TimelineItem::from_render_line(&line);
    assert!(matches!(item, TimelineItem::System("info")));
}

#[test]
fn test_timeline_entry_keys_are_stable() {
    let lines = vec![
        RenderLine::UserMessage("hello".to_string()),
        RenderLine::ToolCall {
            name: "bash".to_string(),
            preview: "cargo test".to_string(),
        },
        RenderLine::ToolResult {
            is_error: false,
            content: "ok".to_string(),
        },
    ];
    let entries = build_timeline_entries(&lines);
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].key.index, 0);
    assert_eq!(entries[0].key.kind, TimelineItemKind::User);
    assert!(!entries[0].key.streaming);
    assert_eq!(entries[1].key.index, 1);
    assert_eq!(entries[1].key.kind, TimelineItemKind::ToolCall);
    assert_eq!(entries[2].key.kind, TimelineItemKind::ToolResult);

    let stream = TimelineEntry::streaming(entries.len(), "partial");
    assert_eq!(stream.key.index, 3);
    assert_eq!(stream.key.kind, TimelineItemKind::StreamingAssistant);
    assert!(stream.key.streaming);
}

#[test]
fn test_per_item_expansion_state_changes_measurement() {
    let line = RenderLine::Reasoning {
        words: 3,
        content: "one\ntwo\nthree".to_string(),
    };
    let entry = TimelineEntry::from_render_line(0, &line);
    let colors = ThemeColors::default();
    let expanded: std::collections::HashSet<TimelineKey> = std::collections::HashSet::new();
    let collapsed_rows = entry.visual_rows_with_state(80, false, &expanded, &colors, true);

    let mut expanded = std::collections::HashSet::new();
    expanded.insert(entry.key);
    assert!(timeline_key_expanded(false, &expanded, &entry.key));
    let expanded_rows = entry.visual_rows_with_state(80, false, &expanded, &colors, true);
    assert!(expanded_rows > collapsed_rows);
}

#[test]
fn test_prepare_timeline_entries_row_sum() {
    let lines = vec![
        RenderLine::UserMessage("hello".to_string()),
        RenderLine::AssistantText("world".to_string()),
        RenderLine::SystemMsg("info".to_string()),
    ];
    let entries = build_timeline_entries(&lines);
    let colors = ThemeColors::default();
    let expanded = std::collections::HashSet::new();
    let mut temp_cache = crate::app::timeline::PreparedCache::new();
    let prepared = prepare_timeline_entries(
        &entries,
        80,
        false,
        &expanded,
        &colors,
        true,
        &mut temp_cache,
        false,
    );
    assert_eq!(prepared.len(), 3);
    let total: u16 = prepared.iter().map(|p| p.rows).sum();
    assert!(total >= 3, "at least 1 row per item; got {total}");
}

#[test]
fn test_snap_to_char_boundary_ascii() {
    let s = "hello world";
    assert_eq!(snap_to_char_boundary(s, 5), 5);
    assert_eq!(snap_to_char_boundary(s, 0), 0);
    assert_eq!(snap_to_char_boundary(s, 100), s.len());
}

#[test]
fn test_snap_to_char_boundary_multibyte() {
    let s = "héllo"; // 'é' is 2 bytes in UTF-8
    // Byte layout: h(1) é(2) l(1) l(1) o(1) = 6 bytes
    assert_eq!(snap_to_char_boundary(s, 1), 1); // after 'h' — valid boundary
    assert_eq!(snap_to_char_boundary(s, 2), 1); // mid-'é' — snaps back to after 'h'
    assert_eq!(snap_to_char_boundary(s, 3), 3); // after 'é' — valid boundary
}

#[test]
fn test_snap_to_char_boundary_emoji() {
    let s = "a🎉b"; // 🎉 is 4 bytes
    // Byte layout: a(1) 🎉(4) b(1) = 6 bytes
    assert_eq!(snap_to_char_boundary(s, 1), 1); // after 'a'
    assert_eq!(snap_to_char_boundary(s, 2), 1); // inside emoji, snap back to after 'a'
    assert_eq!(snap_to_char_boundary(s, 3), 1); // still inside emoji
    assert_eq!(snap_to_char_boundary(s, 4), 1); // still inside emoji
    assert_eq!(snap_to_char_boundary(s, 5), 5); // after emoji — valid
}

#[test]
fn test_streaming_revealed_prefix_snaps_multibyte() {
    // The typewriter reveal offset is byte-based; mid-character offsets must
    // snap back instead of panicking (regression: `build_prepared_entries` and
    // `draw_impl` slice the streaming text on this byte offset).
    // 'é' is 2 bytes; "héllo" = h(1) é(2) l(1) l(1) o(1) = 6 bytes.
    assert_eq!(streaming_revealed_prefix("héllo", 2), "h"); // mid-'é' → after 'h'
    assert_eq!(streaming_revealed_prefix("héllo", 3), "hé"); // after 'é'
    // '🙂' is 4 bytes; "a🙂b" = a(1) 🙂(4) b(1) = 6 bytes.
    assert_eq!(streaming_revealed_prefix("a🙂b", 2), "a"); // inside emoji → after 'a'
    assert_eq!(streaming_revealed_prefix("a🙂b", 5), "a🙂"); // after emoji
    assert_eq!(streaming_revealed_prefix("ab", 100), "ab"); // clamp to end
    assert_eq!(streaming_revealed_prefix("", 5), ""); // empty
    assert_eq!(streaming_revealed_prefix("plain", 0), ""); // zero reveal
}
#[test]
fn test_layout_engine_streaming_entry_grows_and_replaces() {
    use super::timeline::TimelineLayoutEngine;

    let colors = ThemeColors::default();
    let mut engine = TimelineLayoutEngine::new();
    let lines = vec![RenderLine::UserMessage("hello".to_string())];
    let expanded: std::collections::HashSet<TimelineKey> = std::collections::HashSet::new();

    // First streaming chunk
    engine.set_active_stream(Some("Hello world, this is the agent's streaming reply."));
    let prepared = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    // history (1 user line) + streaming entry
    assert_eq!(
        prepared.len(),
        2,
        "streaming entry must be appended to history"
    );
    assert!(prepared[1].rows > 0, "streaming entry must occupy rows");
    let before = prepared[1].rows;

    // Same chunk redrawn (no content change) — cache hit, no size change
    let prepared2 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared2.len(), 2);
    assert_eq!(prepared2[1].rows, before);

    // New chunk arrives → text changes → entry re-prepared and replaced
    engine.set_active_stream(Some(
        "Hello world, this is the agent's streaming reply. It continues with more.",
    ));
    let prepared3 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared3.len(), 2, "no duplicate streaming entries");
    assert!(prepared3[1].rows >= before);

    // Stream ends → streaming text removed → entry dropped
    engine.set_active_stream(None);
    let prepared4 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared4.len(), 1, "streaming entry must be dropped on end");

    // Multi-byte streaming text must layout without panicking
    engine.set_active_stream(Some("你好，我是CADE助手。🚀 正在处理你的请求。"));
    let prepared5 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared5.len(), 2);
    assert!(prepared5[1].rows > 0);
}

#[test]
fn test_layout_engine_live_reasoning_streams_inline() {
    use super::timeline::TimelineLayoutEngine;

    let colors = ThemeColors::default();
    let mut engine = TimelineLayoutEngine::new();
    let lines = vec![RenderLine::UserMessage("hello".to_string())];
    let expanded: std::collections::HashSet<TimelineKey> = std::collections::HashSet::new();

    // Thinking starts → live reasoning entry appears after history
    engine.set_active_reasoning(Some("Analyzing the request."));
    let prepared = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared.len(), 2, "live reasoning appended to history");
    let reasoning_rows = prepared[1].rows;
    assert!(reasoning_rows > 0);
    let joined = prepared[1]
        .lines
        .iter()
        .map(|l| l.to_string())
        .collect::<String>();
    assert!(joined.contains("THINKING"), "live thinking header shown");

    // Thinking grows → entry re-prepared, still after history
    engine.set_active_reasoning(Some(
        "Analyzing the request.\nSearching for relevant files.",
    ));
    let prepared2 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared2.len(), 2);
    assert!(
        prepared2[1].rows > reasoning_rows,
        "longer thinking → more rows"
    );

    // Reasoning stays BEFORE the streaming assistant entry
    engine.set_active_stream(Some("Here is my answer."));
    let prepared3 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared3.len(), 3, "history + reasoning + streaming");
    let reasoning_joined = prepared3[1]
        .lines
        .iter()
        .map(|l| l.to_string())
        .collect::<String>();
    let stream_joined = prepared3[2]
        .lines
        .iter()
        .map(|l| l.to_string())
        .collect::<String>();
    assert!(reasoning_joined.contains("THINKING"));
    assert!(
        stream_joined.contains("CADE"),
        "streaming assistant entry follows reasoning"
    );

    // Thinking commits → reasoning entry dropped, streaming remains
    engine.set_active_reasoning(None);
    let prepared4 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(
        prepared4.len(),
        2,
        "history + streaming after reasoning commit"
    );

    // Streaming ends too → history only
    engine.set_active_stream(None);
    let prepared5 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared5.len(), 1, "history only when nothing streams");

    // Very long thinking is windowed to the most recent lines (never a full
    // re-wrap of the entire reasoning transcript each frame).
    let long = (0..20)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    engine.set_active_reasoning(Some(&long));
    let prepared6 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared6.len(), 2);
    assert!(
        prepared6[1].rows <= 13,
        "live thinking is windowed, not full transcript height"
    );
}

#[test]
fn test_layout_engine_live_status_streams_inline() {
    use super::timeline::TimelineLayoutEngine;

    let colors = ThemeColors::default();
    let mut engine = TimelineLayoutEngine::new();
    let lines = vec![RenderLine::UserMessage("hello".to_string())];
    let expanded: std::collections::HashSet<TimelineKey> = std::collections::HashSet::new();

    // Working status appears as the bottom-most entry
    engine.set_active_status(Some("assessing… (Ctrl+c to interrupt · 2s · 0↑)"));
    let prepared = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared.len(), 2, "history + live status");
    let joined = prepared[1]
        .lines
        .iter()
        .map(|l| l.to_string())
        .collect::<String>();
    assert!(
        joined.contains("assessing"),
        "working status rendered inline: {joined}"
    );

    // Status text updates → entry re-prepared (no duplicates)
    engine.set_active_status(Some("● running tests…"));
    let prepared2 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared2.len(), 2);
    let joined2 = prepared2[1]
        .lines
        .iter()
        .map(|l| l.to_string())
        .collect::<String>();
    assert!(joined2.contains("running tests"));
    assert!(!joined2.contains("assessing"), "old status cleared");

    // Ordering: history + reasoning + streaming + status
    engine.set_active_reasoning(Some("Analyzing."));
    engine.set_active_stream(Some("Here is the reply."));
    let prepared3 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(
        prepared3.len(),
        4,
        "history + reasoning + streaming + status"
    );
    let tails = prepared3[1..]
        .iter()
        .map(|e| e.lines.iter().map(|l| l.to_string()).collect::<String>())
        .collect::<Vec<_>>();
    assert!(tails[0].contains("THINKING"));
    assert!(tails[1].contains("reply"));
    assert!(tails[2].contains("running tests"), "status is bottom-most");

    // Status ends → status entry dropped, dynamic tail collapses
    engine.set_active_status(None);
    let prepared4 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared4.len(), 3, "history + reasoning + streaming");
}

#[test]
fn test_toast_expires_after_ttl() {
    let toast = Toast {
        message: "hello".to_string(),
        level: ToastLevel::Success,
        created_at: Instant::now() - std::time::Duration::from_secs(5),
        ttl: std::time::Duration::from_secs(3),
    };
    assert!(toast.is_expired(), "toast should be expired after TTL");

    let fresh = Toast {
        message: "fresh".to_string(),
        level: ToastLevel::Info,
        created_at: Instant::now(),
        ttl: std::time::Duration::from_secs(3),
    };
    assert!(!fresh.is_expired(), "fresh toast should not be expired");
}

// -- tick_bg_pending_toast

#[test]
fn tick_bg_no_change_returns_false_and_leaves_toast_alone() {
    let mut last = 2usize;
    let mut toast: Option<Toast> = None;
    let wrote = tick_bg_pending_toast(2, &mut last, &mut toast);
    assert!(!wrote, "no change must not write toast");
    assert!(toast.is_none());
    assert_eq!(last, 2);
}

#[test]
fn tick_bg_singular_toast_for_one_pending() {
    let mut last = 0usize;
    let mut toast: Option<Toast> = None;
    let wrote = tick_bg_pending_toast(1, &mut last, &mut toast);
    assert!(wrote);
    let t = toast.expect("toast set");
    assert!(t.message.contains("Subagent finished"));
    assert!(matches!(t.level, ToastLevel::Success));
    assert_eq!(last, 1);
}

#[test]
fn tick_bg_plural_toast_for_many() {
    let mut last = 0usize;
    let mut toast: Option<Toast> = None;
    let wrote = tick_bg_pending_toast(4, &mut last, &mut toast);
    assert!(wrote);
    assert!(
        toast
            .as_ref()
            .unwrap()
            .message
            .contains("4 subagents finished"),
        "got: {}",
        toast.unwrap().message
    );
    assert_eq!(last, 4);
}

#[test]
fn tick_bg_drain_to_zero_resets_counter_without_toast() {
    let mut last = 3usize;
    let mut toast: Option<Toast> = None;
    let wrote = tick_bg_pending_toast(0, &mut last, &mut toast);
    assert!(!wrote, "draining to zero must not toast");
    assert!(toast.is_none());
    assert_eq!(last, 0, "counter must reset so future completions re-toast");
}

#[test]
fn tick_bg_after_drain_re_announces_new_completion() {
    let mut last = 0usize;
    let mut toast: Option<Toast> = None;
    // Simulates: REPL just drained (last=0), then a new completion arrives.
    let wrote = tick_bg_pending_toast(1, &mut last, &mut toast);
    assert!(wrote);
    assert_eq!(last, 1);
}

// -- PlanState scroll offset

#[test]
fn plan_state_has_scroll_offset_defaulting_to_zero() {
    let plan = PlanState {
        steps: vec![PlanStep {
            id: 1,
            description: "task".into(),
            is_done: false,
        }],
        is_visible: true,
        scroll_offset: 0,
    };
    assert_eq!(plan.scroll_offset, 0);
}

#[test]
fn plan_state_auto_scroll_targets_first_incomplete() {
    let mut plan = PlanState {
        steps: (1..=15)
            .map(|i| PlanStep {
                id: i,
                description: format!("Step {i}"),
                is_done: i <= 10,
            })
            .collect(),
        is_visible: true,
        scroll_offset: 0,
    };
    plan.auto_scroll(8); // visible_rows = 8
    // First incomplete is step 11 (index 10).
    // Should scroll so step 11 is visible.
    // With 8 visible rows, offset should be at least 10 - 7 = 3
    assert!(
        plan.scroll_offset >= 3,
        "scroll_offset={}",
        plan.scroll_offset
    );
    assert!(plan.scroll_offset <= 10);
}

#[test]
fn plan_state_auto_scroll_stays_zero_when_all_fit() {
    let mut plan = PlanState {
        steps: (1..=5)
            .map(|i| PlanStep {
                id: i,
                description: format!("Step {i}"),
                is_done: false,
            })
            .collect(),
        is_visible: true,
        scroll_offset: 0,
    };
    plan.auto_scroll(8);
    assert_eq!(plan.scroll_offset, 0);
}

#[test]
fn plan_state_auto_scroll_when_all_done() {
    let mut plan = PlanState {
        steps: (1..=15)
            .map(|i| PlanStep {
                id: i,
                description: format!("Step {i}"),
                is_done: true,
            })
            .collect(),
        is_visible: true,
        scroll_offset: 0,
    };
    plan.auto_scroll(8);
    // All done → scroll to bottom so last steps visible
    let max_offset = plan.steps.len().saturating_sub(8);
    assert_eq!(plan.scroll_offset, max_offset);
}

#[test]
#[ignore = "requires tty"]
fn set_plan_initializes_scroll_offset_zero() {
    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test".into(),
        "test-model".into(),
        None,
    );
    app.set_plan(vec!["a".into(), "b".into(), "c".into()]);
    assert_eq!(app.active_plan.as_ref().unwrap().scroll_offset, 0);
}

#[test]
#[ignore = "requires tty"]
fn test_scrolling_constraints_and_velocity_governor() {
    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test".into(),
        "test-model".into(),
        None,
    );

    // Initial state
    assert_eq!(app.scroll, 0);
    assert_eq!(app.scroll_target, 0);
    assert!(!app.selection_active);

    // 1. Verify ScrollUp increments scroll_target (elastic governor Option A)
    // At scroll_target = 0, scroll = 0, diff = 0 < max_buffer / 2 (which is 50), so increment should be 3
    let consumed = app.handle_scroll_mouse(crossterm::event::MouseEventKind::ScrollUp);
    assert!(consumed);
    assert_eq!(app.scroll_target, 3);
    assert!(!app.follow);

    // 2. Verify lock scrolling during drag (active selection)
    app.selection_active = true;
    let consumed_during_drag = app.handle_scroll_mouse(crossterm::event::MouseEventKind::ScrollUp);
    assert!(!consumed_during_drag);
    assert_eq!(app.scroll_target, 3); // unchanged

    // Key scrolling should also be blocked during drag
    let consumed_key_during_drag = app.handle_scroll_key(
        crossterm::event::KeyCode::PageUp,
        crossterm::event::KeyModifiers::empty(),
    );
    assert!(!consumed_key_during_drag);
    assert_eq!(app.scroll_target, 3); // unchanged

    // Disable selection/drag
    app.selection_active = false;

    // 3. Verify restrict to Scroll-Keys only
    // Non-scroll keys should not be consumed and should not modify scroll_target
    let consumed_non_scroll = app.handle_scroll_key(
        crossterm::event::KeyCode::Char('a'),
        crossterm::event::KeyModifiers::empty(),
    );
    assert!(!consumed_non_scroll);
    assert_eq!(app.scroll_target, 3); // unchanged

    // Valid scroll keys (e.g. PageUp) should be consumed and modify scroll_target
    let consumed_scroll_key = app.handle_scroll_key(
        crossterm::event::KeyCode::PageUp,
        crossterm::event::KeyModifiers::empty(),
    );
    assert!(consumed_scroll_key);
    assert!(app.scroll_target > 3);

    // Printable shifted characters (like Shift+J and Shift+K) must NOT be consumed
    assert!(!app.handle_scroll_key(
        crossterm::event::KeyCode::Char('J'),
        crossterm::event::KeyModifiers::SHIFT,
    ));
    assert!(!app.handle_scroll_key(
        crossterm::event::KeyCode::Char('K'),
        crossterm::event::KeyModifiers::SHIFT,
    ));

    // Dedicated follow-mode / scroll shortcuts (Alt+J, Ctrl+End) MUST be consumed
    assert!(app.handle_scroll_key(
        crossterm::event::KeyCode::Char('j'),
        crossterm::event::KeyModifiers::ALT,
    ));
    assert!(app.handle_scroll_key(
        crossterm::event::KeyCode::End,
        crossterm::event::KeyModifiers::CONTROL,
    ));
}

#[test]
fn test_sidebar_hidden_layout_calculation() {
    let area = ratatui::layout::Rect::new(0, 0, 140, 40);
    // When sidebar is not hidden and width >= SIDEBAR_BREAKPOINT (110), sidebar split is present
    let (main_normal, sidebar_normal) = if area.width >= crate::app::SIDEBAR_BREAKPOINT {
        let sidebar_w = crate::app::SIDEBAR_WIDTH.min(area.width.saturating_sub(24));
        let split = ratatui::layout::Layout::horizontal([
            ratatui::layout::Constraint::Min(24),
            ratatui::layout::Constraint::Length(sidebar_w),
        ])
        .split(area);
        (split[0], Some(split[1]))
    } else {
        (area, None)
    };
    assert!(sidebar_normal.is_some());
    assert!(main_normal.width < area.width);

    // When sidebar_hidden is true, sidebar_area must be None and main_area expands to full width
    let sidebar_hidden = true;
    let (main_hidden, sidebar_hidden_area) =
        if area.width >= crate::app::SIDEBAR_BREAKPOINT && !sidebar_hidden {
            let sidebar_w = crate::app::SIDEBAR_WIDTH.min(area.width.saturating_sub(24));
            let split = ratatui::layout::Layout::horizontal([
                ratatui::layout::Constraint::Min(24),
                ratatui::layout::Constraint::Length(sidebar_w),
            ])
            .split(area);
            (split[0], Some(split[1]))
        } else {
            (area, None)
        };
    assert!(sidebar_hidden_area.is_none());
    assert_eq!(main_hidden.width, area.width);
}

#[test]
#[ignore = "requires tty"]
fn test_toggle_sidebar_state_and_toast() {
    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test".into(),
        "test-model".into(),
        None,
    );
    assert!(!app.sidebar_hidden);

    // First toggle: hides sidebar
    let visible = app.toggle_sidebar();
    assert!(!visible);
    assert!(app.sidebar_hidden);
    assert!(app.draw_dirty);
    assert_eq!(
        app.toast.as_ref().map(|t| t.message.as_str()),
        Some("Sidebar hidden")
    );

    // Second toggle: reveals sidebar
    let visible = app.toggle_sidebar();
    assert!(visible);
    assert!(!app.sidebar_hidden);
    assert!(app.draw_dirty);
    assert_eq!(
        app.toast.as_ref().map(|t| t.message.as_str()),
        Some("Sidebar visible")
    );
}

#[test]
#[ignore = "requires tty"]
fn test_copy_selected_text_basic() {
    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test".into(),
        "test-model".into(),
        None,
    );
    app.push_silent(RenderLine::UserMessage("hello world".to_string()));

    app.messages_area = ratatui::layout::Rect::new(0, 0, 80, 24);

    app.selection_start = Some((4, 1));
    app.selection_current = Some((8, 1));
    app.selection_active = true;

    let result = app.copy_selected_text();
    assert!(result);
}

#[test]
fn test_prepared_cache_content_aware_invalidation() {
    use crate::app::RenderLine;
    use crate::app::timeline::*;
    use crate::colors::ThemeColors;

    let colors = ThemeColors::default();
    let expanded = std::collections::HashSet::new();
    let mut engine = TimelineLayoutEngine::new();

    // Helper to check if a prepared entry contains specific text
    let contains_text = |entry: &PreparedTimelineEntry, text: &str| -> bool {
        entry.lines.iter().any(|line| {
            let line_text: String = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect();
            line_text.contains(text)
        })
    };

    // Line 1: Original content
    let lines_v1 = vec![
        RenderLine::UserMessage("hello v1".to_string()),
        RenderLine::AssistantText("world v1".to_string()),
    ];

    // First layout pass — populates the cache
    let prepared_v1 = engine
        .layout_items(&lines_v1, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared_v1.len(), 2);
    assert!(contains_text(&prepared_v1[0], "hello v1"));

    // Trigger a global layout cache miss by resetting engine's cached version,
    // which forces rebuilding the timeline layout while retaining the per-item PreparedCache.
    engine.version = 0;

    // Second layout pass — exact same content, same version -> should be a cache hit at item-level
    let prepared_v1_hit = engine
        .layout_items(&lines_v1, 80, false, &expanded, &colors, true, 1)
        .to_vec();
    assert_eq!(prepared_v1_hit.len(), 2);
    assert!(contains_text(&prepared_v1_hit[0], "hello v1"));

    // Line 2: Modified content, same index, different content version
    let lines_v2 = vec![
        RenderLine::UserMessage("hello v2".to_string()), // modified
        RenderLine::AssistantText("world v1".to_string()), // unmodified
    ];

    // Third layout pass — content changed at index 0 -> should invalidate and rebuild index 0,
    // but reuse cached layout for index 1 (since index 1 content and width did not change).
    let prepared_v2 = engine
        .layout_items(&lines_v2, 80, false, &expanded, &colors, true, 2)
        .to_vec();
    assert_eq!(prepared_v2.len(), 2);
    assert!(contains_text(&prepared_v2[0], "hello v2")); // correctly updated (cache invalidated)
    assert!(contains_text(&prepared_v2[1], "world v1")); // correctly preserved (cache reused)
}

#[test]
fn test_prepared_cache_width_invalidation() {
    use crate::app::RenderLine;
    use crate::app::timeline::*;
    use crate::colors::ThemeColors;

    let colors = ThemeColors::default();
    let expanded = std::collections::HashSet::new();
    let mut engine = TimelineLayoutEngine::new();

    let lines = vec![RenderLine::UserMessage(
        "hello world this is a long wrapped line".to_string(),
    )];

    // Layout on width 80
    let prepared_80 = engine
        .layout_items(&lines, 80, false, &expanded, &colors, true, 1)
        .to_vec();

    // Layout on width 10 (forces word-wrapping to multiple rows)
    let prepared_10 = engine
        .layout_items(&lines, 10, false, &expanded, &colors, true, 1)
        .to_vec();

    // Width 10 should have significantly more wrapped rows than width 80
    assert!(prepared_10[0].rows > prepared_80[0].rows);
}

#[test]
#[ignore = "requires tty"]
fn test_toggle_last_collapsible_item_assistant_code_block() {
    use crate::app::RenderLine;
    use crate::app::TuiApp;
    use crate::app::timeline::TimelineItemKind;

    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test-agent".to_string(),
        "test-model".to_string(),
        None,
    );

    let mut md = String::from("# Heading\n\n```rust\n");
    for i in 1..=25 {
        md.push_str(&format!("println!(\"line {i}\");\n"));
    }
    md.push_str("```\n");

    app.lines
        .push(RenderLine::UserMessage("Write code".to_string()));
    app.lines.push(RenderLine::AssistantText(md));

    assert!(app.expanded_items.is_empty());

    // Pressing Ctrl+G calls toggle_last_collapsible_item
    app.toggle_last_collapsible_item();

    // The assistant message must now be marked in expanded_items!
    assert_eq!(app.expanded_items.len(), 1);
    let key = app.expanded_items.iter().next().unwrap();
    assert_eq!(key.kind, TimelineItemKind::Assistant);
    assert_eq!(key.index, 1);

    // Toggling again collapses it
    app.toggle_last_collapsible_item();
    assert!(app.expanded_items.is_empty());
}

#[test]
#[ignore = "requires tty"]
fn test_shift_j_and_k_delivered_to_editor_and_not_swallowed() {
    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test-agent".to_string(),
        "test-model".to_string(),
        None,
    );

    assert_eq!(app.editor.text(), "");
    assert_eq!(app.scroll, 0);
    assert_eq!(app.scroll_target, 0);

    let mut history = vec![];
    let mut hist_idx = None;

    // 1. Shift+J at scroll == 0 must append 'J' to editor buffer
    let key_j = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('J'),
        crossterm::event::KeyModifiers::SHIFT,
    );
    let res = app
        .handle_key_input(key_j, &mut history, &mut hist_idx)
        .unwrap();
    assert_eq!(res, None);
    assert_eq!(app.editor.text(), "J");
    assert_eq!(app.scroll, 0);

    // 2. Shift+K at scroll == 0 must append 'K' to editor buffer
    let key_k = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('K'),
        crossterm::event::KeyModifiers::SHIFT,
    );
    let res = app
        .handle_key_input(key_k, &mut history, &mut hist_idx)
        .unwrap();
    assert_eq!(res, None);
    assert_eq!(app.editor.text(), "JK");
    assert_eq!(app.scroll, 0);

    // 3. User scrolls up into history (scroll > 0)
    app.scroll = 20;
    app.scroll_target = 20;
    app.follow = false;

    // Shift+J when scroll > 0 must STILL type 'J' into editor and preserve scroll offset
    let res = app
        .handle_key_input(key_j, &mut history, &mut hist_idx)
        .unwrap();
    assert_eq!(res, None);
    assert_eq!(app.editor.text(), "JKJ");
    assert_eq!(app.scroll, 20);

    // 4. Dedicated follow-mode shortcut: Alt+Shift+J resets scroll to bottom
    let key_alt_j = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('J'),
        crossterm::event::KeyModifiers::ALT | crossterm::event::KeyModifiers::SHIFT,
    );
    let res = app
        .handle_key_input(key_alt_j, &mut history, &mut hist_idx)
        .unwrap();
    assert_eq!(res, None);
    assert_eq!(app.scroll_target, 0);
    assert!(app.follow);
    // Editor buffer remains unchanged
    assert_eq!(app.editor.text(), "JKJ");

    // 5. Dedicated follow-mode shortcut: Ctrl+End resets scroll to bottom
    app.scroll = 15;
    app.scroll_target = 15;
    app.follow = false;

    let key_ctrl_end = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::End,
        crossterm::event::KeyModifiers::CONTROL,
    );
    let res = app
        .handle_key_input(key_ctrl_end, &mut history, &mut hist_idx)
        .unwrap();
    assert_eq!(res, None);
    assert_eq!(app.scroll_target, 0);
    assert!(app.follow);
    assert_eq!(app.editor.text(), "JKJ");
}

#[test]
#[ignore = "requires tty"]
fn test_is_processing_and_toast_suppression() {
    use crate::app::{ToastLevel, TuiApp};

    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test-agent".to_string(),
        "test-model".to_string(),
        None,
    );

    // 1. Idle state: not processing, toasts are allowed
    assert!(!app.is_processing());
    app.show_toast("Idle toast", ToastLevel::Info);
    assert!(app.toast.is_some());
    assert_eq!(app.toast.as_ref().unwrap().message, "Idle toast");

    // 2. Starting thinking: is_processing becomes true, active toast is cleared
    let _arc = app.start_thinking("Processing task...");
    assert!(app.is_processing());
    assert!(
        app.toast.is_none(),
        "Active toast must be dismissed when processing starts"
    );

    // 3. Attempting to show toast while thinking: must be suppressed/dropped
    app.show_toast("Suppressed toast", ToastLevel::Warning);
    assert!(
        app.toast.is_none(),
        "Toasts must not be queued or displayed while processing"
    );

    // 4. Stop thinking: returns to idle
    let _ = app.stop_thinking();
    assert!(!app.is_processing());
    app.show_toast("After processing toast", ToastLevel::Success);
    assert!(app.toast.is_some());
    assert_eq!(
        app.toast.as_ref().unwrap().message,
        "After processing toast"
    );

    // 5. Streaming active: is_processing is true, toast dropped
    app.streaming_active = true;
    assert!(app.is_processing());
    app.toast = None;
    app.show_toast("Streaming toast", ToastLevel::Info);
    assert!(
        app.toast.is_none(),
        "Toasts must not be queued or displayed while streaming"
    );
}

#[test]
fn test_plan_lifecycle_state_and_height_calculation() {
    // 1. None active_plan produces height 0
    let no_plan: Option<PlanState> = None;
    let height_none = if let Some(plan) = &no_plan {
        if plan.is_visible {
            (plan.steps.len() as u16 + 2).min(10).max(4)
        } else {
            0
        }
    } else {
        0
    };
    assert_eq!(height_none, 0);

    // 2. Visible plan with 2 steps produces height 4
    let mut plan = PlanState {
        steps: vec![
            PlanStep {
                id: 1,
                description: "Investigate problem".into(),
                is_done: false,
            },
            PlanStep {
                id: 2,
                description: "Implement fix".into(),
                is_done: false,
            },
        ],
        is_visible: true,
        scroll_offset: 0,
    };
    let height_2 = (plan.steps.len() as u16 + 2).min(10).max(4);
    assert_eq!(height_2, 4);

    // 3. Step completion update
    plan.steps[0].is_done = true;
    assert!(plan.steps[0].is_done);
    assert!(!plan.steps[1].is_done);

    // 4. Hidden plan produces height 0
    plan.is_visible = false;
    let height_hidden = if plan.is_visible {
        (plan.steps.len() as u16 + 2).min(10).max(4)
    } else {
        0
    };
    assert_eq!(height_hidden, 0);
}

#[test]
fn test_plan_update_json_event_payload_conformance() {
    let payload = serde_json::json!({
        "message_type": "plan_update",
        "plan": {
            "title": "Roadmap",
            "steps": [
                { "id": 1, "description": "Step 1", "is_done": false },
                { "id": 2, "description": "Step 2", "is_done": true }
            ]
        }
    });

    let plan_obj = payload.get("plan").expect("must contain plan");
    let steps_arr = plan_obj
        .get("steps")
        .and_then(|v| v.as_array())
        .expect("steps array");
    assert_eq!(steps_arr.len(), 2);

    let step1_desc = steps_arr[0]
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap();
    let step1_done = steps_arr[0]
        .get("is_done")
        .and_then(|v| v.as_bool())
        .unwrap();
    assert_eq!(step1_desc, "Step 1");
    assert!(!step1_done);

    let step2_done = steps_arr[1]
        .get("is_done")
        .and_then(|v| v.as_bool())
        .unwrap();
    assert!(step2_done);
}

#[test]
fn test_multi_select_other_combines_checked_and_custom_text() {
    use crate::app::ActiveQuestionState;
    use crate::overlay_component::OverlayComponent;
    use crate::question::{Question, QuestionAnswer, QuestionOption};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let q = Question {
        header: "Test".into(),
        text: "Select options".into(),
        options: vec![
            QuestionOption {
                label: "Opt1".into(),
                description: "".into(),
            },
            QuestionOption {
                label: "Opt2".into(),
                description: "".into(),
            },
        ],
        multi_select: true,
        allow_other: true,
        progress: None,
    };
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState::new(q, tx);

    // Toggle Opt1 (cursor starts at 0)
    state.handle_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(state.draw_state.checked[0]);

    // Move down to Other (idx 2)
    state.handle_input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    state.handle_input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(state.draw_state.cursor_pos, state.draw_state.other_idx);

    // Type "Custom"
    for c in "Custom".chars() {
        state.handle_input(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(state.draw_state.custom_text, "Custom");

    // Press Enter on Other
    let res = state.handle_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        res,
        crate::overlay_component::OverlayInputResult::Dismiss
    ));

    let ans = rx
        .try_recv()
        .expect("must receive answer")
        .expect("answer is some");
    assert_eq!(
        ans,
        QuestionAnswer::Multi(vec!["Opt1".into(), "Custom".into()])
    );
}

#[test]
fn test_question_modal_renders_centered_with_radios_and_backdrop() {
    use crate::question::{Question, QuestionOption};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let question = Question {
        header: "Database".to_string(),
        text: "Select a persistent storage backend:".to_string(),
        options: vec![
            QuestionOption {
                label: "PostgreSQL".to_string(),
                description: "Standard ACID relational store".to_string(),
            },
            QuestionOption {
                label: "SQLite".to_string(),
                description: "Zero-config embedded store".to_string(),
            },
        ],
        multi_select: false,
        allow_other: false,
        progress: None,
    };

    let (tx, _rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(question),
        tx: Some(tx),
        result: None,
    };

    let colors = ThemeColors::default();
    terminal
        .draw(|f| {
            let full_area = f.area();
            state.render_overlay(f, full_area, &colors);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered: String = (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Modal title & questions
    assert!(
        rendered.contains("Database"),
        "must contain header in title"
    );
    assert!(rendered.contains("Select a persistent storage backend:"));

    // Options with radio indicators
    assert!(
        rendered.contains("(•)"),
        "initial selected option has (•) radio"
    );
    assert!(rendered.contains("( )"), "unselected option has ( ) radio");
    assert!(rendered.contains("PostgreSQL"));
    assert!(rendered.contains("SQLite"));
    assert!(rendered.contains("Standard ACID relational store"));

    // Navigation and quick-pick hint
    assert!(rendered.contains("PgUp/PgDn details"));

    // The active decision reserves the input region rather than the timeline.
    assert!(state.inline_height(24) > 0);
}

#[test]
fn test_question_modal_keeps_approval_choice_and_hint_visible_with_long_details() {
    use crate::question::{Question, QuestionOption};
    use ratatui::{Terminal, backend::TestBackend};

    let question = Question {
        header: "Approve file.write".into(),
        text: (0..20)
            .map(|i| format!("Argument {i}: a long value\n"))
            .collect(),
        options: (0..12)
            .map(|i| QuestionOption {
                label: format!("Choice {i}"),
                description: format!("Description {i}"),
            })
            .collect(),
        multi_select: false,
        allow_other: false,
        progress: None,
    };
    let mut state = ActiveQuestionDrawState::new(question);
    state.cursor_pos = 11;
    let mut terminal = Terminal::new(TestBackend::new(60, 13)).unwrap();
    terminal
        .draw(|frame| {
            crate::app::layout::question::render_question_modal(
                frame,
                &state,
                frame.area(),
                &ThemeColors::default(),
            );
        })
        .unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(
        screen.contains("Choice 11"),
        "focused choice must be visible: {screen}"
    );
    assert!(
        screen.contains("Esc deny"),
        "approval hint must be visible: {screen}"
    );
}

#[test]
fn test_question_modal_single_select_number_key_resolves() {
    use crate::question::{Question, QuestionAnswer, QuestionOption};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let question = Question {
        header: "Choice".to_string(),
        text: "Pick an environment:".to_string(),
        options: vec![
            QuestionOption {
                label: "Staging".to_string(),
                description: "".to_string(),
            },
            QuestionOption {
                label: "Production".to_string(),
                description: "".to_string(),
            },
        ],
        multi_select: false,
        allow_other: false,
        progress: None,
    };

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(question),
        tx: Some(tx),
        result: None,
    };

    // Press '2' to pick Production immediately
    let key = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE);
    let res = state.handle_input(key);

    assert!(matches!(res, OverlayInputResult::Dismiss));
    let received = rx.try_recv().expect("must receive answer on channel");
    assert_eq!(
        received,
        Some(QuestionAnswer::Single("Production".to_string()))
    );
}

#[test]
#[ignore = "requires tty"]
fn test_turn_count_increments_on_user_message_and_queued_count() {
    use crate::app::RenderLine;
    use crate::app::TuiApp;

    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test-agent".to_string(),
        "test-model".to_string(),
        None,
    );

    assert_eq!(app.turn_count, 0);
    assert_eq!(app.queued_count, 0);

    // Push non-user message
    app.push_silent(RenderLine::SystemMsg("Welcome".to_string()));
    assert_eq!(app.turn_count, 0);

    // Push user message via push_silent
    app.push_silent(RenderLine::UserMessage("Hello agent".to_string()));
    assert_eq!(app.turn_count, 1);

    // Set and increment turn count
    app.increment_turn();
    assert_eq!(app.turn_count, 2);

    app.set_turn_count(10);
    assert_eq!(app.turn_count, 10);

    // Test queued count tracking
    app.queued_count = 3;
    assert_eq!(app.queued_count, 3);
}

#[test]
fn test_question_modal_navigation_and_enter_select() {
    use crate::question::{Question, QuestionAnswer, QuestionOption};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let question = Question {
        header: "Choice".to_string(),
        text: "Select item:".to_string(),
        options: vec![
            QuestionOption {
                label: "Alpha".to_string(),
                description: "".to_string(),
            },
            QuestionOption {
                label: "Beta".to_string(),
                description: "".to_string(),
            },
        ],
        multi_select: false,
        allow_other: false,
        progress: None,
    };

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(question),
        tx: Some(tx),
        result: None,
    };

    // Navigate down to item 1 (Beta)
    let key_down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    let res = state.handle_input(key_down);
    assert!(matches!(res, OverlayInputResult::Consumed));
    assert_eq!(state.draw_state.cursor_pos, 1);

    // Press Enter to confirm
    let key_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let res2 = state.handle_input(key_enter);
    assert!(matches!(res2, OverlayInputResult::Dismiss));

    let received = rx.try_recv().expect("must receive answer");
    assert_eq!(received, Some(QuestionAnswer::Single("Beta".to_string())));
}

#[test]
fn test_question_modal_esc_dismisses_cleanly() {
    use crate::question::Question;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let question = Question {
        header: "Confirm".to_string(),
        text: "Proceed?".to_string(),
        options: vec![],
        multi_select: false,
        allow_other: false,
        progress: None,
    };

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(question),
        tx: Some(tx),
        result: None,
    };

    let key_esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    let res = state.handle_input(key_esc);
    assert!(matches!(res, OverlayInputResult::Dismiss));

    let received = rx.try_recv().expect("must receive cancellation");
    assert_eq!(received, None);
}

#[test]
fn test_question_modal_multi_select_spacebar_toggle() {
    use crate::question::{Question, QuestionAnswer, QuestionOption};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let question = Question {
        header: "Languages".to_string(),
        text: "Select languages you know:".to_string(),
        options: vec![
            QuestionOption {
                label: "Rust".to_string(),
                description: "".to_string(),
            },
            QuestionOption {
                label: "Go".to_string(),
                description: "".to_string(),
            },
            QuestionOption {
                label: "Python".to_string(),
                description: "".to_string(),
            },
        ],
        multi_select: true,
        allow_other: false,
        progress: None,
    };

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(question),
        tx: Some(tx),
        result: None,
    };

    // 1. Press Space on Option 0 (Rust) -> toggles to true
    let key_space = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE);
    let res = state.handle_input(key_space);
    assert!(matches!(res, OverlayInputResult::Consumed));
    assert!(state.draw_state.checked[0]);

    // 2. Navigate Down to Option 1 (Go)
    let key_down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    state.handle_input(key_down);
    assert_eq!(state.draw_state.cursor_pos, 1);

    // 3. Press Space on Option 1 -> toggles to true
    state.handle_input(key_space);
    assert!(state.draw_state.checked[1]);

    // 4. Press Space again on Option 1 -> toggles back to false
    state.handle_input(key_space);
    assert!(!state.draw_state.checked[1]);

    // 5. Press '3' to toggle Option 2 (Python) without submitting
    let key_3 = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE);
    let res3 = state.handle_input(key_3);
    assert!(matches!(res3, OverlayInputResult::Consumed));
    assert!(state.draw_state.checked[2]);
    assert_eq!(state.draw_state.cursor_pos, 2);

    // 6. Navigate to Submit button (submit_idx is 3)
    state.handle_input(key_down);
    assert_eq!(state.draw_state.cursor_pos, state.draw_state.submit_idx);

    // 7. Press Enter on Submit button
    let key_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let res_enter = state.handle_input(key_enter);
    assert!(matches!(res_enter, OverlayInputResult::Dismiss));

    // Must receive both Rust and Python
    let received = rx.try_recv().expect("must receive multi-selection");
    assert_eq!(
        received,
        Some(QuestionAnswer::Multi(vec![
            "Rust".to_string(),
            "Python".to_string(),
        ]))
    );
}

#[test]
fn test_multi_select_submit_combines_checked_and_custom_text() {
    use crate::app::ActiveQuestionState;
    use crate::overlay_component::OverlayComponent;
    use crate::question::{Question, QuestionAnswer, QuestionOption};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let q = Question {
        header: "Test".into(),
        text: "Select options".into(),
        options: vec![
            QuestionOption {
                label: "Opt1".into(),
                description: "".into(),
            },
            QuestionOption {
                label: "Opt2".into(),
                description: "".into(),
            },
        ],
        multi_select: true,
        allow_other: true,
        progress: None,
    };
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState::new(q, tx);

    // Move to Opt2 (idx 1) and toggle
    state.handle_input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    state.handle_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(state.draw_state.checked[1]);

    // Move to Other (idx 2) and type "Custom2"
    state.handle_input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    for c in "Custom2".chars() {
        state.handle_input(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }

    // Move to Confirm selection (idx 3)
    state.handle_input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(state.draw_state.cursor_pos, state.draw_state.submit_idx);

    // Press Enter on Submit
    let res = state.handle_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(
        res,
        crate::overlay_component::OverlayInputResult::Dismiss
    ));

    let ans = rx
        .try_recv()
        .expect("must receive answer")
        .expect("answer is some");
    assert_eq!(
        ans,
        QuestionAnswer::Multi(vec!["Opt2".into(), "Custom2".into()])
    );
}

#[test]
fn test_question_modal_multi_select_renders_checkboxes() {
    use crate::question::{Question, QuestionOption};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let question = Question {
        header: "Tags".to_string(),
        text: "Select applicable tags:".to_string(),
        options: vec![
            QuestionOption {
                label: "Backend".to_string(),
                description: "".to_string(),
            },
            QuestionOption {
                label: "Frontend".to_string(),
                description: "".to_string(),
            },
        ],
        multi_select: true,
        allow_other: false,
        progress: None,
    };

    let (tx, _rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(question),
        tx: Some(tx),
        result: None,
    };

    // Pre-check the first option
    state.draw_state.checked[0] = true;

    let colors = ThemeColors::default();
    terminal
        .draw(|f| {
            let full_area = f.area();
            state.render_overlay(f, full_area, &colors);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered: String = (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("[✓]"), "checked option must show [✓]");
    assert!(rendered.contains("[ ]"), "unchecked option must show [ ]");
    assert!(
        rendered.contains("Confirm selection"),
        "submit choice must remain visible"
    );
    assert!(rendered.contains("Enter"));
}

#[test]
fn test_question_modal_freeform_other_text_editing_and_submit() {
    use crate::question::{Question, QuestionAnswer, QuestionOption};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let question = Question {
        header: "Custom".to_string(),
        text: "Select or specify:".to_string(),
        options: vec![QuestionOption {
            label: "Option A".to_string(),
            description: "".to_string(),
        }],
        multi_select: false,
        allow_other: true,
        progress: None,
    };

    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(question),
        tx: Some(tx),
        result: None,
    };

    // 1. Move cursor to other_idx (index 1)
    let key_down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    state.handle_input(key_down);
    assert_eq!(state.draw_state.cursor_pos, state.draw_state.other_idx);

    // 2. Type "FooBar"
    for c in "FooBar".chars() {
        state.handle_input(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    assert_eq!(state.draw_state.custom_text, "FooBar");
    assert_eq!(state.draw_state.custom_cursor_pos, 6);

    // 3. Move cursor left 3 positions (between Foo and Bar)
    let key_left = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
    state.handle_input(key_left);
    state.handle_input(key_left);
    state.handle_input(key_left);
    assert_eq!(state.draw_state.custom_cursor_pos, 3);

    // 4. Type " "
    state.handle_input(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(state.draw_state.custom_text, "Foo Bar");
    assert_eq!(state.draw_state.custom_cursor_pos, 4);

    // 5. Backspace removes the space
    let key_backspace = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
    state.handle_input(key_backspace);
    assert_eq!(state.draw_state.custom_text, "FooBar");
    assert_eq!(state.draw_state.custom_cursor_pos, 3);

    // 6. Press Enter on "Other" to submit
    let key_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let res = state.handle_input(key_enter);
    assert!(matches!(res, OverlayInputResult::Dismiss));

    let received = rx.try_recv().expect("must receive custom answer");
    assert_eq!(received, Some(QuestionAnswer::Custom("FooBar".to_string())));
}

#[test]
fn test_question_modal_overflow_scrolling_and_small_viewport() {
    use crate::question::{Question, QuestionOption};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    // Very constrained viewport: 50 columns x 10 rows
    let backend = TestBackend::new(50, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    let options: Vec<QuestionOption> = (1..=12)
        .map(|i| QuestionOption {
            label: format!("Choice {i}"),
            description: format!("Description for {i}"),
        })
        .collect();

    let question = Question {
        header: "Long List".to_string(),
        text: "Please pick an item from this long list:".to_string(),
        options,
        multi_select: false,
        allow_other: false,
        progress: None,
    };

    let (tx, _rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(question),
        tx: Some(tx),
        result: None,
    };

    let colors = ThemeColors::default();

    // Render at top of list
    terminal
        .draw(|f| {
            let full_area = f.area();
            state.render_overlay(f, full_area, &colors);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    assert_eq!(buffer.area.width, 50);
    assert_eq!(buffer.area.height, 10);

    // Navigate down to item 10 to exercise scrolling logic
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let key_down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    for _ in 0..10 {
        state.handle_input(key_down);
    }
    assert_eq!(state.draw_state.cursor_pos, 10);

    // Render with cursor at item 10 — must not panic and must calculate scroll cleanly
    terminal
        .draw(|f| {
            let full_area = f.area();
            state.render_overlay(f, full_area, &colors);
        })
        .unwrap();
}

#[test]
fn test_question_modal_sequence_progression_and_draft_preservation() {
    use crate::question::{Question, QuestionAnswer, QuestionOption};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let colors = ThemeColors::default();

    // 1. Question 1 of 2
    let q1 = Question {
        header: "Step 1".to_string(),
        text: "Select environment:".to_string(),
        options: vec![
            QuestionOption {
                label: "Dev".to_string(),
                description: "".to_string(),
            },
            QuestionOption {
                label: "Prod".to_string(),
                description: "".to_string(),
            },
        ],
        multi_select: false,
        allow_other: false,
        progress: Some((1, 2)),
    };

    let (tx1, mut rx1) = tokio::sync::oneshot::channel();
    let mut state1 = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(q1),
        tx: Some(tx1),
        result: None,
    };

    terminal
        .draw(|f| {
            let full_area = f.area();
            state1.render_overlay(f, full_area, &colors);
        })
        .unwrap();

    let buffer1 = terminal.backend().buffer();
    let rendered1: String = (0..buffer1.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer1.area.width {
                line.push_str(buffer1[(x, y)].symbol());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered1.contains("1/2"));
    assert!(rendered1.contains("Dev"));

    // Select option 1 via key '1'
    let res1 = state1.handle_input(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    assert!(matches!(res1, OverlayInputResult::Dismiss));
    assert_eq!(
        rx1.try_recv().unwrap(),
        Some(QuestionAnswer::Single("Dev".to_string()))
    );

    // 2. Question 2 of 2
    let q2 = Question {
        header: "Step 2".to_string(),
        text: "Select features:".to_string(),
        options: vec![
            QuestionOption {
                label: "Auth".to_string(),
                description: "".to_string(),
            },
            QuestionOption {
                label: "Metrics".to_string(),
                description: "".to_string(),
            },
        ],
        multi_select: true,
        allow_other: false,
        progress: Some((2, 2)),
    };

    let (tx2, mut rx2) = tokio::sync::oneshot::channel();
    let mut state2 = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(q2),
        tx: Some(tx2),
        result: None,
    };

    terminal
        .draw(|f| {
            let full_area = f.area();
            state2.render_overlay(f, full_area, &colors);
        })
        .unwrap();

    let buffer2 = terminal.backend().buffer();
    let rendered2: String = (0..buffer2.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer2.area.width {
                line.push_str(buffer2[(x, y)].symbol());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered2.contains("2/2"));
    assert!(rendered2.contains("Auth"));
    assert!(rendered2.contains("Metrics"));

    // Toggle both Auth and Metrics via '1' and '2', navigate to Submit, and Enter
    state2.handle_input(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
    state2.handle_input(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
    state2.handle_input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    state2.handle_input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let res2 = state2.handle_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(res2, OverlayInputResult::Dismiss));

    assert_eq!(
        rx2.try_recv().unwrap(),
        Some(QuestionAnswer::Multi(vec![
            "Auth".to_string(),
            "Metrics".to_string(),
        ]))
    );

    assert!(state1.inline_height(24) > 0);
    assert!(state2.inline_height(24) > 0);
}

#[test]
fn test_app_state_reset_context() -> Result<()> {
    // -- Setup & Fixtures
    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test-agent".into(),
        "test-model".into(),
        None,
    );
    app.set_context_pct(99);
    app.footer_extra = Some("metrics info".to_string());
    app.modified_files_tracker.record_mutation(
        "/workspace/src/lib.rs",
        "old content",
        "new content",
    );
    assert_eq!(app.context_pct, Some(99));
    assert_eq!(app.token_history, vec![99]);
    assert_eq!(app.footer_extra.as_deref(), Some("metrics info"));
    assert_eq!(app.modified_files_tracker.len(), 1);

    // -- Exec
    app.reset_context();

    // -- Check
    assert!(app.context_pct.is_none(), "context_pct should be cleared to None");
    assert!(app.token_history.is_empty(), "token_history should be cleared");
    assert!(app.footer_extra.is_none(), "footer_extra should be cleared");
    assert!(
        app.modified_files_tracker.is_empty(),
        "modified_files_tracker must be cleared on reset_context"
    );
    Ok(())
}

#[test]
fn test_question_modal_renders_advisory_badges_and_section() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use crate::question::{Question, QuestionOption};

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let colors = ThemeColors::default();

    let question = Question {
        header: "Approve bash · [Risk: Low] [Scope: Aligned]".to_string(),
        text: "Approval app-1: allow 'bash' to run?\n\nReason: Run tests\n\nAdvisory: Diff looks aligned\nProvider: jev-1.13.0\n\nArguments:\n```json\n{\n  \"command\": \"cargo test\"\n}\n```".to_string(),
        options: vec![
            QuestionOption {
                label: "Allow once".to_string(),
                description: "Approve single run".to_string(),
            },
            QuestionOption {
                label: "Deny".to_string(),
                description: "Reject tool execution".to_string(),
            },
        ],
        multi_select: false,
        allow_other: false,
        progress: None,
    };

    let (tx, _rx) = tokio::sync::oneshot::channel();
    let mut state = ActiveQuestionState {
        draw_state: ActiveQuestionDrawState::new(question),
        tx: Some(tx),
        result: None,
        approval_id: None,
    };

    terminal
        .draw(|f| {
            let full_area = f.area();
            state.render_overlay(f, full_area, &colors);
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered: String = (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            for x in 0..buffer.area.width {
                line.push_str(buffer[(x, y)].symbol());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        rendered.contains("[Risk: Low]"),
        "modal header must render [Risk: Low] badge"
    );
    assert!(
        rendered.contains("[Scope: Aligned]"),
        "modal header must render [Scope: Aligned] badge"
    );
    assert!(
        rendered.contains("Advisory: Diff looks aligned"),
        "modal body must render advisory summary"
    );
    assert!(
        rendered.contains("Provider: jev-1.13.0"),
        "modal body must render advisory provider"
    );
}
