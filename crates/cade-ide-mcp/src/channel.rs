//! Editor ↔ `cade-ide-mcp` callback channel.
//!
//! Editor adapters (the VS Code extension, the JetBrains plugin, tests)
//! implement [`EditorChannel`]. Read-only tools only inspect
//! [`crate::EditorState`] and do not touch the channel; edit / task /
//! terminal / debug tools will (in later phases) route through the
//! channel to call back into the editor.

/// Callbacks the MCP tools can invoke against the connected editor.
///
/// Phase M-IDE-1a exposes only lifecycle methods: a label for the
/// adapter and a connection flag. Callback methods for editing, tasks,
/// the terminal, and the debugger are added in subsequent phases so
/// each lands with its own failing test.
pub trait EditorChannel: Send + Sync + 'static {
    /// Human-readable label for the adapter
    /// (`"vscode-1.90.0"`, `"intellij-2025.1"`, `"null"`).
    fn label(&self) -> &str;

    /// `true` when the adapter is connected and able to service
    /// callbacks; `false` before the adapter attaches or after it
    /// disconnects. Read-only tools are unaffected.
    fn is_connected(&self) -> bool;
}

/// No-op channel used before a real adapter attaches and by tests that
/// only exercise read-only behaviors. `label()` is `"null"` and
/// `is_connected()` is always `false`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullEditorChannel;

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
}

// endregion: --- Tests

