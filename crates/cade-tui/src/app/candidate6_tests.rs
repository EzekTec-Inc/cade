//! Behavioral tests at the ratatui frame and decision/event seams.
use super::permission_overlay::{PermissionOverlay, PermissionVerdict};
use super::timeline::{
    PreparedCache, TimelineLayoutEngine, build_timeline_entries, prepare_timeline_entries,
    render_timeline_viewport,
};
use super::{ActiveQuestionDrawState, ActiveQuestionState, RenderLine};
use crate::colors::{ThemeColors, ThemeColorsExt};
use crate::overlay_component::{OverlayComponent, OverlayInputResult};
use crate::question::{Question, QuestionAnswer, QuestionOption};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn question(multi: bool) -> Question {
    Question {
        header: "Choose target".into(),
        text: "Review **these targets** before continuing.".into(),
        options: (0..12).map(|i| QuestionOption {
            label: format!("Target {i}"),
            description: format!("Target {i} has a long description with wide characters 界界 and additional wrapped details."),
        }).collect(),
        multi_select: multi, allow_other: true, progress: Some((1, 2)),
    }
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn draw(
    terminal: &mut Terminal<TestBackend>,
    overlay: &mut dyn OverlayComponent,
    colors: &ThemeColors,
) {
    terminal
        .draw(|frame| {
            let area = frame.area();
            overlay.render_overlay(frame, area, colors);
        })
        .unwrap();
}

#[test]
fn candidate6_adaptive_selected_row_resize_and_mouse_hit_testing() {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut dialog = ActiveQuestionState::new(question(false), tx);
    let colors = ThemeColors::default();
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    draw(&mut terminal, &mut dialog, &colors);
    let wide = dialog.draw_state.geometry.lock().unwrap().panel;
    assert!(wide.x > 0 && wide.y > 0);
    assert_eq!(dialog.inline_height_for(Rect::new(0, 0, 120, 40)), 0);
    for _ in 0..11 {
        dialog.handle_event(&key(KeyCode::Down));
    }
    draw(&mut terminal, &mut dialog, &colors);
    assert!(screen(&terminal).contains("Target 11"));

    terminal.backend_mut().resize(32, 8);
    terminal.resize(Rect::new(0, 0, 32, 8)).unwrap();
    draw(&mut terminal, &mut dialog, &colors);
    assert!(screen(&terminal).contains("Target 11"));
    assert!(screen(&terminal).contains("Esc cancel"));
    let geometry = dialog.draw_state.geometry.lock().unwrap().clone();
    assert_eq!(geometry.panel.width, 32);
    assert!(dialog.inline_height_for(Rect::new(0, 0, 32, 8)) > 0);
    let rect = geometry
        .choices
        .iter()
        .find(|(index, _)| *index == 11)
        .unwrap()
        .1;
    assert_eq!(
        dialog.handle_event(&Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 1,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        })),
        OverlayInputResult::Dismiss
    );
    assert_eq!(
        rx.try_recv().unwrap(),
        Some(QuestionAnswer::Single("Target 11".into()))
    );
}

#[test]
fn candidate6_freeform_digits_spaces_cursor_and_wrapped_paste_stay_typed() {
    let mut q = question(false);
    q.header = "Approve bash.exec".into();
    q.options.truncate(1);
    q.options[0].label = "Allow once".into();
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut dialog = ActiveQuestionState::new(q, tx);
    dialog.handle_event(&key(KeyCode::Down));
    dialog.handle_event(&Event::Paste("Allow once".into()));
    dialog.handle_event(&key(KeyCode::Home));
    dialog.handle_event(&key(KeyCode::Char('1')));
    dialog.handle_event(&key(KeyCode::Char(' ')));
    assert_eq!(dialog.draw_state.custom_text, "1 Allow once");
    dialog.handle_event(&key(KeyCode::Backspace));
    dialog.handle_event(&key(KeyCode::Backspace));
    let mut terminal = Terminal::new(TestBackend::new(32, 8)).unwrap();
    draw(&mut terminal, &mut dialog, &ThemeColors::default());
    assert!(screen(&terminal).contains('▌'));
    assert_eq!(
        dialog.handle_event(&key(KeyCode::Enter)),
        OverlayInputResult::Dismiss
    );
    assert_eq!(
        rx.try_recv().unwrap(),
        Some(QuestionAnswer::Custom("Allow once".into()))
    );

    // A long, double-width answer and a middle insertion cursor remain visible.
    let mut dialog = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(question(false)),
        tx: None,
        result: None,
    };
    dialog.draw_state.cursor_pos = dialog.draw_state.other_idx;
    dialog.handle_event(&Event::Paste(
        "界界 a long response with spaces ".repeat(20),
    ));
    dialog.handle_event(&key(KeyCode::Home));
    for _ in 0..15 {
        dialog.handle_event(&key(KeyCode::Right));
    }
    draw(&mut terminal, &mut dialog, &ThemeColors::default());
    assert!(
        screen(&terminal).contains('▌'),
        "middle cursor must be visible"
    );
}

