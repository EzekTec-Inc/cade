/// CADE terminal UI layer — ratatui-based rendering.
///
/// # Architecture
///
/// - [`OutputRenderer`]: Renders all streaming and bounded output.
///   - Streaming text (reasoning/assistant chunks) → direct stdout with word-wrap
///   - Bounded content (tool calls, system msgs) → ratatui `insert_before` boxes
/// - [`InputWidget`]: Ratatui `Viewport::Inline(5)` input box + status line.
///   Replaces the raw-crossterm `read_line()` implementation.
/// - [`ThinkingBar`]: 1-row animated status bar pinned to `term_h-1` while
///   the agent is working.

pub mod input;
pub mod output;
pub mod status;

pub use input::{InputWidget, RawModeGuard};
pub use output::{OutputRenderer, make_relative_path};
pub use status::ThinkingBar;
