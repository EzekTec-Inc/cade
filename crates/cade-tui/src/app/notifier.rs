//! Terminal Attention Cues and Desktop Notifications (`TerminalNotifier`).
//!
//! Emits terminal bell (`\x07`) and OSC 9 / OSC 99 desktop notification sequences.

// region:    --- Imports

use std::io::{self, Write};

// endregion: --- Imports

// region:    --- Types

/// Event types that can trigger terminal attention cues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionCue {
    /// Agent finished executing its turn.
    TurnFinished,
    /// Permission confirmation requested from the user.
    PermissionPrompt,
    /// Background subagent or team finished execution.
    SubagentComplete,
    /// Unrecoverable task or build failure.
    TaskError,
}

// endregion: --- Types

// region:    --- TerminalNotifier

/// Deep module managing terminal bells and OSC desktop notifications.
#[derive(Debug, Clone)]
pub struct TerminalNotifier {
    pub enable_bell: bool,
    pub enable_osc: bool,
    pub is_silent: bool,
}

impl Default for TerminalNotifier {
    fn default() -> Self {
        Self {
            enable_bell: true,
            enable_osc: true,
            is_silent: false,
        }
    }
}

impl TerminalNotifier {
    pub fn new(enable_bell: bool, enable_osc: bool, is_silent: bool) -> Self {
        Self {
            enable_bell,
            enable_osc,
            is_silent,
        }
    }

    /// Format an OSC 9 notification sequence (supported by ConEmu, Windows Terminal, iTerm2).
    pub fn format_osc9(message: &str) -> String {
        format!("\x1b]9;{}\x07", message.replace(';', " "))
    }

    /// Format an OSC 99 desktop notification sequence (Ghostty, Kitty, modern terminals).
    pub fn format_osc99(title: &str, body: &str) -> String {
        format!(
            "\x1b]99;i=1:d=0;{}\x1b\\\x1b]99;i=1:d=1:p=body;{}\x1b\\",
            title.replace(';', " "),
            body.replace(';', " ")
        )
    }

    /// Emit attention cues to stdout if enabled.
    pub fn notify(&self, cue: AttentionCue, title: &str, body: &str) {
        if self.is_silent {
            return;
        }

        let mut stdout = io::stdout();

        // 1. Emit Terminal Bell if enabled
        if self.enable_bell {
            let _ = write!(stdout, "\x07");
        }

        // 2. Emit OSC notification sequences if enabled
        if self.enable_osc {
            let seq = match cue {
                AttentionCue::PermissionPrompt => {
                    Self::format_osc99(&format!("⚠️ {}", title), body)
                }
                AttentionCue::SubagentComplete | AttentionCue::TurnFinished => {
                    Self::format_osc99(&format!("✅ {}", title), body)
                }
                AttentionCue::TaskError => Self::format_osc99(&format!("❌ {}", title), body),
            };
            let _ = write!(stdout, "{}", seq);
        }

        let _ = stdout.flush();
    }
}

// endregion: --- TerminalNotifier

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_osc9_sequence_formatting() {
        let seq = TerminalNotifier::format_osc9("CADE Task Complete");
        assert_eq!(seq, "\x1b]9;CADE Task Complete\x07");
    }

    #[test]
    fn test_osc99_sequence_formatting() {
        let seq = TerminalNotifier::format_osc99("CADE", "Build finished with 0 errors");
        assert!(seq.starts_with("\x1b]99;i=1:d=0;CADE\x1b\\"));
        assert!(seq.contains("Build finished with 0 errors"));
    }
}

// endregion: --- Tests
