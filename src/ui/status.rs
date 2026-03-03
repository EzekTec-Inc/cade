/// ThinkingBar — animated 1-row status bar pinned to the terminal bottom.
///
/// Renders while the agent is working (`agent_turn`). All content output
/// from `OutputRenderer::with_insert_before` anchors to `term_h - 2` (one
/// row above this bar) so content scrolls without overwriting it.
///
/// Layout (during agent work):
/// ```
/// … [content scrolling above] …
/// ⠋ ● read_file…                   ← ThinkingBar at term_h-1
/// ```
///
/// After the turn it clears itself; the InputWidget then renders its own
/// 5-row block (including a status/summary row) over that area.

use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crossterm::{
    cursor, execute, terminal,
    style::{Color, ResetColor, SetForegroundColor},
};
use tokio::task::JoinHandle;

const BRAILLE: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A running thinking bar.  Drop + `stop()` to clear.
pub struct ThinkingBar {
    /// Shared text updated by the caller (e.g. per tool call).
    pub text: Arc<std::sync::Mutex<String>>,
    /// Set to pause rendering while a modal (e.g. QuestionWidget) owns the terminal.
    pub pause_flag: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
}

impl ThinkingBar {
    /// Spawn the animation background task and return the bar handle + join handle.
    pub fn start() -> (Self, JoinHandle<()>) {
        let text: Arc<std::sync::Mutex<String>> =
            Arc::new(std::sync::Mutex::new("CADE thinking…".to_string()));
        let stop_flag  = Arc::new(AtomicBool::new(false));
        let pause_flag = Arc::new(AtomicBool::new(false));

        let text2  = text.clone();
        let stop2  = stop_flag.clone();
        let pause2 = pause_flag.clone();

        let handle = tokio::spawn(async move {
            let mut i: usize = 0;
            loop {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                if !pause2.load(Ordering::SeqCst) {
                    Self::render_frame(i, &text2);
                    i += 1;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
            }
            // Clear the status row on exit
            if let Ok((_, term_h)) = terminal::size() {
                let mut out = io::stdout();
                let _ = execute!(
                    out,
                    cursor::MoveToRow(term_h.saturating_sub(1)),
                    terminal::Clear(terminal::ClearType::CurrentLine),
                );
                let _ = out.flush();
            }
        });

        (Self { text, pause_flag, stop_flag }, handle)
    }

    /// Update the status text (e.g. when a new tool call starts).
    pub fn set_text(&self, s: impl Into<String>) {
        *self.text.lock().unwrap() = s.into();
    }

    /// Signal the task to stop (non-blocking; await the JoinHandle to wait for cleanup).
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    fn render_frame(frame_idx: usize, text: &Arc<std::sync::Mutex<String>>) {
        let Ok((term_w, term_h)) = terminal::size() else { return };
        let glyph = BRAILLE[frame_idx % BRAILLE.len()];
        let label = text.lock().unwrap().clone();
        // Truncate to fit terminal width (spinner + space + label, max term_w chars)
        let full = format!("{glyph} {label}");
        let truncated: String = full.chars().take(term_w as usize).collect();

        // Write directly without any Terminal::with_options — avoids cursor position
        // queries (\033[6n → stdin) which race with the event loop's event::read().
        // SavePosition / RestorePosition are pure stdout commands with no stdin read.
        let mut out = io::stdout();
        let _ = execute!(
            out,
            cursor::SavePosition,
            cursor::MoveToRow(term_h.saturating_sub(1)),
            cursor::MoveToColumn(0),
            terminal::Clear(terminal::ClearType::CurrentLine),
            SetForegroundColor(Color::DarkGrey),
        );
        let _ = write!(out, "{truncated}");
        let _ = execute!(out, ResetColor, cursor::RestorePosition);
        let _ = out.flush();
    }
}
