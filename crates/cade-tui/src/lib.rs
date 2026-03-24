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
pub mod session_tree;
pub mod component;
pub mod editor;
pub mod markdown;
pub mod menu;
pub mod question;
pub mod skills;

pub use app::{RenderLine, TuiApp, cycle_mode, cycle_mode_back, truncate_str};
pub use colors::ThemeColors;
pub use session_tree::{TreeAction, show_session_tree};
pub use autocomplete::{
    AutocompleteProvider, Completion, FileAutocompleteProvider, SlashCommandDef,
    SlashCommandProvider,
};
pub use component::{Component, RenderedLine};
pub use editor::Editor;
pub use question::{Question, QuestionAnswer, QuestionOption, QuestionWidget};

// endregion: --- Modules
