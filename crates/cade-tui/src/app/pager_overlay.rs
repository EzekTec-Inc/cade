use std::any::Any;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::colors::{ThemeColors, ThemeColorsExt};
use crate::overlay_component::{OverlayComponent, OverlayInputResult};

/// Dedicated modal overlay for paging and searching long tool outputs and compiler/process logs.
pub struct PagerOverlay {
    pub title: String,
    pub lines: Vec<String>,
    pub scroll: usize,
    pub dismissed: bool,
    pub search_query: String,
    pub in_search_mode: bool,
    pub search_matches: Vec<usize>,
    pub current_match_idx: usize,
    pub copy_requested: Option<String>,
}

impl PagerOverlay {
    pub fn new(title: String, content: String) -> Self {
        let lines: Vec<String> = if content.is_empty() {
            vec!["(no output recorded)".to_string()]
        } else {
            content.lines().map(String::from).collect()
        };
        Self {
            title,
            lines,
            scroll: 0,
            dismissed: false,
            search_query: String::new(),
            in_search_mode: false,
            search_matches: Vec::new(),
            current_match_idx: 0,
            copy_requested: None,
        }
    }

    fn update_search_matches(&mut self) {
        self.search_matches.clear();
        self.current_match_idx = 0;
        if self.search_query.trim().is_empty() {
            return;
        }
        let q = self.search_query.to_lowercase();
        for (idx, line) in self.lines.iter().enumerate() {
            if line.to_lowercase().contains(&q) {
                self.search_matches.push(idx);
            }
        }
    }

    fn jump_to_current_match(&mut self, viewport_h: usize) {
        if let Some(&target_line) = self.search_matches.get(self.current_match_idx) {
            let half = viewport_h / 2;
            self.scroll = target_line.saturating_sub(half);
        }
    }
}

