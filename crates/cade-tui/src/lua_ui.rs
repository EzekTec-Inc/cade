use crossterm::event::KeyEvent;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::ThemeColors;
use crate::colors::ThemeColorsExt;
use crate::slots::SlotComponent;

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
impl LuaWidget {
    pub fn height_constraint(&self) -> Constraint {
        match self {
            LuaWidget::Text { .. } => Constraint::Length(1),
            LuaWidget::Button { .. } | LuaWidget::Toggle { .. } | LuaWidget::Clock { .. } => {
                Constraint::Length(1)
            }
            LuaWidget::Gauge { .. } => Constraint::Length(3),
            LuaWidget::List { items, .. } => Constraint::Length((items.len() as u16).max(1) + 2),
            LuaWidget::Paragraph { .. } => Constraint::Min(2),
            LuaWidget::Popup { .. } => Constraint::Min(5),
            LuaWidget::Layout { .. } => Constraint::Min(1),
        }
    }
}

pub struct LuaUiSlot {
    pub is_header: bool,
    pub root: Option<Vec<LuaWidget>>,
    pub focused_idx: usize,
    pub event_queue: Arc<Mutex<VecDeque<(String, serde_json::Value)>>>,
    pub has_clock: bool,
    pub hitboxes: Vec<(String, ratatui::layout::Rect)>,
    pub is_focused: bool,
}

