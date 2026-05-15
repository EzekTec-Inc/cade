//! Minimal mouse capture — scroll-wheel only.
//!
//! crossterm's `EnableMouseCapture` enables **all** mouse tracking modes
//! (`?1000h`, `?1002h`, `?1003h`, `?1006h`), which captures clicks, drags,
//! and all motion — preventing terminal-native text selection.
//!
//! CADE only needs scroll-wheel events.  These custom commands enable only:
//! - `?1000h` — normal tracking (button press/release, includes scroll)
//! - `?1006h` — SGR encoding (extended coordinates)
//!
//! This leaves click-and-drag free for the terminal to handle natively,
//! so users can select and copy text without any toggle or slash command.

use std::fmt;

/// Enable scroll-wheel capture only (`?1000h` + `?1006h`).
///
/// Does NOT enable `?1002h` (drag tracking) or `?1003h` (all-motion tracking),
/// so the terminal retains native text selection via click-and-drag.
pub struct EnableScrollCapture;

impl crossterm::Command for EnableScrollCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        // Normal tracking: reports button press/release (scroll = button 4/5)
        f.write_str("\x1b[?1000h")?;
        // SGR mouse mode: extended coordinates (>223 columns)
        f.write_str("\x1b[?1006h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        // Fall back to crossterm's full mouse capture on Windows
        crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)
    }
}

/// Disable scroll-wheel capture (reverses `EnableScrollCapture`).
pub struct DisableScrollCapture;

impl crossterm::Command for DisableScrollCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str("\x1b[?1006l")?;
        f.write_str("\x1b[?1000l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::Command;

    #[test]
    fn enable_scroll_capture_emits_correct_sequences() {
        let mut buf = String::new();
        EnableScrollCapture.write_ansi(&mut buf).unwrap();
        assert!(buf.contains("\x1b[?1000h"), "must enable normal tracking");
        assert!(buf.contains("\x1b[?1006h"), "must enable SGR encoding");
        assert!(!buf.contains("1002"), "must NOT enable drag tracking");
        assert!(!buf.contains("1003"), "must NOT enable all-motion tracking");
    }

    #[test]
    fn disable_scroll_capture_emits_correct_sequences() {
        let mut buf = String::new();
        DisableScrollCapture.write_ansi(&mut buf).unwrap();
        assert!(buf.contains("\x1b[?1000l"), "must disable normal tracking");
        assert!(buf.contains("\x1b[?1006l"), "must disable SGR encoding");
    }
}
