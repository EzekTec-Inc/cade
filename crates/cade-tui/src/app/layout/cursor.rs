use crate::app::*;
use crate::colors::ThemeColorsExt;

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
            let mut byte_offset = 0;

            for word in seg.split_inclusive([' ', '\t']) {
                let word_w = unicode_width::UnicodeWidthStr::width(word) as u16;
                let word_len = word.len();
                
                if row_w > 0 && row_w + word_w > w {
                    y_offset += 1;
                    row_w = 0;
                }

                // If cursor is inside this word
                if cursor_col >= byte_offset && cursor_col <= byte_offset + word_len {
                    // Calculate exactly how far into the word the cursor is
                    let prefix = &word[..(cursor_col - byte_offset)];
                    let prefix_w = unicode_width::UnicodeWidthStr::width(prefix) as u16;
                    
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
                
                byte_offset += word_len;
            }

            // If cursor_col was exactly at the end of the segment (or beyond),
            // the loop will handle it because split_inclusive includes the end.
            // But if segment is empty and cursor is 0, we still need to break.
            if seg.is_empty() {
                visual_y += y_offset;
                visual_x = row_w;
            }

            break;
        }
    }

    (visual_x.min(w.saturating_sub(1)), visual_y)
}
