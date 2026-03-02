/// CADE terminal UI layer — ratatui-based rendering.
///
/// # Architecture
///
/// - [`OutputRenderer`]: Renders all streaming and bounded output.
///   - Streaming text (reasoning/assistant chunks) → direct stdout with word-wrap
///   - Bounded content (tool calls, system msgs) → ratatui `insert_before` boxes
/// - [`InputWidget`]: Ratatui `Viewport::Inline(3)` input box + status line.
///   Replaces the raw-crossterm `read_line()` implementation.

pub mod input;
pub mod output;

pub use input::{InputWidget, RawModeGuard};
pub use output::{OutputRenderer, make_relative_path};
