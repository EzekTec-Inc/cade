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
            let seg_before = if cursor_col <= seg.len() {
                &seg[..cursor_col]
            } else {
                seg
            };
            
            let mut row_w = 0;
            let mut y_offset = 0;
            
            // To find the exact visual x and y offset for the cursor within this line,
            // we do a simple character-by-character wrap simulation.
            // This is an approximation of word wrap, but since we just need the hardware
            // cursor to not fly off screen, it's sufficient to place it generally correctly.
            // A more exact approach is to use count_wrapped_segment on the substring.
            
            for word in seg_before.split_inclusive([' ', '\t']) {
                let word_w = unicode_width::UnicodeWidthStr::width(word) as u16;
                if row_w > 0 && row_w + word_w > w {
                    y_offset += 1;
                    row_w = 0;
                }
                if word_w > w {
                    let extra_rows = (word_w.saturating_sub(1)) / w;
                    y_offset += extra_rows;
                    row_w = word_w - (extra_rows * w);
                } else {
                    row_w += word_w;
                }
            }
            
            // If the cursor is right at a boundary where a new line would start,
            // row_w will equal w, and we should wrap it.
            if row_w == w && !seg_before.ends_with(' ') {
                // It might stay at the edge depending on terminal behavior, 
                // but we clamp it below anyway.
            }
            
            visual_y += y_offset;
            visual_x = row_w;
            break;
        }
    }
    
    (visual_x.min(w.saturating_sub(1)), visual_y)
}
