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
