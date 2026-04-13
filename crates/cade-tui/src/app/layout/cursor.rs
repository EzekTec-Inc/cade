use crate::app::*;

pub(crate) fn input_mode_badge(mode: InputMode, colors: &ThemeColors) -> (&'static str, RC) {
    match mode {
        InputMode::Regular => (" CHAT ", colors.bg_surface2),
        InputMode::BashCommand { silent: false } => (" SHELL ", colors.warning),
        InputMode::BashCommand { silent: true } => (" LOCAL ", colors.border_base),
        InputMode::SlashCommand => (" COMMAND ", colors.primary),
    }
}

pub(crate) fn calc_input_rows(buf: &str, available_width: u16, prefix_width: u16) -> u16 {
    let w = available_width.max(1) as usize;
    let first_row_capacity = w.saturating_sub(prefix_width as usize).max(1);
    if buf.is_empty() {
        return 1;
    }
    let mut total: u16 = 0;
    for seg in buf.split('\n') {
        let chars = seg.chars().count();
        let rows = if chars == 0 {
            1
        } else if chars <= first_row_capacity {
            1
        } else {
            1 + (chars - first_row_capacity).div_ceil(w) as u16
        };
        total += rows;
    }
    total.clamp(1, MAX_INPUT_ROWS)
}

pub(crate) fn calc_visual_cursor(
    before_cursor: &str,
    available_width: u16,
    prefix_width: u16,
) -> (u16, u16) {
    // Mirror exactly how render_frame builds the Paragraph:
    //   • Each logical line (split on '\n') is its own ratatui Line.
    //   • The first visual row starts after the input-mode badge + "> " prefix.
    //   • The paragraph uses Wrap { trim: false }, meaning it wraps exactly
    //     at the available_width boundary. Wrapped lines do NOT get the prefix
    //     so they start at column 0.
    let w = available_width.max(1) as usize;

    let mut vis_row: u16 = 0;
    let mut vis_col: u16 = prefix_width;

    for (li, seg) in before_cursor.split('\n').enumerate() {
        if li > 0 {
            // Crossed a \n: start a new logical line → new visual row, prefix col
            vis_row += 1;
            vis_col = prefix_width;
        }
        // Walk through the segment, wrapping when we exceed available width
        let mut chars_on_row = vis_col as usize;
        for _ch in seg.chars() {
            chars_on_row += 1;
            if chars_on_row > w {
                // Wrap to next visual row within this logical line
                vis_row += 1;
                chars_on_row = 1;
                vis_col = 1; // 0-indexed column is 0, so 1st char is length 1
            } else {
                vis_col = chars_on_row as u16;
            }
        }
        // After processing all chars of this segment, vis_col is already set
        // correctly for the end of the segment.  If the segment was empty
        // (bare \n), vis_col stays at prefix_width (just the prefix).
    }

    (vis_row, vis_col)
}

/// Given the full input `buf`, the visual text-column width `text_w`
/// (= available_width - 2, matching `calc_visual_cursor`), and a target
/// `(row, col)` in visual space, return the **byte offset** in `buf` of the
/// character at that visual position.
/// Used by the Up/Down cursor-movement logic.
pub(crate) fn find_cursor_at_visual_row_col(
    buf: &str,
    available_width: u16,
    prefix_width: u16,
    target_row: u16,
    target_col: u16,
) -> usize {
    let text_w = available_width.max(1) as usize;
    let mut vis_row: u16 = 0;
    let mut chars_on_row: usize = prefix_width as usize;
    let mut byte_offset: usize = 0;

    for (li, seg) in buf.split('\n').enumerate() {
        if li > 0 {
            vis_row += 1;
            chars_on_row = prefix_width as usize;
            byte_offset += 1; // the '\n' byte
        }
        if vis_row > target_row {
            break;
        }
        let seg_start = byte_offset;
        for ch in seg.chars() {
            chars_on_row += 1;
            if chars_on_row > text_w {
                // visual wrap
                vis_row += 1;
                chars_on_row = 1;
            }
            if vis_row == target_row {
                // We're on the target row — check column
                // target_col is raw screen column; chars_on_row matches raw length
                let content_col = target_col as usize;
                if chars_on_row > content_col {
                    return byte_offset;
                }
            }
            if vis_row > target_row {
                // Overshot — return last valid position on target row
                return byte_offset;
            }
            byte_offset += ch.len_utf8();
        }
        // If we passed through the whole segment without overshooting, the
        // cursor target is at the end of the segment (or beyond — clamp to end).
        if vis_row == target_row {
            // Return end of this segment (before the next \n or end of string)
            return byte_offset;
        }
        let _ = seg_start; // suppress unused warning
    }
    // Clamp to end of buffer
    buf.len()
}
