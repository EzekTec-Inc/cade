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

/// 0-indexed line + UTF-16 code-unit offset within that line (LSP
/// convention). Matching LSP keeps the VS Code and JetBrains adapters
/// straightforward to wire up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// An inclusive-start / exclusive-end range in a text document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// The user's current text selection in the active editor.
///
/// An empty [`Range`] (start == end) represents a caret position with no
/// highlighted selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// File path the selection lives in. Must be one of the open files.
    pub path: String,
    /// The selected range.
    pub range: Range,
    /// Convenience: the text covered by `range`.
    pub text: String,
}

/// Severity of a diagnostic reported by the editor's language services.
/// Mirrors the LSP `DiagnosticSeverity` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// A single diagnostic (compile error, lint warning, etc.) reported by
/// the editor's language services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: String,
    pub range: Range,
    pub severity: DiagnosticSeverity,
    /// Human-readable message (e.g. `"unused variable: `x`"`).
    pub message: String,
    /// Producer (e.g. `"rustc"`, `"tsc"`, `"eslint"`).
    pub source: Option<String>,
    /// Optional rule code (e.g. `"E0001"`, `"noUnusedLocals"`).
    pub code: Option<String>,
}

/// Shared editor-state handle. Phase M-IDE-1a ships an empty skeleton;
/// later phases add workspace-folder fields and async accessors.
#[derive(Debug, Default, Clone)]
pub struct EditorState {
    open_files: Vec<OpenFile>,
    active_file: Option<String>,
    selection: Option<Selection>,
    diagnostics: Vec<Diagnostic>,
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

    /// Path of the file the user is currently focused on, if any.
    pub fn active_file(&self) -> Option<&str> {
        self.active_file.as_deref()
    }

    /// Update the currently-focused file. Pass `None` to clear.
    pub fn set_active_file(&mut self, path: Option<String>) {
        self.active_file = path;
    }

    /// The user's current selection, if any.
    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// Update the current selection. Pass `None` to clear.
    pub fn set_selection(&mut self, sel: Option<Selection>) {
        self.selection = sel;
    }

    /// All diagnostics currently reported across the workspace.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Replace the full diagnostic list with a fresh snapshot.
    pub fn replace_diagnostics(&mut self, diags: Vec<Diagnostic>) {
        self.diagnostics = diags;
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

    #[test]
    fn active_file_round_trips_through_setter() {
        let mut s = EditorState::new();
        assert_eq!(s.active_file(), None);
        s.set_active_file(Some("/tmp/a.rs".into()));
        assert_eq!(s.active_file(), Some("/tmp/a.rs"));
        s.set_active_file(None);
        assert_eq!(s.active_file(), None);
    }

    #[test]
    fn selection_round_trips_through_setter() {
        let mut s = EditorState::new();
        assert_eq!(s.selection(), None);

        let sel = Selection {
            path: "/tmp/a.rs".into(),
            range: Range {
                start: Position { line: 1, character: 0 },
                end:   Position { line: 1, character: 5 },
            },
            text: "hello".into(),
        };
        s.set_selection(Some(sel.clone()));
        assert_eq!(s.selection(), Some(&sel));

        s.set_selection(None);
        assert_eq!(s.selection(), None);
    }

    #[test]
    fn replace_diagnostics_updates_slice() {
        let mut s = EditorState::new();
        assert_eq!(s.diagnostics().len(), 0);

        let d = Diagnostic {
            path: "/tmp/a.rs".into(),
            range: Range {
                start: Position { line: 0, character: 0 },
                end:   Position { line: 0, character: 4 },
            },
            severity: DiagnosticSeverity::Error,
            message: "unused variable: `x`".into(),
            source: Some("rustc".into()),
            code: Some("E0001".into()),
        };
        s.replace_diagnostics(vec![d.clone()]);
        assert_eq!(s.diagnostics(), &[d]);
    }
}

// endregion: --- Tests


