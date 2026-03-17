/// CADE terminal UI layer — pure ratatui fullscreen rendering.
///
/// Single render path: all output goes through [`TuiApp`] which owns one
/// `ratatui::DefaultTerminal` (alternate screen, raw mode). No hybrid DECSTBM/
/// inline-viewport hacks — every frame redraws the full screen from state.

pub mod app;
pub mod autocomplete;
pub mod component;
pub mod editor;
pub mod question;
pub mod menu;
pub mod markdown;
pub mod stats;

pub use app::{TuiApp, RenderLine, cycle_mode, cycle_mode_back, truncate_str, SkillsOverlayState, SkillsMode};
pub use stats::{SessionStats, ModelStats};
pub use autocomplete::{AutocompleteProvider, FileAutocompleteProvider, SlashCommandProvider, SlashCommandDef, Completion};
pub use component::{Component, RenderedLine};
pub use editor::Editor;
pub use question::{QuestionWidget, QuestionAnswer, QuestionOption, Question};
