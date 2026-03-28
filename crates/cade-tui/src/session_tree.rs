/// Session checkpoint tree browser.
///
/// A full-screen TUI that shows the agent's checkpoints as a navigable list.
/// The user can:
///   - Browse checkpoints by label and timestamp
///   - Press Enter to select one (returns the checkpoint ID for restore)
///   - Press 'n' to start a new conversation from this point
///   - Press Esc / 'q' to cancel
use crate::{Result, colors::ThemeColors, overlay};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};
use serde_json::Value;

// region:    --- Types

/// What the user chose to do with a checkpoint.
#[derive(Debug, Clone)]
pub enum TreeAction {
    /// Restore working tree to this checkpoint.
    Restore { checkpoint_id: String },
    /// Cancel — do nothing.
    Cancel,
}

// endregion: --- Types

// region:    --- Public entry point

/// Show the session tree browser.
///
/// `checkpoints` is a list of checkpoint JSON objects from the server.
/// Returns the user's chosen action.
pub fn show_session_tree(
    terminal: &mut DefaultTerminal,
    checkpoints: &[Value],
    colors: &ThemeColors,
) -> Result<TreeAction> {
    if checkpoints.is_empty() {
        return Ok(TreeAction::Cancel);
    }

    let mut list_state = ListState::default();
    list_state.select(Some(0));

    loop {
        terminal.draw(|f| draw_tree(f, checkpoints, &mut list_state, colors))?;

        if let Ok(evt) = event::read() {
            match evt {
                Event::Key(k) if k.kind == KeyEventKind::Press => match (k.code, k.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => {
                        return Ok(TreeAction::Cancel);
                    }
                    (KeyCode::Enter, _) => {
                        if let Some(idx) = list_state.selected()
                            && let Some(cp) = checkpoints.get(idx)
                        {
                            let id = cp["id"].as_str().unwrap_or("").to_string();
                            if !id.is_empty() {
                                return Ok(TreeAction::Restore { checkpoint_id: id });
                            }
                        }
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        let n = checkpoints.len();
                        let next = list_state
                            .selected()
                            .map(|i| if i == 0 { n - 1 } else { i - 1 })
                            .unwrap_or(0);
                        list_state.select(Some(next));
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        let n = checkpoints.len();
                        let next = list_state.selected().map(|i| (i + 1) % n).unwrap_or(0);
                        list_state.select(Some(next));
                    }
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
}

// endregion: --- Public entry point

// region:    --- Drawing

fn draw_tree(
    frame: &mut Frame,
    checkpoints: &[Value],
    list_state: &mut ListState,
    colors: &ThemeColors,
) {
    let area = frame.area();
    let inner = overlay::render_overlay_shell(frame, area, "Session Checkpoints", colors);

    let [header_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // -- Header
    let header = Paragraph::new(" Browse checkpoints and restore a saved working state")
        .style(overlay::overlay_muted_style(colors));
    frame.render_widget(header, header_area);

    // -- Checkpoint list
    let items: Vec<ListItem<'static>> = checkpoints
        .iter()
        .enumerate()
        .map(|(i, cp)| {
            let id = cp["id"].as_str().unwrap_or("?");
            let label = cp["label"].as_str().unwrap_or("(unlabelled)");
            let ts = cp["created_at"].as_i64().unwrap_or(0);
            let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0)
                .map(|d| d.format("%m-%d %H:%M").to_string())
                .unwrap_or_default();
            let stash = cp["git_stash_ref"].as_str().filter(|s| !s.is_empty());
            let commit = cp["git_commit_hash"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|h| &h[..8.min(h.len())]);

            // Indicator: 🔀 if has git stash, 📍 otherwise
            let icon = if stash.is_some() { "🔀" } else { "📍" };
            let git_str = match (stash, commit) {
                (Some(s), Some(h)) => format!("  {s} @ {h}"),
                (None, Some(h)) => format!("  @ {h}"),
                (Some(s), None) => format!("  {s}"),
                _ => String::new(),
            };

            let line = Line::from(vec![
                Span::styled(format!("  {icon} "), Style::default()),
                Span::styled(
                    label.to_string(),
                    if list_state.selected() == Some(i) {
                        Style::default()
                            .fg(colors.overlay_selected_fg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(
                    format!("  {dt}"),
                    overlay::overlay_muted_style(colors),
                ),
                Span::styled(git_str, Style::default().fg(colors.dim)),
                Span::styled(
                    format!("  ({})", &id[..8.min(id.len())]),
                    Style::default().fg(colors.overlay_border),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().style(Style::default().bg(colors.overlay_bg)))
        .highlight_style(overlay::overlay_selected_style(colors))
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(list, list_area, list_state);

    // -- Footer
    overlay::render_overlay_hint(
        frame,
        footer_area,
        "↑↓ / jk navigate · Enter restore · Esc / q cancel",
        colors,
    );
}

// endregion: --- Drawing