#[test]
fn candidate6_multiselect_freeform_space_and_mouse_confirm() {
    let mut q = question(true);
    q.options.truncate(2);
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut dialog = ActiveQuestionState::new(q, tx);
    dialog.draw_state.cursor_pos = dialog.draw_state.submit_idx;
    assert_eq!(
        dialog.handle_event(&key(KeyCode::Char(' '))),
        OverlayInputResult::Consumed
    );
    assert!(
        rx.try_recv().is_err(),
        "empty confirmation cannot silently submit a selection"
    );
    dialog.draw_state.cursor_pos = 0;
    dialog.handle_event(&key(KeyCode::Char(' ')));
    dialog.handle_event(&key(KeyCode::Down));
    dialog.handle_event(&key(KeyCode::Down));
    for c in "custom 1".chars() {
        dialog.handle_event(&key(KeyCode::Char(c)));
    }
    assert_eq!(dialog.draw_state.custom_text, "custom 1");
    dialog.handle_event(&key(KeyCode::Down));
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    draw(&mut terminal, &mut dialog, &ThemeColors::default());
    assert!(screen(&terminal).contains("[✓]"));
    let rect = dialog
        .draw_state
        .geometry
        .lock()
        .unwrap()
        .choices
        .iter()
        .find(|(index, _)| *index == dialog.draw_state.submit_idx)
        .unwrap()
        .1;
    dialog.handle_event(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        rx.try_recv().unwrap(),
        Some(QuestionAnswer::Multi(vec![
            "Target 0".into(),
            "custom 1".into()
        ]))
    );
}

#[test]
fn candidate6_permission_details_scroll_themed_and_pointer_verdict() {
    let (tx, mut rx) = tokio::sync::oneshot::channel();
    let mut permission = PermissionOverlay::new("bash.exec", "git status\n".repeat(30), tx);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let colors = ThemeColors::default();
    draw(&mut terminal, &mut permission, &colors);
    assert!(screen(&terminal).contains("git status"));
    assert!(screen(&terminal).contains("Allow for this session"));
    permission.handle_event(&key(KeyCode::PageDown));
    permission.handle_event(&Event::Paste("Allow for this session".into()));
    assert!(
        rx.try_recv().is_err(),
        "paste must not authorize a permission"
    );
    permission.handle_event(&key(KeyCode::Down));
    permission.handle_event(&key(KeyCode::Down));
    draw(&mut terminal, &mut permission, &colors);
    // The same selected-row presentation as a normal question, with the
    // permission's warning accent sourced from the active theme.
    let buffer = terminal.backend().buffer();
    assert!(
        buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "◆" && cell.fg == colors.c_warning())
    );
    let row = screen(&terminal)
        .lines()
        .position(|line| line.contains("3. (•) Allow for this session"))
        .unwrap() as u16;
    permission.handle_event(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 30,
        row,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(rx.try_recv().unwrap(), PermissionVerdict::AllowSession);
}

#[test]
fn candidate6_detail_scrolling_uses_visual_rows_and_preserves_choice() {
    let mut q = question(false);
    q.text = (0..40)
        .map(|i| format!("Row {i}: long details that wrap across a narrow terminal.\n"))
        .collect();
    let mut dialog = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(q),
        tx: None,
        result: None,
    };
    let mut terminal = Terminal::new(TestBackend::new(55, 13)).unwrap();
    let colors = ThemeColors::default();
    draw(&mut terminal, &mut dialog, &colors);
    let before = screen(&terminal);
    dialog.handle_event(&key(KeyCode::PageDown));
    draw(&mut terminal, &mut dialog, &colors);
    assert_ne!(before, screen(&terminal));
    assert_eq!(dialog.draw_state.cursor_pos, 0);
    let details = dialog.draw_state.geometry.lock().unwrap().details;
    let offset = dialog.draw_state.detail_scroll;
    dialog.handle_event(&Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: details.x,
        row: details.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(dialog.draw_state.detail_scroll > offset);
    assert_eq!(dialog.draw_state.cursor_pos, 0);
    for _ in 0..100 {
        dialog.handle_event(&key(KeyCode::PageDown));
    }
    let maximum = dialog.draw_state.geometry.lock().unwrap().detail_max;
    assert_eq!(
        dialog.draw_state.detail_scroll, maximum,
        "scroll stops at the last visual row"
    );
    dialog.handle_event(&key(KeyCode::PageUp));
    assert!(
        dialog.draw_state.detail_scroll < maximum,
        "one upward step works immediately at the end"
    );
}