impl OverlayComponent for PagerOverlay {
    fn id(&self) -> &'static str {
        "pager_overlay"
    }

    fn render_overlay(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let v_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(5),
                Constraint::Percentage(90),
                Constraint::Percentage(5),
            ])
            .split(area);
        let h_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(5),
                Constraint::Percentage(90),
                Constraint::Percentage(5),
            ])
            .split(v_split[1]);

        let modal_area = h_split[1];
        frame.render_widget(Clear, modal_area);

        let total_lines = self.lines.len();
        let title_span = format!(
            " Tool Output: {} ({} lines) [Esc/q: close | /: search | y: copy] ",
            self.title, total_lines
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(
                Style::default()
                    .fg(colors.c_primary())
                    .add_modifier(Modifier::BOLD),
            )
            .title(Span::styled(
                title_span,
                Style::default()
                    .fg(colors.c_primary())
                    .add_modifier(Modifier::BOLD),
            ));

        let inner_area = block.inner(modal_area);
        frame.render_widget(block, modal_area);

        if inner_area.height == 0 || inner_area.width == 0 {
            return;
        }

        let content_h = inner_area.height.saturating_sub(1) as usize;
        let max_scroll = total_lines.saturating_sub(content_h);
        self.scroll = self.scroll.min(max_scroll);

        let gutter_w = 6;
        let text_w = (inner_area.width as usize).saturating_sub(gutter_w);

        let mut rendered_lines: Vec<Line<'static>> = Vec::new();

        use ansi_to_tui::IntoText;

        for (i, raw_line) in self
            .lines
            .iter()
            .skip(self.scroll)
            .take(content_h)
            .enumerate()
        {
            let line_num = self.scroll + i + 1;
            let mut spans = Vec::new();

            spans.push(Span::styled(
                format!("{:>4} │ ", line_num),
                Style::default().fg(colors.c_text_muted()),
            ));

            let parsed_text = raw_line
                .as_str()
                .into_text()
                .unwrap_or_else(|_| ratatui::text::Text::raw(raw_line.clone()));

            for p_line in parsed_text.lines {
                for mut s in p_line.spans {
                    let len = s.content.chars().count();
                    if len > text_w {
                        let truncated = s.content.chars().take(text_w).collect::<String>();
                        s.content = std::borrow::Cow::Owned(truncated);
                        spans.push(s);
                        break;
                    } else {
                        spans.push(s);
                    }
                }
            }

            rendered_lines.push(Line::from(spans));
        }

        let content_rect = Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width,
            height: content_h as u16,
        };
        frame.render_widget(Paragraph::new(rendered_lines), content_rect);

        let footer_rect = Rect {
            x: inner_area.x,
            y: inner_area.y + content_h as u16,
            width: inner_area.width,
            height: 1,
        };

        let footer_line = if self.in_search_mode {
            Line::from(vec![
                Span::styled(
                    " Search: ",
                    Style::default()
                        .fg(colors.c_primary())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{}_", self.search_query),
                    Style::default().fg(colors.c_text_primary()),
                ),
                Span::styled(
                    " (Enter: confirm, Esc: cancel)",
                    Style::default().fg(colors.c_text_muted()),
                ),
            ])
        } else if !self.search_matches.is_empty() {
            Line::from(vec![Span::styled(
                format!(
                    " Match {}/{} for \"{}\" [n/N: next/prev] │ Line {}/{}",
                    self.current_match_idx + 1,
                    self.search_matches.len(),
                    self.search_query,
                    self.scroll + 1,
                    total_lines
                ),
                Style::default().fg(colors.c_primary()),
            )])
        } else {
            Line::from(vec![Span::styled(
                format!(
                    " Line {}/{} ({}%) · Press / to search · y to copy all",
                    self.scroll + 1,
                    total_lines,
                    ((self.scroll + 1) * 100)
                        .checked_div(total_lines)
                        .unwrap_or(100)
                ),
                Style::default().fg(colors.c_text_muted()),
            )])
        };

        frame.render_widget(Paragraph::new(footer_line), footer_rect);
    }

    fn handle_input(&mut self, key: KeyEvent) -> OverlayInputResult {
        if self.in_search_mode {
            match key.code {
                KeyCode::Esc => {
                    self.in_search_mode = false;
                    return OverlayInputResult::Consumed;
                }
                KeyCode::Enter => {
                    self.in_search_mode = false;
                    self.update_search_matches();
                    self.jump_to_current_match(25);
                    return OverlayInputResult::Consumed;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                    self.update_search_matches();
                    return OverlayInputResult::Consumed;
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                    self.update_search_matches();
                    return OverlayInputResult::Consumed;
                }
                _ => return OverlayInputResult::Consumed,
            }
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.dismissed = true;
                OverlayInputResult::Dismiss
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll = self.scroll.saturating_add(1);
                OverlayInputResult::Consumed
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
                OverlayInputResult::Consumed
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll = self.scroll.saturating_add(15);
                OverlayInputResult::Consumed
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll = self.scroll.saturating_sub(15);
                OverlayInputResult::Consumed
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(25);
                OverlayInputResult::Consumed
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(25);
                OverlayInputResult::Consumed
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.scroll = 0;
                OverlayInputResult::Consumed
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.scroll = self.lines.len();
                OverlayInputResult::Consumed
            }
            KeyCode::Char('/') => {
                self.in_search_mode = true;
                OverlayInputResult::Consumed
            }
            KeyCode::Char('n') => {
                if !self.search_matches.is_empty() {
                    self.current_match_idx =
                        (self.current_match_idx + 1) % self.search_matches.len();
                    self.jump_to_current_match(25);
                }
                OverlayInputResult::Consumed
            }
            KeyCode::Char('N') => {
                if !self.search_matches.is_empty() {
                    self.current_match_idx = if self.current_match_idx == 0 {
                        self.search_matches.len() - 1
                    } else {
                        self.current_match_idx - 1
                    };
                    self.jump_to_current_match(25);
                }
                OverlayInputResult::Consumed
            }
            KeyCode::Char('y') => {
                self.copy_requested = Some(self.lines.join("\n"));
                OverlayInputResult::Consumed
            }
            _ => OverlayInputResult::NotHandled,
        }
    }

    fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    fn take_result(&mut self) -> Option<Box<dyn Any>> {
        self.copy_requested
            .take()
            .map(|c| Box::new(c) as Box<dyn Any>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pager_overlay_initialization_and_scroll() {
        let content = "line 1\nline 2\nline 3\nline 4\nline 5";
        let mut pager = PagerOverlay::new("test_tool".to_string(), content.to_string());
        assert_eq!(pager.lines.len(), 5);
        assert_eq!(pager.scroll, 0);

        let down_key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let res = pager.handle_input(down_key);
        assert_eq!(res, OverlayInputResult::Consumed);
        assert_eq!(pager.scroll, 1);
    }

    #[test]
    fn test_pager_overlay_search() {
        let content = "alpha\nbeta\ngamma\nbeta test";
        let mut pager = PagerOverlay::new("test_tool".to_string(), content.to_string());

        pager.search_query = "beta".to_string();
        pager.update_search_matches();
        assert_eq!(pager.search_matches.len(), 2);
        assert_eq!(pager.search_matches, vec![1, 3]);
    }

    #[test]
    fn test_pager_overlay_dismiss() {
        let mut pager = PagerOverlay::new("test".to_string(), "abc".to_string());
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let res = pager.handle_input(esc);
        assert_eq!(res, OverlayInputResult::Dismiss);
        assert!(pager.is_dismissed());
    }
}
