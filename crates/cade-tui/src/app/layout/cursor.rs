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