#[test]
fn candidate6_prepared_cache_direct_resize_theme_revision_and_reclamation() {
    let mut colors = ThemeColors::default();
    let lines = vec![RenderLine::AssistantText(
        "A long line of text that must wrap differently when the terminal becomes narrow.".into(),
    )];
    let entries = build_timeline_entries(&lines);
    let mut cache = PreparedCache::new();
    let expanded = Default::default();
    let wide = prepare_timeline_entries(
        &entries, 80, true, &expanded, &colors, false, &mut cache, false,
    );
    let narrow = prepare_timeline_entries(
        &entries, 24, true, &expanded, &colors, false, &mut cache, false,
    );
    assert!(
        narrow[0].rows > wide[0].rows,
        "direct preparation owns width validity"
    );
    assert_eq!(cache.cache.len(), 1);

    let mut engine = TimelineLayoutEngine::new();
    engine.set_active_stream(Some("old stream"));
    let before = engine
        .layout_items(&lines, 80, true, &expanded, &colors, false, 1)
        .to_vec();
    let mut terminal = Terminal::new(TestBackend::new(84, 30)).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render_timeline_viewport(f, area, &before, 0, &colors, None, None);
        })
        .unwrap();
    assert!(screen(&terminal).contains("old stream"));
    colors.register_token("accent.primary", opaline::OpalineColor::new(17, 34, 51));
    let recolored = engine
        .layout_items(&lines, 80, true, &expanded, &colors, false, 1)
        .to_vec();
    assert_ne!(
        recolored.last().unwrap().lines,
        before.last().unwrap().lines,
        "unchanged live text must be recolored after a same-name theme update"
    );
    engine.set_active_stream(Some("new stream"));
    let after = engine
        .layout_items(&lines, 80, true, &expanded, &colors, false, 1)
        .to_vec();
    terminal
        .draw(|f| {
            let area = f.area();
            render_timeline_viewport(f, area, &after, 0, &colors, None, None);
        })
        .unwrap();
    assert!(screen(&terminal).contains("new stream"));
    assert!(!screen(&terminal).contains("old stream"));
    assert!(
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.symbol() == "C" && cell.fg == colors.c_primary())
    );
    let themed = prepare_timeline_entries(
        &entries, 24, true, &expanded, &colors, false, &mut cache, false,
    );
    assert_ne!(
        themed[0].lines, narrow[0].lines,
        "same-name theme update must invalidate prepared spans"
    );
    for revision in 0..40 {
        let live = vec![RenderLine::SystemMsg(format!("revision {revision}"))];
        engine.layout_items(&live, 80, false, &expanded, &colors, false, revision + 2);
        assert_eq!(
            engine.item_cache.cache.len(),
            1,
            "old content revisions must be reclaimed"
        );
    }
    engine.set_active_stream(None);
    engine.layout_items(&[], 80, false, &expanded, &colors, false, 100);
    assert!(
        engine.item_cache.cache.is_empty(),
        "server-replaced history frees obsolete artifacts"
    );
}

#[test]
fn candidate6_dialog_json_formatting_and_same_name_theme_cache() {
    let mut q = question(false);
    q.text = r#"{"command":"git status","nested":{"value":7},"enabled":true}"#.into();
    let mut dialog = ActiveQuestionState {
        approval_id: None,
        draw_state: ActiveQuestionDrawState::new(q),
        tx: None,
        result: None,
    };
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut colors = ThemeColors::default();
    draw(&mut terminal, &mut dialog, &colors);
    let content = screen(&terminal);
    let command_row = content
        .lines()
        .position(|line| line.contains("\"command\""))
        .unwrap();
    let nested_row = content
        .lines()
        .position(|line| line.contains("\"nested\""))
        .unwrap();
    assert_ne!(
        command_row, nested_row,
        "compact JSON must be presented as structured, formatted details"
    );

    dialog.draw_state.question.text = "Plain details retain their active theme.".into();
    draw(&mut terminal, &mut dialog, &colors);
    colors.register_token("text.primary", opaline::OpalineColor::new(31, 47, 63));
    draw(&mut terminal, &mut dialog, &colors);
    let details = dialog.draw_state.geometry.lock().unwrap().details;
    let x = (details.x..details.right())
        .find(|x| terminal.backend().buffer()[(*x, details.y)].symbol() == "P")
        .unwrap();
    assert_eq!(
        terminal.backend().buffer()[(x, details.y)].fg,
        colors.c_text_primary(),
        "cached details must not retain the old palette when the theme name is unchanged"
    );
}

