use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use crossterm::event::{KeyCode, KeyEvent};

use crate::slots::SlotComponent;
use crate::ThemeColors;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LuaWidget {
    Text {
        content: String,
        color: Option<String>,
    },
    Button {
        id: String,
        label: String,
    },
    Toggle {
        id: String,
        label: String,
        state: bool,
    },
    Layout {
        direction: Option<String>, // "horizontal" or "vertical"
        children: Vec<LuaWidget>,
    },
    Clock {
        format: Option<String>,
        color: Option<String>,
    },
    Gauge {
        label: Option<String>,
        ratio: f64,
        color: Option<String>,
    },
    List {
        id: Option<String>,
        items: Vec<String>,
        selected: Option<usize>,
    },
    Popup {
        title: Option<String>,
        content: Box<LuaWidget>,
    },
    Paragraph {
        content: String,
        wrap: bool,
    },
}

pub struct LuaUiSlot {
    pub is_header: bool,
    pub root: Option<Vec<LuaWidget>>,
    pub focused_idx: usize,
    pub event_queue: Arc<Mutex<VecDeque<(String, serde_json::Value)>>>,
    pub has_clock: bool,
}

impl LuaUiSlot {
    pub fn new(is_header: bool, event_queue: Arc<Mutex<VecDeque<(String, serde_json::Value)>>>) -> Self {
        Self {
            is_header,
            root: None,
            focused_idx: 0,
            event_queue,
            has_clock: false,
        }
    }

    pub fn update(&mut self, root: Option<Vec<LuaWidget>>) {
        self.root = root;
        self.has_clock = self.check_clock();
        // Clamp focused_idx
        let interactives = self.interactive_ids();
        if interactives.is_empty() {
            self.focused_idx = 0;
        } else if self.focused_idx >= interactives.len() {
            self.focused_idx = interactives.len() - 1;
        }
    }

    fn interactive_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        fn walk(widget: &LuaWidget, ids: &mut Vec<String>) {
            match widget {
                LuaWidget::Button { id, .. } | LuaWidget::Toggle { id, .. } | LuaWidget::List { id: Some(id), .. } => {
                    ids.push(id.clone());
                }
                LuaWidget::Layout { children, .. } => {
                    for child in children {
                        walk(child, ids);
                    }
                }
                LuaWidget::Popup { content, .. } => {
                    walk(content, ids);
                }
                _ => {}
            }
        }
        if let Some(children) = &self.root {
            for child in children {
                walk(child, &mut ids);
            }
        }
        ids
    }

    fn check_clock(&self) -> bool {
        let mut has_clock = false;
        fn walk(widget: &LuaWidget, has: &mut bool) {
            match widget {
                LuaWidget::Clock { .. } => *has = true,
                LuaWidget::Layout { children, .. } => {
                    for child in children {
                        walk(child, has);
                    }
                }
                LuaWidget::Popup { content, .. } => {
                    walk(content, has);
                }
                _ => {}
            }
        }
        if let Some(children) = &self.root {
            for child in children {
                walk(child, &mut has_clock);
            }
        }
        has_clock
    }
}

impl SlotComponent for LuaUiSlot {
    fn render(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let inner_area = if self.is_header {
            area // Header has no borders for now, to save space
        } else {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" Plugins ");
            let i_area = block.inner(area);
            frame.render_widget(block, area);
            i_area
        };

