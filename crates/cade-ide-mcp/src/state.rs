//! In-memory editor state — snapshot of the connected editor's open
//! files, selection, diagnostics, and workspace folders.

/// A single file currently open in an editor tab.
///
/// Phase M-IDE-1a carries just enough information for open-file
/// counting; later phases extend this with buffer text, dirty flag,
/// language id, and version counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFile {
    /// Absolute filesystem path. `None` for unsaved scratch buffers.
    pub path: Option<String>,
}

/// Shared editor-state handle. Phase M-IDE-1a ships an empty skeleton;
/// later phases add selection / diagnostic fields and async accessors.
#[derive(Debug, Default, Clone)]
pub struct EditorState {
    open_files: Vec<OpenFile>,
}

impl EditorState {
    /// Build a fresh, empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of files currently open in the editor.
    pub fn open_file_count(&self) -> usize {
        self.open_files.len()
    }

    /// Replace the open-file list with a fresh snapshot from the adapter.
    pub fn replace_open_files(&mut self, files: Vec<OpenFile>) {
        self.open_files = files;
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

    #[test]
    fn replace_open_files_updates_count() {
        let mut s = EditorState::new();
        s.replace_open_files(vec![
            OpenFile { path: Some("/tmp/a.rs".into()) },
            OpenFile { path: Some("/tmp/b.rs".into()) },
        ]);
        assert_eq!(s.open_file_count(), 2);
    }
}

// endregion: --- Tests


