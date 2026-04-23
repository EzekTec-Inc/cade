//! In-memory editor state — snapshot of the connected editor's open
//! files, selection, diagnostics, and workspace folders.

/// Shared editor-state handle. Phase M-IDE-1a ships an empty skeleton;
/// later phases add open-file / selection / diagnostic fields and
/// async accessors.
#[derive(Debug, Default, Clone)]
pub struct EditorState;

impl EditorState {
    /// Build a fresh, empty state.
    pub fn new() -> Self {
        Self
    }

    /// Number of files currently open in the editor.
    pub fn open_file_count(&self) -> usize {
        0
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_has_no_open_files() {
        let s = EditorState::new();
        assert_eq!(s.open_file_count(), 0);
    }
}

// endregion: --- Tests
