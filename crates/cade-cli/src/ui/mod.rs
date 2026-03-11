/// CADE terminal UI layer — pure ratatui fullscreen rendering.
///
/// Single render path: all output goes through [`TuiApp`] which owns one
/// `ratatui::DefaultTerminal` (alternate screen, raw mode). No hybrid DECSTBM/
/// inline-viewport hacks — every frame redraws the full screen from state.

pub mod app;
pub mod question;
pub mod menu;
pub mod markdown;

pub use app::{TuiApp, RenderLine, cycle_mode, cycle_mode_back, truncate_str, SkillsOverlayState, SkillsMode};
pub use question::{QuestionWidget, QuestionAnswer, QuestionOption, Question};
