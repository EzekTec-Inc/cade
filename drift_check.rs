
// drift_check.rs: Compare theme palette mappings between TUI and GUI
use cade_core::resources::themes::{ThemeColors, ColorDef};

// Mock GUI EguiColorExt
trait EguiColorExt {
    fn to_egui(self) -> (u8, u8, u8);
}
impl EguiColorExt for ColorDef {
    fn to_egui(self) -> (u8, u8, u8) {
        match self {
            ColorDef::Rgb(r, g, b) => (r, g, b),
            ColorDef::Reset => (205, 214, 244),
        }
    }
}

// Mock TUI ColorDefExt
trait ColorDefExt {
    fn to_ratatui(self) -> (u8, u8, u8);
}
impl ColorDefExt for ColorDef {
    fn to_ratatui(self) -> (u8, u8, u8) {
        match self {
            ColorDef::Rgb(r, g, b) => (r, g, b),
            ColorDef::Reset => (0, 0, 0), // Note: TUI Reset is usually 0,0,0 or default
        }
    }
}

fn main() {
    let dark = ThemeColors::dark();
    let bg_base = dark.bg_base;
    
    let gui = bg_base.to_egui();
    let tui = bg_base.to_ratatui();
    
    if gui != tui {
        println!("Drift detected in bg_base: GUI={:?}, TUI={:?}", gui, tui);
    } else {
        println!("bg_base matches.");
    }
}
