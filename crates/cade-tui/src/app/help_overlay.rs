use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
};

use crate::colors::{ThemeColors, ThemeColorsExt};
use crate::overlay_component::{OverlayComponent, OverlayInputResult};

pub struct HelpOverlay {
    items: Vec<(&'static str, &'static str)>, // (Shortcut, Description)
    state: ListState,
    dismissed: bool,
}

impl Default for HelpOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl HelpOverlay {
    pub fn new() -> Self {
        let items = vec![
            ("Enter", "Send active prompt or run selected slash command"),
            ("Tab", "Trigger autocomplete list / cycle suggestions"),
            (
                "Esc",
                "Close floating help overlay or active autocomplete menu",
            ),
            (
                "Ctrl+O",
                "Toggle timeline expansion (expand/compress long tool outputs)",
            ),
            ("Ctrl+F", "Cycle keyboard focus between prompt and active UI slots (Sidebar/Header/Footer)"),
            (
                "Ctrl+Y",
                "Trigger timeline Copy Overlay to select and copy outputs",
            ),
            (
                "@",
                "Open fuzzy File Picker to search and insert project files",
            ),
            ("Up / Down", "Navigate autocomplete or file list selections"),
            ("Ctrl+? / ?", "Display this keyboard shortcut help menu"),
        ];

        let mut state = ListState::default();
        if !items.is_empty() {
            state.select(Some(0));
        }

        Self {
            items,
            state,
            dismissed: false,
        }
    }
}

impl OverlayComponent for HelpOverlay {
    fn id(&self) -> &'static str {
        "help_overlay"
    }

    fn render_overlay(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors.c_border_base()))
            .title(Span::styled(
                " Keyboard Shortcuts & Command Help (Esc/Enter to close) ",
                Style::default().add_modifier(Modifier::BOLD),
            ));

        // Center the overlay (e.g. 70% width, 60% height)
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(20),
                Constraint::Percentage(60),
                Constraint::Percentage(20),
            ])
            .split(area);
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(15),
                Constraint::Percentage(70),
                Constraint::Percentage(15),
            ])
            .split(vertical[1]);

        let inner_area = horizontal[1];

        // Clear background
        frame.render_widget(Clear, inner_area);

        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, (shortcut, desc))| {
                let is_selected = self.state.selected() == Some(i);
                let style = if is_selected {
                    Style::default()
                        .fg(colors.c_bg_base())
                        .bg(colors.c_border_accent())
                } else {
                    Style::default().fg(colors.c_text_primary())
                };

                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:15}", shortcut),
                        Style::default()
                            .fg(if is_selected {
                                colors.c_bg_base()
                            } else {
                                colors.c_border_accent()
                            })
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(*desc),
                ]))
                .style(style)
            })
            .collect();

        let list = List::new(list_items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::BOLD));

        frame.render_stateful_widget(list, inner_area, &mut self.state);
    }

    fn handle_input(&mut self, key: KeyEvent) -> OverlayInputResult {
        let selected = self.state.selected().unwrap_or(0);

        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.dismissed = true;
                return OverlayInputResult::Dismiss;
            }
            KeyCode::Up if selected > 0 => {
                self.state.select(Some(selected - 1));
            }
            KeyCode::Down if selected < self.items.len() - 1 => {
                self.state.select(Some(selected + 1));
            }
            _ => {}
        }
        OverlayInputResult::Consumed
    }

    fn is_dismissed(&self) -> bool {
        self.dismissed
    }
}
