#![allow(clippy::manual_clamp)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::vec_init_then_push)]
#![allow(clippy::if_same_then_else)]
#![allow(clippy::empty_line_after_doc_comments)]
/// CADE terminal UI layer — pure ratatui fullscreen rendering.
///
/// Single render path: all output goes through [`TuiApp`] which owns one
/// `ratatui::DefaultTerminal` (alternate screen, raw mode). No hybrid DECSTBM/
/// inline-viewport hacks — every frame redraws the full screen from state.
// region:    --- Modules
mod error;

pub use error::{Error, Result};

pub mod app;
pub mod autocomplete;
pub mod colors;
pub mod component;
pub mod editor;
pub mod editor_component;
pub mod icons;
pub mod markdown;
pub mod mcp_picker;
pub mod menu;
// pub mod mouse;
pub mod lua_engine;
pub mod lua_ui;
pub mod overlay;
pub mod overlay_component;
pub mod question;
pub mod session_tree;
pub mod skills;
pub mod slots;
pub mod subagent_tracker;

pub use app::{RenderLine, ToastLevel, TuiApp, cycle_mode, cycle_mode_back, truncate_str};
pub use autocomplete::{
    AutocompleteProvider, Completion, FileAutocompleteProvider, SlashCommandDef,
    SlashCommandProvider,
};
pub use colors::ThemeColors;
pub use component::{Component, RenderedLine};
pub use editor::Editor;
pub use question::{Question, QuestionAnswer, QuestionOption, QuestionWidget};
pub use session_tree::{TreeAction, show_session_tree};

// endregion: --- Modules
pub mod test_rich;
