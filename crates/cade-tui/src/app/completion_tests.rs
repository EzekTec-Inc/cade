//! Completion regressions at the production dispatch/editor boundary.

use super::*;
use crate::autocomplete::{AutocompleteOverlay, SlashCommandDef, SlashCommandProvider};
use crate::editor::Editor;
use crate::editor_component::EditorComponent;
use crate::overlay_component::OverlayComponent;

fn commands() -> SlashCommandProvider {
    SlashCommandProvider::new(vec![
        SlashCommandDef {
            name: "help".into(),
            description: "Help".into(),
        },
        SlashCommandDef {
            name: "clear".into(),
            description: "Clear".into(),
        },
    ])
}

fn completion_stack() -> Vec<Box<dyn OverlayComponent>> {
    vec![Box::new(AutocompleteOverlay::new(
        commands().completions("/", 1),
        0,
        1,
    ))]
}

#[test]
fn completion_typing_and_paste_reach_editor_and_filter_suggestions() {
    for event in [
        Event::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
        Event::Paste("h".into()),
    ] {
        let mut editor = Editor::new();
        editor.set_text("/".into());
        editor.set_cursor_pos(1);
        let mut overlays = completion_stack();
        let dispatch = dispatch_overlay_stack(&mut overlays, &event);
        if !dispatch.owned {
            match event {
                Event::Key(key) => {
                    EditorComponent::handle_input(&mut editor, key, 80);
                }
                Event::Paste(text) => editor.handle_paste(&text),
                _ => unreachable!(),
            }
        }
        assert_eq!(editor.text(), "/h");
        let overlay = overlays[0]
            .as_any_mut()
            .unwrap()
            .downcast_mut::<AutocompleteOverlay>()
            .unwrap();
        overlay.update_suggestions(
            &editor.text(),
            editor.cursor_pos(),
            &commands(),
            &Default::default(),
            &Default::default(),
        );
        assert_eq!(overlay.suggestions.len(), 1);
        assert_eq!(overlay.suggestions[0].text, "/help");
    }
}

#[test]
fn completion_preserves_editing_shortcuts_and_mouse_input() {
    let mut events = vec![
        Event::Paste("help".into()),
        Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::ScrollUp,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        }),
    ];
    for code in [
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
    ] {
        events.push(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }
    for code in [KeyCode::Char('c'), KeyCode::Char('v'), KeyCode::Enter] {
        events.push(Event::Key(KeyEvent::new(code, KeyModifiers::CONTROL)));
    }
    events.push(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::SHIFT,
    )));
    for event in events {
        assert!(
            !dispatch_overlay_stack(&mut completion_stack(), &event).owned,
            "captured {event:?}"
        );
    }
}

#[test]
fn completion_navigation_acceptance_and_escape_remain_owned() {
    let mut overlays = completion_stack();
    for code in [KeyCode::Down, KeyCode::Up] {
        assert!(
            dispatch_overlay_stack(
                &mut overlays,
                &Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
            )
            .owned
        );
    }
    let selected = dispatch_overlay_stack(
        &mut overlays,
        &Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    assert!(selected.owned);
    assert_eq!(
        selected
            .action
            .unwrap()
            .downcast::<crate::autocomplete::AutocompleteAction>()
            .unwrap()
            .text,
        "/help"
    );
    assert!(overlays.is_empty());
    let mut overlays = completion_stack();
    assert!(
        dispatch_overlay_stack(
            &mut overlays,
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        )
        .owned
    );
    assert!(
        !dispatch_overlay_stack(
            &mut overlays,
            &Event::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        )
        .owned
    );
}

#[test]
fn completion_under_permission_dialog_cannot_receive_input() {
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let mut overlays = completion_stack();
    overlays.push(Box::new(
        crate::app::permission_overlay::PermissionOverlay::new("bash.exec", "git status", tx),
    ));
    for event in [
        Event::Paste("help".into()),
        Event::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
    ] {
        let dispatch = dispatch_overlay_stack(&mut overlays, &event);
        assert!(dispatch.owned);
        assert!(dispatch.action.is_none());
        assert_eq!(overlays.len(), 2);
    }
    dispatch_overlay_stack(
        &mut overlays,
        &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert!(
        !dispatch_overlay_stack(
            &mut overlays,
            &Event::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE))
        )
        .owned
    );
}

#[test]
#[ignore = "requires tty"]
fn completion_idle_events_refresh_after_typing_paste_and_cursor_motion() {
    let mut app = TuiApp::new(
        cade_core::permissions::PermissionMode::Default,
        "test-agent".into(),
        "test-model".into(),
        None,
    );
    app.lua_engine = None;
    app.slash_ac = commands();
    let mut history = vec![];
    let mut index = None;
    let mut send = |app: &mut TuiApp, event| {
        app.last_keypress = std::time::Instant::now() - std::time::Duration::from_millis(120);
        assert!(
            app.handle_idle_event(event, &mut history, &mut index)
                .unwrap()
                .is_none()
        );
    };
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
    );
    assert_eq!(app.overlays.len(), 1);
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
    );
    assert_eq!(app.editor.text(), "/h");
    send(&mut app, Event::Paste("e".into()));
    assert_eq!(app.editor.text(), "/he");
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    assert_eq!(app.editor.text(), "/help ");
    // Editing a command before existing arguments must retain the suffix exactly once.
    app.editor.set_text("/he please".into());
    app.editor.set_cursor_pos(3);
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    assert_eq!(app.overlays.len(), 1);
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    assert_eq!(app.editor.text(), "/help please");
    // Completion in the middle of an existing token replaces its suffix too.
    app.editor.set_cursor_pos(2);
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
    );
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    assert_eq!(app.editor.text(), "/help please");
    assert_eq!(app.editor.cursor_pos(), 6);
    // Active-turn edits also refresh before accepting a suggestion.
    app.editor.set_text("/".into());
    app.editor.set_cursor_pos(1);
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    app.editor.insert_str("cl");
    app.refresh_autocomplete();
    assert!(
        app.dispatch_overlay_event(&Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)))
            .unwrap()
            .0
    );
    assert_eq!(app.editor.text(), "/clear ");
    // Paste yielding no matches must retire the popup immediately.
    app.editor.set_text("/".into());
    app.editor.set_cursor_pos(1);
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    send(&mut app, Event::Paste("no-such-command".into()));
    assert_eq!(app.editor.text(), "/no-such-command");
    assert!(app.overlays.is_empty());
    // Leaving the command token retires completion; modified Enter inserts a newline.
    app.editor.set_text("/he".into());
    app.editor.set_cursor_pos(3);
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
    );
    send(
        &mut app,
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
    );
    assert!(app.editor.text().contains('\n'));
    assert!(app.overlays.is_empty());
}
