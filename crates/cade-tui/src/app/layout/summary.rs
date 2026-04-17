use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::Span,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use crate::colors::ThemeColors;
use crate::app::SummaryState;

pub fn render_summary(
    frame: &mut Frame,
    state: &SummaryState,
    area: Rect,
    colors: &ThemeColors,
) {
    let w = (area.width * 80 / 100).max(40).min(area.width.saturating_sub(4));
    let h = (area.height * 80 / 100).max(10).min(area.height.saturating_sub(4));

    let rect = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(colors.border_style.to_ratatui())
        .border_style(colors.border_focus())
        .title(Span::styled(" Conversation Summary ", colors.primary_bold()));

    let paragraph = Paragraph::new(state.text.as_str())
        .block(block)
        .style(colors.text_primary())
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_y, 0));

    frame.render_widget(paragraph, rect);

    // Scrollbar-like indicator or instructions
    let instructions = " Esc/Enter to close • Up/Down/PgUp/PgDn to scroll ";
    let instr_x = rect.x + rect.width.saturating_sub(instructions.chars().count() as u16) / 2;
    let instr_y = rect.y + rect.height - 1;
    
    if rect.width > instructions.chars().count() as u16 + 2 {
        frame.render_widget(
            Paragraph::new(Span::styled(instructions, colors.text_dim().add_modifier(Modifier::DIM))),
            Rect::new(instr_x, instr_y, instructions.chars().count() as u16, 1),
        );
    }
}
