//! In-memory editor state — snapshot of the connected editor's open
//! files, selection, diagnostics, and workspace folders.
//!
//! [`EditorState`] is an `Arc<RwLock<…>>` handle, so clones share
//! storage. The editor adapter clones it to push updates; tools clone
//! it to read them.

use std::sync::Arc;

use tokio::sync::RwLock;

/// A single file currently open in an editor tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFile {
    /// Absolute filesystem path. `None` for unsaved scratch buffers.
    pub path: Option<String>,
}

/// 0-indexed line + UTF-16 code-unit offset (LSP convention).
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub path: String,
    pub range: Range,
    pub text: String,
}

/// Severity of a diagnostic reported by the editor's language services.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// A single diagnostic (compile error, lint warning, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: String,
    pub range: Range,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

/// A workspace root opened in the editor (e.g. a repo checkout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFolder {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Default)]
struct Inner {
    open_files: Vec<OpenFile>,
    active_file: Option<String>,
    selection: Option<Selection>,
    diagnostics: Vec<Diagnostic>,
    workspace_folders: Vec<WorkspaceFolder>,
    visible_range: Option<(u32, u32)>,
}

/// Shared, thread-safe editor-state handle. Clones share storage.
#[derive(Debug, Default, Clone)]
pub struct EditorState {
    inner: Arc<RwLock<Inner>>,
}

impl EditorState {
    /// Build a fresh, empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of files currently open in the editor.
    pub async fn open_file_count(&self) -> usize {
        self.inner.read().await.open_files.len()
    }

    /// Snapshot of all files currently open in editor tabs.
    pub async fn open_files_snapshot(&self) -> Vec<OpenFile> {
        self.inner.read().await.open_files.clone()
    }

    /// Replace the open-file list with a fresh snapshot from the adapter.
    pub async fn replace_open_files(&self, files: Vec<OpenFile>) {
        self.inner.write().await.open_files = files;
    }

    /// Path of the file the user is currently focused on, if any.
    pub async fn active_file(&self) -> Option<String> {
        self.inner.read().await.active_file.clone()
    }

    /// Update the currently-focused file. Pass `None` to clear.
    pub async fn set_active_file(&self, path: Option<String>) {
        self.inner.write().await.active_file = path;
    }

    /// The user's current selection, if any.
    pub async fn selection(&self) -> Option<Selection> {
        self.inner.read().await.selection.clone()
    }

    /// Update the current selection. Pass `None` to clear.
    pub async fn set_selection(&self, sel: Option<Selection>) {
        self.inner.write().await.selection = sel;
    }

    /// Snapshot of all diagnostics currently reported across the workspace.
    pub async fn diagnostics(&self) -> Vec<Diagnostic> {
        self.inner.read().await.diagnostics.clone()
    }

    /// Replace the full diagnostic list with a fresh snapshot.
    pub async fn replace_diagnostics(&self, diags: Vec<Diagnostic>) {
        self.inner.write().await.diagnostics = diags;
    }

    /// Snapshot of workspace roots the editor currently has open.
    pub async fn workspace_folders(&self) -> Vec<WorkspaceFolder> {
        self.inner.read().await.workspace_folders.clone()
    }

    /// Replace the workspace-folder list with a fresh snapshot.
    pub async fn replace_workspace_folders(&self, folders: Vec<WorkspaceFolder>) {
        self.inner.write().await.workspace_folders = folders;
    }

    /// `(start_line, end_line)` of the active editor viewport, 0-indexed
    /// inclusive on both ends. `None` when no editor is focused.
    pub async fn visible_range(&self) -> Option<(u32, u32)> {
        self.inner.read().await.visible_range
    }

    /// Update the visible viewport range. Pass `None` to clear.
    pub async fn set_visible_range(&self, range: Option<(u32, u32)>) {
        self.inner.write().await.visible_range = range;
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn clones_share_storage_after_mutation() {
        let a = EditorState::new();
        let b = a.clone();
        b.set_active_file(Some("/tmp/a.rs".into())).await;
        assert_eq!(a.active_file().await.as_deref(), Some("/tmp/a.rs"));
    }

    #[tokio::test]
    async fn new_state_has_no_open_files() {
        let s = EditorState::new();
        assert_eq!(s.open_file_count().await, 0);
    }

    #[tokio::test]
    async fn replace_open_files_updates_count() {
        let s = EditorState::new();
        s.replace_open_files(vec![
            OpenFile { path: Some("/tmp/a.rs".into()) },
            OpenFile { path: Some("/tmp/b.rs".into()) },
        ])
        .await;
        assert_eq!(s.open_file_count().await, 2);
    }

    #[tokio::test]
    async fn active_file_round_trips_through_setter() {
        let s = EditorState::new();
        assert_eq!(s.active_file().await, None);
        s.set_active_file(Some("/tmp/a.rs".into())).await;
        assert_eq!(s.active_file().await.as_deref(), Some("/tmp/a.rs"));
        s.set_active_file(None).await;
        assert_eq!(s.active_file().await, None);
    }

    #[tokio::test]
    async fn selection_round_trips_through_setter() {
        let s = EditorState::new();
        assert_eq!(s.selection().await, None);

        let sel = Selection {
            path: "/tmp/a.rs".into(),
            range: Range {
                start: Position { line: 1, character: 0 },
                end:   Position { line: 1, character: 5 },
            },
            text: "hello".into(),
        };
        s.set_selection(Some(sel.clone())).await;
        assert_eq!(s.selection().await, Some(sel));

        s.set_selection(None).await;
        assert_eq!(s.selection().await, None);
    }

    #[tokio::test]
    async fn replace_diagnostics_updates_slice() {
        let s = EditorState::new();
        assert_eq!(s.diagnostics().await.len(), 0);

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
        s.replace_diagnostics(vec![d.clone()]).await;
        assert_eq!(s.diagnostics().await, vec![d]);
    }

    #[tokio::test]
    async fn replace_workspace_folders_updates_slice() {
        let s = EditorState::new();
        assert_eq!(s.workspace_folders().await.len(), 0);

        let f = WorkspaceFolder {
            path: "/home/eng/proj".into(),
            name: "proj".into(),
        };
        s.replace_workspace_folders(vec![f.clone()]).await;
        assert_eq!(s.workspace_folders().await, vec![f]);
    }

    #[tokio::test]
    async fn visible_range_round_trips_through_setter() {
        let s = EditorState::new();
        assert_eq!(s.visible_range().await, None);
        s.set_visible_range(Some((5, 42))).await;
        assert_eq!(s.visible_range().await, Some((5, 42)));
        s.set_visible_range(None).await;
        assert_eq!(s.visible_range().await, None);
    }
}

// endregion: --- Tests
