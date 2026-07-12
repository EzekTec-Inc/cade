#[allow(unused)]
type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>; // For tests.

use super::render::count_wrapped_segment;
use super::*;

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