impl LuaUiSlot {
    pub fn new(
        is_header: bool,
        event_queue: Arc<Mutex<VecDeque<(String, serde_json::Value)>>>,
    ) -> Self {
        Self {
            is_header,
            root: None,
            focused_idx: 0,
            event_queue,
            has_clock: false,
            hitboxes: Vec::new(),
            is_focused: false,
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
                LuaWidget::Button { id, .. }
                | LuaWidget::Toggle { id, .. }
                | LuaWidget::List { id: Some(id), .. } => {
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
    fn set_focused(&mut self, focused: bool) {
        self.is_focused = focused;
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors) {
        let inner_area = if self.is_header {
            area // Header has no borders for now, to save space
        } else {
            let border_style = if self.is_focused {
                colors.border_focus()
            } else {
                colors.border_base()
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" Plugins ");
            let i_area = block.inner(area);
            frame.render_widget(block, area);
            i_area
        };

        if let Some(children) = &self.root {
            let interactives = self.interactive_ids();
            let focused_id = interactives.get(self.focused_idx).cloned();

            let constraints: Vec<Constraint> = if self.is_header {
                std::iter::repeat_n(
                    Constraint::Percentage(100 / children.len() as u16),
                    children.len(),
                )
                .collect()
            } else {
                children.iter().map(|c| c.height_constraint()).collect()
            };
            let layout = Layout::default()
                .direction(if self.is_header {
                    Direction::Horizontal
                } else {
                    Direction::Vertical
                })
                .constraints(constraints)
                .split(inner_area);

            self.hitboxes.clear();
            for (i, child) in children.iter().enumerate() {
                if let Some(child_area) = layout.get(i) {
                    render_widget(
                        child,
                        frame,
                        *child_area,
                        colors,
                        focused_id.as_deref(),
                        &mut self.hitboxes,
                    );
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
        use crossterm::event::KeyCode;

        let interactives = self.interactive_ids();
        if interactives.is_empty() {
            return false;
        }

        match key.code {
            KeyCode::Down | KeyCode::Tab => {
                // Focus next interactive element
                self.focused_idx = (self.focused_idx + 1) % interactives.len();
                true
            }
            KeyCode::Up | KeyCode::BackTab => {
                // Focus previous interactive element
                self.focused_idx = self
                    .focused_idx
                    .checked_sub(1)
                    .unwrap_or(interactives.len() - 1);
                true
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                // Activate/Toggle the currently focused element
                if let Some(id) = interactives.get(self.focused_idx) {
                    // Try to toggle if it is a toggle
                    if let Some(new_state) = self.root.as_mut().and_then(|r| toggle_widget(r, id)) {
                        let mut args = serde_json::Map::new();
                        args.insert("state".to_string(), serde_json::Value::Bool(new_state));
                        self.event_queue
                            .lock()
                            .unwrap()
                            .push_back((id.clone(), serde_json::Value::Object(args)));
                    } else {
                        // Treat as general button click / generic action trigger
                        let mut args = serde_json::Map::new();
                        args.insert(
                            "row_offset".to_string(),
                            serde_json::Value::Number(0.into()),
                        );
                        args.insert(
                            "col_offset".to_string(),
                            serde_json::Value::Number(0.into()),
                        );
                        self.event_queue
                            .lock()
                            .unwrap()
                            .push_back((id.clone(), serde_json::Value::Object(args)));
                    }
                    true
                } else {
                    false
                }
            }
            KeyCode::Right => {
                // If focused item is a list, cycle choice forward
                if let Some(id) = interactives.get(self.focused_idx) {
                    if let Some(new_idx) = self
                        .root
                        .as_mut()
                        .and_then(|r| move_list_selection(r, id, true))
                    {
                        let mut args = serde_json::Map::new();
                        args.insert(
                            "selected".to_string(),
                            serde_json::Value::Number(new_idx.into()),
                        );
                        self.event_queue
                            .lock()
                            .unwrap()
                            .push_back((id.clone(), serde_json::Value::Object(args)));
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            KeyCode::Left => {
                // If focused item is a list, cycle choice backward
                if let Some(id) = interactives.get(self.focused_idx) {
                    if let Some(new_idx) = self
                        .root
                        .as_mut()
                        .and_then(|r| move_list_selection(r, id, false))
                    {
                        let mut args = serde_json::Map::new();
                        args.insert(
                            "selected".to_string(),
                            serde_json::Value::Number(new_idx.into()),
                        );
                        self.event_queue
                            .lock()
                            .unwrap()
                            .push_back((id.clone(), serde_json::Value::Object(args)));
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) -> bool {
        use crossterm::event::{MouseButton, MouseEventKind};
        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
            for (id, rect) in &self.hitboxes {
                if mouse.column >= rect.x
                    && mouse.column < rect.x + rect.width
                    && mouse.row >= rect.y
                    && mouse.row < rect.y + rect.height
                {
                    // Focus this widget
                    let interactives = self.interactive_ids();
                    if let Some(idx) = interactives.iter().position(|i| i == id) {
                        self.focused_idx = idx;
                    }
                    // Trigger it
                    let mut args = serde_json::Map::new();
                    args.insert(
                        "row_offset".to_string(),
                        serde_json::Value::Number((mouse.row - rect.y).into()),
                    );
                    args.insert(
                        "col_offset".to_string(),
                        serde_json::Value::Number((mouse.column - rect.x).into()),
                    );
                    self.event_queue
                        .lock()
                        .unwrap()
                        .push_back((id.clone(), serde_json::Value::Object(args)));
                    return true;
                }
            }
        }
        false
    }

    fn preferred_height(&self) -> u16 {
        if self.is_header { 1 } else { 0 }
    }

    fn preferred_width(&self) -> u16 {
        if self.is_header { 0 } else { 40 }
    }

    fn requires_tick(&self) -> bool {
        self.has_clock
    }
}

fn render_widget(
    widget: &LuaWidget,
    frame: &mut Frame,
    area: Rect,
    _colors: &ThemeColors,
    focused_id: Option<&str>,
    hitboxes: &mut Vec<(String, ratatui::layout::Rect)>,
) {
    use ratatui::layout::{Alignment, Rect};
    use ratatui::style::{Color, Style};
    use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap};

    match widget {
        LuaWidget::Text { content, color: _ } => {
            let p = Paragraph::new(content.as_str());
            frame.render_widget(p, area);
        }
        LuaWidget::Button { id, label } => {
            let button_area = Rect {
                height: 1.min(area.height),
                ..area
            };
            hitboxes.push((id.clone(), button_area));
            let is_focused = focused_id == Some(id);
            let style = if is_focused {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            let p = Paragraph::new(format!("[ {} ]", label)).style(style);
            frame.render_widget(p, button_area);
        }
        LuaWidget::Toggle { id, label, state } => {
            let toggle_area = Rect {
                height: 1.min(area.height),
                ..area
            };
            hitboxes.push((id.clone(), toggle_area));
            let is_focused = focused_id == Some(id);
            let style = if is_focused {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            let p = Paragraph::new(format!("[{}] {}", if *state { "X" } else { " " }, label))
                .style(style);
            frame.render_widget(p, toggle_area);
        }
        LuaWidget::Layout {
            direction,
            children,
        } => {
            let is_horizontal = direction.as_deref() == Some("horizontal");
            let mut constraints = Vec::new();
            // Equal constraints for simplicity, or we could use constraints if provided.
            for _ in children {
                constraints.push(Constraint::Ratio(1, children.len().max(1) as u32));
            }
            let dir = if is_horizontal {
                Direction::Horizontal
            } else {
                Direction::Vertical
            };
            let chunks = Layout::default()
                .direction(dir)
                .constraints(constraints)
                .split(area);

            for (i, child) in children.iter().enumerate() {
                if let Some(child_area) = chunks.get(i) {
                    render_widget(child, frame, *child_area, _colors, focused_id, hitboxes);
                }
            }
        }
        LuaWidget::Clock { format, color: _ } => {
            let fmt_str = format.as_deref().unwrap_or("%H:%M:%S");
            let time_str = chrono::Local::now().format(fmt_str).to_string();
            let p = Paragraph::new(time_str).alignment(Alignment::Right);
            frame.render_widget(p, area);
        }
        LuaWidget::Gauge {
            label,
            ratio,
            color: _,
        } => {
            let mut gauge = Gauge::default()
                .block(Block::default().borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
                .ratio((*ratio).clamp(0.0, 1.0));
            if let Some(l) = label {
                gauge = gauge.label(l.as_str());
            }
            frame.render_widget(gauge, area);
        }
        LuaWidget::List {
            id,
            items,
            selected,
        } => {
            if let Some(lid) = id {
                hitboxes.push((lid.clone(), area));
            }
            let is_focused = id.as_deref() == focused_id;
            let mut list_items = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let mut style = Style::default();
                if Some(i) == *selected {
                    style = style.fg(Color::Black).bg(Color::Cyan);
                }
                list_items.push(ListItem::new(item.as_str()).style(style));
            }
            let mut block = Block::default().borders(Borders::ALL);
            if is_focused {
                block = block.border_style(Style::default().fg(Color::Cyan));
            }
            let list = List::new(list_items).block(block);
            frame.render_widget(list, area);
        }
        LuaWidget::Paragraph { content, wrap } => {
            let mut p = Paragraph::new(content.as_str());
            if *wrap {
                p = p.wrap(Wrap { trim: true });
            }
            frame.render_widget(p, area);
        }
        LuaWidget::Popup { title, content } => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(title.as_deref().unwrap_or(""))
                .border_style(Style::default().fg(Color::Cyan));

            let popup_w = 50u16.min(area.width.saturating_sub(2));
            let popup_h = 10u16.min(area.height.saturating_sub(2));
            let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
            let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
            let popup_area = Rect::new(x, y, popup_w, popup_h);

            frame.render_widget(Clear, popup_area); // Clear background
            let inner_area = block.inner(popup_area);
            frame.render_widget(block, popup_area);
            render_widget(content, frame, inner_area, _colors, focused_id, hitboxes);
        }
    }
}

// region:    --- Support

fn toggle_widget(children: &mut [LuaWidget], target_id: &str) -> Option<bool> {
    for widget in children {
        match widget {
            LuaWidget::Toggle { id, state, .. } if id == target_id => {
                *state = !*state;
                return Some(*state);
            }
            LuaWidget::Layout { children, .. } => {
                if let Some(s) = toggle_widget(children, target_id) {
                    return Some(s);
                }
            }
            LuaWidget::Popup { content, .. } => {
                if let Some(s) = toggle_widget(std::slice::from_mut(content), target_id) {
                    return Some(s);
                }
            }
            _ => {}
        }
    }
    None
}

fn move_list_selection(
    children: &mut [LuaWidget],
    target_id: &str,
    forward: bool,
) -> Option<usize> {
    for widget in children {
        match widget {
            LuaWidget::List {
                id: Some(id),
                items,
                selected,
                ..
            } if id == target_id => {
                if !items.is_empty() {
                    let current = selected.unwrap_or(0);
                    let next = if forward {
                        (current + 1) % items.len()
                    } else {
                        current.checked_sub(1).unwrap_or(items.len() - 1)
                    };
                    *selected = Some(next);
                    return Some(next);
                }
            }
            LuaWidget::Layout { children, .. } => {
                if let Some(s) = move_list_selection(children, target_id, forward) {
                    return Some(s);
                }
            }
            LuaWidget::Popup { content, .. } => {
                if let Some(s) =
                    move_list_selection(std::slice::from_mut(content), target_id, forward)
                {
                    return Some(s);
                }
            }
            _ => {}
        }
    }
    None
}

// endregion: --- Support
