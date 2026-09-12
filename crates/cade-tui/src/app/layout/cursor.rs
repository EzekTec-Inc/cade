use crate::app::*;
use crate::colors::ThemeColorsExt;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;
use tui_textarea::TextArea;

pub(crate) fn input_mode_badge(mode: InputMode, colors: &ThemeColors) -> (&'static str, RC) {
    match mode {
        InputMode::Regular => (" CHAT ", colors.c_bg_surface2()),
        InputMode::BashCommand { silent: false } => (" SHELL ", colors.c_warning()),
        InputMode::BashCommand { silent: true } => (" LOCAL ", colors.c_border_base()),
        InputMode::SlashCommand => (" COMMAND ", colors.c_primary()),
    }
}

pub(crate) fn calc_input_rows(buf: &str, available_width: u16, prefix_width: u16) -> u16 {
    let w = available_width.saturating_sub(prefix_width).max(1);
    if buf.is_empty() {
        return 1;
    }
    let mut total: u16 = 0;
    for seg in buf.split('\n') {
        total += crate::app::render::count_wrapped_segment(seg, w);
    }
    total.clamp(1, MAX_INPUT_ROWS)
}

pub(crate) fn calc_visual_cursor(
    buf: &str,
    cursor_row: usize,
    cursor_col: usize,
    available_width: u16,
    prefix_width: u16,
) -> (u16, u16) {
    let w = available_width.saturating_sub(prefix_width).max(1);
    let mut visual_y = 0;
    let mut current_row = 0;
    let mut visual_x = 0;

    for seg in buf.split('\n') {
        if current_row < cursor_row {
            visual_y += crate::app::render::count_wrapped_segment(seg, w);
            current_row += 1;
        } else if current_row == cursor_row {
            let mut row_w = 0;
            let mut y_offset = 0;
            let mut char_offset = 0;

            for word in seg.split_inclusive([' ', '\t']) {
                let word_w = unicode_width::UnicodeWidthStr::width(word) as u16;
                let word_char_len = word.chars().count();

                if row_w > 0 && row_w + word_w > w {
                    y_offset += 1;
                    row_w = 0;
                }

                // If cursor is inside this word
                if cursor_col >= char_offset && cursor_col <= char_offset + word_char_len {
                    // Calculate exactly how far into the word the cursor is in chars
                    let prefix_chars = cursor_col - char_offset;
                    let prefix: String = word.chars().take(prefix_chars).collect();
                    let prefix_w = unicode_width::UnicodeWidthStr::width(prefix.as_str()) as u16;

                    if word_w > w {
                        // Word itself wraps across multiple lines
                        let total_w = row_w + prefix_w;
                        let extra_rows = total_w / w;
                        y_offset += extra_rows;
                        row_w = total_w % w;
                    } else {
                        row_w += prefix_w;
                    }

                    // Special case: if the cursor is exactly at the width boundary
                    // and not trailing space, terminal cursor usually wraps to the next line.
                    if row_w == w {
                        y_offset += 1;
                        row_w = 0;
                    }

                    visual_y += y_offset;
                    visual_x = row_w;
                    break;
                }

                // Advance state for next word
                if word_w > w {
                    let total_w = row_w + word_w;
                    let extra_rows = total_w / w;
                    y_offset += extra_rows;
                    row_w = total_w % w;
                } else {
                    row_w += word_w;
                }

                char_offset += word_char_len;
            }

            if seg.is_empty() {
                visual_y += y_offset;
                visual_x = row_w;
            }
            break;
        }
    }

    (visual_x, visual_y)
}

pub(crate) fn rendered_textarea_cursor_position(
    textarea: &TextArea<'_>,
    area: Rect,
    cursor_style: Style,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }

    let mut buf = Buffer::empty(area);
    textarea.render(area, &mut buf);

    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            let cell = buf.cell((x, y))?;
            let style = cell.style();
            if cursor_style.fg.is_some()
                && cursor_style.bg.is_some()
                && style.fg == cursor_style.fg
                && style.bg == cursor_style.bg
            {
                return Some((x.saturating_sub(area.x), y.saturating_sub(area.y)));
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calc_visual_cursor_with_multibyte_characters() {
        let buf = "🔄 prefix test";
        // cursor is right after the emoji '🔄'
        // '🔄' is 1 character, but 4 bytes in UTF-8
        let cursor_col = 1;
        let (x, y) = calc_visual_cursor(buf, 0, cursor_col, 80, 0);

        // Should compile and run without panicking on char boundaries!
        assert_eq!(y, 0);
        assert!(x > 0);
    }

    #[test]
    fn test_calc_visual_cursor_uses_rendered_textarea_width_without_prompt_prefix() {
        let buf = "abcdefghij";
        let cursor_row = 0;
        let cursor_col = 10;
        let rendered_textarea_width = 10;

        let (x, y) = calc_visual_cursor(buf, cursor_row, cursor_col, rendered_textarea_width, 0);

        assert_eq!(y, 1);
        assert_eq!(x, 0);
    }

    #[test]
    fn test_rendered_textarea_cursor_position_tracks_library_wrap_for_tabs() {
        let cursor_style = Style::default().fg(RC::Red).bg(RC::Blue);
        let mut textarea = TextArea::from(["abc\tdef"]);
        textarea.set_wrap_mode(tui_textarea::WrapMode::Word);
        textarea.set_cursor_style(cursor_style);
        textarea.move_cursor(tui_textarea::CursorMove::Jump(0, 4));

        let rendered =
            rendered_textarea_cursor_position(&textarea, Rect::new(0, 0, 6, 3), cursor_style);

        assert_eq!(rendered, Some((0, 1)));
        assert_ne!(
            calc_visual_cursor("abc\tdef", 0, 4, 6, 0),
            rendered.unwrap()
        );
    }

    #[test]
    fn test_rendered_textarea_cursor_position_tracks_library_viewport() {
        let cursor_style = Style::default().fg(RC::Red).bg(RC::Blue);
        let mut textarea = TextArea::from(["alpha beta gamma delta"]);
        textarea.set_wrap_mode(tui_textarea::WrapMode::Word);
        textarea.set_cursor_style(cursor_style);
        textarea.move_cursor(tui_textarea::CursorMove::End);

        let rendered =
            rendered_textarea_cursor_position(&textarea, Rect::new(0, 0, 6, 2), cursor_style);

        assert_eq!(rendered, Some((5, 1)));
    }

    #[test]
    fn test_calc_visual_cursor_with_overflowing_and_scrolling_clamping() {
        let buf = "line1\nline2\nline3\nline4\nline5\nline6\nline7";
        let cursor_row = 6;
        let cursor_col = 3;

        let (x, y) = calc_visual_cursor(buf, cursor_row, cursor_col, 40, 10);
        assert_eq!(y, 6);
        assert_eq!(x, 3);

        let height = 3;
        let relative_y = if y >= height {
            let scroll_top = y.saturating_sub(height).saturating_add(1);
            y.saturating_sub(scroll_top)
        } else {
            y
        };

        assert_eq!(relative_y, 2);
    }
}
