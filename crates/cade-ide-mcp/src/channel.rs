//! Editor ↔ `cade-ide-mcp` callback channel.
//!
//! Editor adapters (the VS Code extension, the JetBrains plugin, tests)
//! implement [`EditorChannel`]. Read-only tools only inspect
//! [`crate::EditorState`] and do not touch the channel; mutating tools
//! (edit / task / terminal / debug) route through the channel to call
//! back into the editor.

use async_trait::async_trait;
use rmcp::model::ErrorData;

use crate::state::{ApplyEditRequest, Range};

/// Callbacks the MCP tools can invoke against the connected editor.
///
/// The trait is `async_trait`-shaped so adapters can be stored as
/// `Arc<dyn EditorChannel>` (rmcp tools hold one) while still using
/// async methods — native `async fn` in traits is not yet
/// dyn-compatible without nightly features.
///
/// Every mutating method has a default implementation that returns
/// JSON-RPC `method not found` / `-32601`. Concrete adapters override
/// the callbacks they actually support; [`NullEditorChannel`] keeps
/// every default, which makes the mutating-tool surface fail loudly
/// until a real adapter attaches.
#[async_trait]
pub trait EditorChannel: Send + Sync + 'static {
    /// Human-readable label for the adapter
    /// (`"vscode-1.90.0"`, `"intellij-2025.1"`, `"null"`).
    fn label(&self) -> &str;

    /// `true` when the adapter is connected and able to service
    /// callbacks; `false` before the adapter attaches or after it
    /// disconnects. Read-only tools are unaffected.
    fn is_connected(&self) -> bool;

    /// Apply a batch of [`crate::state::TextEdit`]s to a single open
    /// file. Default implementation refuses with
    /// `ErrorData::method_not_found`.
    async fn apply_edit(&self, _req: ApplyEditRequest) -> Result<(), ErrorData> {
        Err(ErrorData::new(
            rmcp::model::ErrorCode::METHOD_NOT_FOUND,
            format!(
                "editor adapter '{}' does not support apply_edit",
                self.label()
            ),
            None,
        ))
    }

    /// Open the file at `path` in the editor and bring it into focus,
    /// creating a tab if it is not already open. Default implementation
    /// refuses with `ErrorData::method_not_found`.
    async fn reveal_file(&self, _path: String) -> Result<(), ErrorData> {
        Err(ErrorData::new(
            rmcp::model::ErrorCode::METHOD_NOT_FOUND,
            format!(
                "editor adapter '{}' does not support reveal_file",
                self.label()
            ),
            None,
        ))
    }

    /// Replace the active selection in `path` with `range`. The file
    /// must be open; the adapter is free to also reveal it. Default
    /// implementation refuses with `ErrorData::method_not_found`.
    async fn set_selection(
        &self,
        _path: String,
        _range: Range,
    ) -> Result<(), ErrorData> {
        Err(ErrorData::new(
            rmcp::model::ErrorCode::METHOD_NOT_FOUND,
            format!(
                "editor adapter '{}' does not support set_selection",
                self.label()
            ),
            None,
        ))
    }

    /// Save the open buffer at `path`, or — when `path` is `None` —
    /// save every dirty buffer. Default implementation refuses with
    /// `ErrorData::method_not_found`.
    async fn save(&self, _path: Option<String>) -> Result<(), ErrorData> {
        Err(ErrorData::new(
            rmcp::model::ErrorCode::METHOD_NOT_FOUND,
            format!(
                "editor adapter '{}' does not support save",
                self.label()
            ),
            None,
        ))
    }

    /// Run a named editor task (e.g. a VS Code `tasks.json` entry, a
    /// JetBrains run configuration). Output, exit code, and lifecycle
    /// are the adapter's responsibility; this callback returns once
    /// the task has been *started*. Default implementation refuses
    /// with `ErrorData::method_not_found`.
    async fn run_task(&self, _name: String) -> Result<(), ErrorData> {
        Err(ErrorData::new(
            rmcp::model::ErrorCode::METHOD_NOT_FOUND,
            format!(
                "editor adapter '{}' does not support run_task",
                self.label()
            ),
            None,
        ))
    }
}

/// No-op channel used before a real adapter attaches and by tests that
/// only exercise read-only behaviors. `label()` is `"null"`,
/// `is_connected()` is always `false`, and every mutating callback
/// inherits the trait default (method-not-found error).
#[derive(Debug, Default, Clone, Copy)]
pub struct NullEditorChannel;

#[async_trait]
impl EditorChannel for NullEditorChannel {
    fn label(&self) -> &str {
        "null"
    }
    fn is_connected(&self) -> bool {
        false
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_channel_reports_disconnected_with_label_null() {
        let c = NullEditorChannel;
        assert_eq!(c.label(), "null");
        assert!(!c.is_connected());
    }

    #[test]
    fn editor_channel_is_object_safe_and_send_sync() {
        // Compile-time assertion: EditorChannel must be trait-object-safe
        // (so adapters can be stored as `Arc<dyn EditorChannel>`) and
        // Send + Sync (so tokio can share them across tasks).
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn EditorChannel>();

        let boxed: std::sync::Arc<dyn EditorChannel> = std::sync::Arc::new(NullEditorChannel);
        assert_eq!(boxed.label(), "null");
    }

    #[tokio::test]
    async fn default_apply_edit_returns_method_not_supported() {
        let c = NullEditorChannel;
        let err = c
            .apply_edit(ApplyEditRequest {
                path: "/tmp/a.rs".into(),
                text_edits: vec![],
            })
            .await
            .expect_err("NullEditorChannel must refuse apply_edit");
        assert_eq!(err.code.0, rmcp::model::ErrorCode::METHOD_NOT_FOUND.0);
    }

    #[tokio::test]
    async fn default_reveal_file_returns_method_not_supported() {
        let c = NullEditorChannel;
        let err = c
            .reveal_file("/tmp/a.rs".into())
            .await
            .expect_err("NullEditorChannel must refuse reveal_file");
        assert_eq!(err.code.0, rmcp::model::ErrorCode::METHOD_NOT_FOUND.0);
    }

    #[tokio::test]
    async fn default_set_selection_returns_method_not_supported() {
        use crate::state::{Position, Range};
        let c = NullEditorChannel;
        let err = c
            .set_selection(
                "/tmp/a.rs".into(),
                Range {
                    start: Position { line: 0, character: 0 },
                    end:   Position { line: 0, character: 0 },
                },
            )
            .await
            .expect_err("NullEditorChannel must refuse set_selection");
        assert_eq!(err.code.0, rmcp::model::ErrorCode::METHOD_NOT_FOUND.0);
    }

    #[tokio::test]
    async fn default_save_returns_method_not_supported() {
        let c = NullEditorChannel;
        let err = c
            .save(Some("/tmp/a.rs".into()))
            .await
            .expect_err("NullEditorChannel must refuse save");
        assert_eq!(err.code.0, rmcp::model::ErrorCode::METHOD_NOT_FOUND.0);

        let err_all = c
            .save(None)
            .await
            .expect_err("NullEditorChannel must refuse save(None)");
        assert_eq!(err_all.code.0, rmcp::model::ErrorCode::METHOD_NOT_FOUND.0);
    }

    #[tokio::test]
    async fn default_run_task_returns_method_not_supported() {
        let c = NullEditorChannel;
        let err = c
            .run_task("build".into())
            .await
            .expect_err("NullEditorChannel must refuse run_task");
        assert_eq!(err.code.0, rmcp::model::ErrorCode::METHOD_NOT_FOUND.0);
    }
}

// endregion: --- Tests
