use cade_core::resources::themes::{ThemeColors, ColorDef};

fn main() {
    let dark = ThemeColors::dark();
    let bg_base = dark.bg_base;
    
    // GUI mapping logic
    let gui_rgb = match bg_base {
        ColorDef::Rgb(r, g, b) => (r, g, b),
        ColorDef::Reset => (205, 214, 244),
    };

    // TUI mapping logic
    let tui_rgb = match bg_base {
        ColorDef::Rgb(r, g, b) => (r, g, b),
        ColorDef::Reset => (0, 0, 0),
    };
    
    if gui_rgb != tui_rgb {
        println!("Drift detected in bg_base: GUI={:?}, TUI={:?}", gui_rgb, tui_rgb);
    } else {
        println!("bg_base matches.");
    }
}
