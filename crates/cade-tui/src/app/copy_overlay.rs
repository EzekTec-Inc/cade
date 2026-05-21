use std::any::Any;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
};

use crate::colors::{ThemeColors, ThemeColorsExt};
use crate::overlay_component::{OverlayComponent, OverlayInputResult};

pub struct CopyOverlay {
    items: Vec<(String, String)>, // (Label/Preview, Full Content)
    state: ListState,
    dismissed: bool,
    copied_content: Option<String>,
}

impl CopyOverlay {
    pub fn new(lines: &[crate::app::RenderLine]) -> Self {
        let mut items = Vec::new();
        for line in lines {
            match line {
                crate::app::RenderLine::UserMessage(text) => {
                    items.push(("User Message".to_string(), text.clone()));
                }
                crate::app::RenderLine::AssistantText(text) => {
                    items.push(("Assistant Response".to_string(), text.clone()));
                }
                crate::app::RenderLine::ToolResult { content, .. } => {
                    items.push(("Tool Result".to_string(), content.clone()));
                }
                crate::app::RenderLine::Reasoning { content, .. } => {
                    items.push(("Reasoning Block".to_string(), content.clone()));
                }
                _ => {}
            }
        }

        let mut state = ListState::default();
        if !items.is_empty() {
            state.select(Some(items.len() - 1));
        }

        Self {
            items,
            state,
            dismissed: false,
            copied_content: None,
        }
    }
}

pub struct CopyAction(pub String);

impl OverlayComponent for CopyOverlay {
    fn id(&self) -> &'static str {
        "copy_overlay"
    }

    fn render_overlay(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(colors.c_border_base()))
            .title(Span::styled(
                " Copy to Clipboard (Ctrl+Y/Enter to copy, Esc to close) ",
                Style::default().add_modifier(Modifier::BOLD),
            ));

        // Center the overlay (e.g. 80% width, 80% height)
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(10),
                Constraint::Percentage(80),
                Constraint::Percentage(10),
            ])
            .split(area);
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(10),
                Constraint::Percentage(80),
                Constraint::Percentage(10),
            ])
            .split(vertical[1]);

        let inner_area = horizontal[1];

        // Clear background
        frame.render_widget(Clear, inner_area);

        if self.items.is_empty() {
            let p = ratatui::widgets::Paragraph::new("No copyable content available.")
                .block(block)
                .style(Style::default().fg(colors.c_text_primary()));
            frame.render_widget(p, inner_area);
            return;
        }

        let list_items: Vec<ListItem> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, (label, content))| {
                let preview = content.replace('\n', " ");
                let preview = if preview.len() > 100 {
                    format!("{}...", &preview[..100])
                } else {
                    preview
                };

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
                        format!("[{}] ", label),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(preview),
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
        if self.items.is_empty() {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter)
                || (matches!(key.code, KeyCode::Char('y'))
                    && key.modifiers.contains(KeyModifiers::CONTROL))
            {
                self.dismissed = true;
                return OverlayInputResult::Dismiss;
            }
            return OverlayInputResult::Consumed;
        }

        let selected = self.state.selected().unwrap_or(0);

        match key.code {
            KeyCode::Esc => {
                self.dismissed = true;
                return OverlayInputResult::Dismiss;
            }
            KeyCode::Enter => {
                self.copied_content = Some(self.items[selected].1.clone());
                self.dismissed = true;
                return OverlayInputResult::Dismiss;
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.copied_content = Some(self.items[selected].1.clone());
                self.dismissed = true;
                return OverlayInputResult::Dismiss;
            }
            KeyCode::Up => {
                if selected > 0 {
                    self.state.select(Some(selected - 1));
                }
            }
            KeyCode::Down => {
                if selected < self.items.len() - 1 {
                    self.state.select(Some(selected + 1));
                }
            }
            KeyCode::PageUp => {
                self.state.select(Some(selected.saturating_sub(10)));
            }
            KeyCode::PageDown => {
                let next = (selected + 10).min(self.items.len() - 1);
                self.state.select(Some(next));
            }
            _ => {}
        }
        OverlayInputResult::Consumed
    }

    fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    fn take_result(&mut self) -> Option<Box<dyn Any>> {
        self.copied_content
            .take()
            .map(|s| Box::new(CopyAction(s)) as Box<dyn Any>)
    }
}
