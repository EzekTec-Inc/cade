/// Re-export the standalone `cade-tui` crate so existing `crate::ui::*` paths
/// throughout `cade-cli` continue to resolve without changes.
// region:    --- Modules
pub use cade_tui::*;

// endregion: --- Modules