        if let Some(children) = &self.root {
            let interactives = self.interactive_ids();
            let focused_id = interactives.get(self.focused_idx).cloned();

            let constraints: Vec<Constraint> = if self.is_header {
                std::iter::repeat(Constraint::Percentage(100 / children.len() as u16)).take(children.len()).collect()
            } else {
                std::iter::repeat(Constraint::Length(1)).take(children.len()).collect()
            };
            let layout = Layout::default()
                .direction(if self.is_header { Direction::Horizontal } else { Direction::Vertical })
                .constraints(constraints)
                .split(inner_area);

            for (i, child) in children.iter().enumerate() {
                if let Some(child_area) = layout.get(i) {
                    render_widget(child, frame, *child_area, colors, focused_id.as_deref());
                }
            }
        } else {
            let p = Paragraph::new(Line::from(vec![Span::styled(
                "No plugins active",
                Style::default().fg(Color::DarkGray),
            )]));
            frame.render_widget(p, inner_area);
        }
    }

    fn handle_input(&mut self, key: KeyEvent) -> bool {
        let interactives = self.interactive_ids();
        if interactives.is_empty() {
            return false;
        }

        match key.code {
            KeyCode::Up => {
                if self.focused_idx > 0 {
                    self.focused_idx -= 1;
                } else {
                    self.focused_idx = interactives.len() - 1;
                }
                true
            }
            KeyCode::Down => {
                if self.focused_idx + 1 < interactives.len() {
                    self.focused_idx += 1;
                } else {
                    self.focused_idx = 0;
                }
                true
            }
            KeyCode::Enter => {
                if let Some(id) = interactives.get(self.focused_idx) {
                    self.event_queue.lock().unwrap().push_back((id.clone(), serde_json::Value::Null));
                    true
                } else {
                    false
                }
            }
            KeyCode::Char(c) => {
                if let Some(id) = interactives.get(self.focused_idx) {
                    self.event_queue.lock().unwrap().push_back((
                        id.clone(),
                        serde_json::json!({ "char": c.to_string() }),
                    ));
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn preferred_height(&self) -> u16 {
        if self.is_header { 1 } else { 0 }
    }

    fn requires_tick(&self) -> bool {
        self.has_clock
    }
}

fn render_widget(widget: &LuaWidget, frame: &mut Frame, area: Rect, _colors: &ThemeColors, focused_id: Option<&str>) {
    match widget {
        LuaWidget::Text { content, color: _ } => {
            let p = Paragraph::new(content.as_str());
            frame.render_widget(p, area);
        }
        LuaWidget::Button { id, label } => {
            let is_focused = focused_id == Some(id);
            let style = if is_focused {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            let p = Paragraph::new(format!("[ {} ]", label)).style(style);
            frame.render_widget(p, area);
        }
        LuaWidget::Toggle { id, label, state } => {
            let is_focused = focused_id == Some(id);
            let style = if is_focused {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            let p = Paragraph::new(format!("[{}] {}", if *state { "X" } else { " " }, label)).style(style);
            frame.render_widget(p, area);
        }
        LuaWidget::Layout { direction: _, children: _ } => {
            // Placeholder: layouts are more complex. For now, we only render flat children via the top-level loop.
            // If we want nested layouts, we can implement it recursively.
        }
        LuaWidget::Clock { format, color: _ } => {
            let fmt_str = format.as_deref().unwrap_or("%H:%M:%S");
            let time_str = chrono::Local::now().format(fmt_str).to_string();
            let p = Paragraph::new(time_str).alignment(ratatui::layout::Alignment::Right);
            frame.render_widget(p, area);
        }
        LuaWidget::Gauge { label, ratio, color: _ } => {
            let mut gauge = ratatui::widgets::Gauge::default()
                .block(ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL))
                .gauge_style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan).bg(ratatui::style::Color::DarkGray))
                .ratio((*ratio).clamp(0.0, 1.0));
            if let Some(l) = label {
                gauge = gauge.label(l.as_str());
            }
            frame.render_widget(gauge, area);
        }
        LuaWidget::List { id, items, selected } => {
            let is_focused = id.as_deref() == focused_id;
            let mut list_items = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let mut style = ratatui::style::Style::default();
                if Some(i) == *selected {
                    style = style.fg(ratatui::style::Color::Black).bg(ratatui::style::Color::Cyan);
                }
                list_items.push(ratatui::widgets::ListItem::new(item.as_str()).style(style));
            }
            let mut block = ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL);
            if is_focused {
                block = block.border_style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan));
            }
            let list = ratatui::widgets::List::new(list_items).block(block);
            frame.render_widget(list, area);
        }
        LuaWidget::Paragraph { content, wrap } => {
            let mut p = ratatui::widgets::Paragraph::new(content.as_str());
            if *wrap {
                p = p.wrap(ratatui::widgets::Wrap { trim: true });
            }
            frame.render_widget(p, area);
        }
        LuaWidget::Popup { title, content } => {
            let block = ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title(title.as_deref().unwrap_or(""))
                .border_style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan));
            frame.render_widget(ratatui::widgets::Clear, area); // Clear background
            let inner_area = block.inner(area);
            frame.render_widget(block, area);
            render_widget(content, frame, inner_area, _colors, focused_id);
        }
    }
}