#[test]
fn candidate6_cancelled_waiter_releases_dialog_ownership() {
    let (tx, receiver) = tokio::sync::oneshot::channel();
    let question = ActiveQuestionState::new(question(false), tx);
    assert!(!question.is_dismissed());
    drop(receiver);
    assert!(question.is_dismissed());
    let (tx, receiver) = tokio::sync::oneshot::channel();
    let permission = PermissionOverlay::new("file.write", "example.txt", tx);
    drop(receiver);
    assert!(
        permission.is_dismissed(),
        "a cancelled tool cannot retain input ownership"
    );
}

#[test]
fn candidate6_sparse_theme_and_tiny_width_keep_choices_readable() {
    let colors = ThemeColors::builder("sparse-dialog")
        .token("bg.base", opaline::OpalineColor::new(18, 24, 32))
        .token("text.primary", opaline::OpalineColor::new(220, 224, 228))
        .build();
    assert_eq!(colors.c_bg_surface0(), colors.c_bg_base());
    assert_eq!(colors.c_bg_surface2(), colors.c_bg_base());
    assert_eq!(colors.c_warning(), colors.c_text_primary());
    assert_ne!(colors.c_text_primary(), colors.c_bg_surface2());
    let q = Question {
        header: "Approve".into(),
        text: "```bash\necho ok\n```".into(),
        options: vec![QuestionOption {
            label: "Go".into(),
            description: String::new(),
        }],
        multi_select: false,
        allow_other: false,
        progress: None,
    };
    let dialog = ActiveQuestionDrawState::new(q);
    let mut terminal = Terminal::new(TestBackend::new(32, 12)).unwrap();
    // Keep a valid backend while exercising zero-sized and tiny layout areas.
    // These are the actual compact renderer and Markdown parser entry points.
    for width in 0..=16 {
        terminal
            .draw(|frame| {
                super::layout::question::render_question_inline(
                    frame,
                    &dialog,
                    Rect::new(0, 0, width, 12),
                    &colors,
                )
            })
            .unwrap();
        let geometry = dialog.geometry.lock().unwrap();
        assert_eq!(geometry.panel.width, width);
        for (_, choice) in &geometry.choices {
            assert!(choice.right() <= width && choice.bottom() <= 12);
        }
        if width >= 4 {
            let buffer = terminal.backend().buffer();
            let cell = buffer
                .content()
                .iter()
                .find(|cell| cell.symbol() == "G")
                .expect("selected Go label remains visible");
            assert_eq!(cell.fg, colors.c_text_primary());
            assert_eq!(cell.bg, colors.c_bg_base());
        }
    }
    let empty = ThemeColors::builder("terminal-defaults").build();
    assert_eq!(empty.c_text_primary(), ratatui::style::Color::Reset);
    assert_eq!(empty.c_bg_surface0(), ratatui::style::Color::Reset);
    for text in ["```text\nhello\n```", "```text\nhello"] {
        let lines = crate::markdown::parse_markdown_lines_with_theme(text, &empty, 6, true);
        let span = lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content.contains("hello"))
            .unwrap();
        assert_eq!(span.style.fg, Some(ratatui::style::Color::Reset));
    }
}

#[cfg(feature = "syntax-highlighting")]
#[test]
fn candidate6_code_details_use_active_theme_for_closed_and_streaming_fences() {
    let mut colors = ThemeColors::default();
    colors.register_token("code.string", opaline::OpalineColor::new(61, 127, 191));
    for text in [
        "```json\n{\"value\":\"plain\"}\n```",
        "```json\n{\"value\":\"plain\"}",
    ] {
        let lines = crate::markdown::parse_markdown_lines_with_theme(text, &colors, 80, true);
        let span = lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content.contains("plain"))
            .unwrap();
        assert_eq!(
            span.style.fg,
            Some(colors.c_syntax_string()),
            "code must use the active palette, including unfinished fences"
        );
    }
}
