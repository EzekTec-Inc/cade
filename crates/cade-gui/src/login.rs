//! Login screen state machine for the cade-gui WASM client.
//!
//! Kept pure so it is fully native-testable.  The egui widget in `app.rs`
//! drives this machine via `on_input` / `on_submit`; the render code
//! contains no conditional logic of its own.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginState {
    /// User is entering their API key.  `buffer` is the text-field content
    /// (including any pasted whitespace — trimming happens only on submit).
    Entering { buffer: String },
    /// User pressed Connect with a non-empty (post-trim) token.  `key`
    /// carries the trimmed value.
    Submitted { key: String },
}

impl Default for LoginState {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginState {
    /// Fresh state — empty buffer, waiting for user input.
    pub fn new() -> Self {
        Self::Entering {
            buffer: String::new(),
        }
    }

    /// Current text-field content.  Returns `""` for `Submitted` so render
    /// code can display nothing after submit without case-matching.
    pub fn buffer(&self) -> &str {
        match self {
            Self::Entering { buffer } => buffer,
            Self::Submitted { .. } => "",
        }
    }

    /// Replace the entering buffer.  No-op after submit so the captured key
    /// cannot be mutated by stray keystrokes.
    pub fn on_input(&mut self, s: &str) {
        if let Self::Entering { buffer } = self {
            buffer.clear();
            buffer.push_str(s);
        }
    }

    /// Attempt to submit.  Trims surrounding whitespace; if the trimmed
    /// value is empty the state stays in `Entering`.
    pub fn on_submit(&mut self) {
        if let Self::Entering { buffer } = self {
            let trimmed = buffer.trim();
            if !trimmed.is_empty() {
                let key = trimmed.to_string();
                *self = Self::Submitted { key };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_in_entering_with_empty_buffer() {
        let s = LoginState::new();
        assert!(matches!(s, LoginState::Entering { .. }));
        assert_eq!(s.buffer(), "");
    }

    #[test]
    fn on_input_updates_buffer() {
        let mut s = LoginState::new();
        s.on_input("abc");
        assert_eq!(s.buffer(), "abc");
        s.on_input("abcdef");
        assert_eq!(s.buffer(), "abcdef");
    }

    #[test]
    fn on_submit_with_non_empty_buffer_transitions_to_submitted() {
        let mut s = LoginState::new();
        s.on_input("my-token");
        s.on_submit();
        match s {
            LoginState::Submitted { ref key } => assert_eq!(key, "my-token"),
            _ => panic!("expected Submitted, got {s:?}"),
        }
    }

    #[test]
    fn on_submit_with_empty_buffer_stays_in_entering() {
        let mut s = LoginState::new();
        s.on_submit();
        assert!(matches!(s, LoginState::Entering { .. }));
    }

    #[test]
    fn on_submit_with_whitespace_only_stays_in_entering() {
        let mut s = LoginState::new();
        s.on_input("   ");
        s.on_submit();
        assert!(matches!(s, LoginState::Entering { .. }));
    }

    #[test]
    fn on_submit_trims_surrounding_whitespace() {
        let mut s = LoginState::new();
        s.on_input("  tok  \n");
        s.on_submit();
        match s {
            LoginState::Submitted { ref key } => assert_eq!(key, "tok"),
            _ => panic!("expected Submitted"),
        }
    }

    #[test]
    fn on_input_ignored_after_submit() {
        let mut s = LoginState::new();
        s.on_input("tok");
        s.on_submit();
        s.on_input("evil-append");
        match s {
            LoginState::Submitted { ref key } => assert_eq!(key, "tok"),
            _ => panic!("expected Submitted"),
        }
    }
}
