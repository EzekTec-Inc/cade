//! The MCP server that wraps [`EditorState`] and an [`EditorChannel`]
//! into a single handler. Tools defined via rmcp's `#[tool]` macro
//! (added in a later cycle) will read from the state and call back
//! into the channel to apply edits, run tasks, etc.

use std::sync::Arc;

use crate::channel::{EditorChannel, NullEditorChannel};
use crate::state::EditorState;

/// Top-level MCP server handler — the only type editor adapters
/// instantiate. Holds a shared [`EditorState`] that the adapter pushes
/// snapshots into, and an `Arc<dyn EditorChannel>` through which later
/// phases will issue callbacks into the editor (apply edits, run tasks,
/// control the terminal / debugger).
#[derive(Clone)]
pub struct IdeMcpServer {
    state: EditorState,
    channel: Arc<dyn EditorChannel>,
}

impl IdeMcpServer {
    /// Build a server with the supplied state and adapter.
    pub fn new(state: EditorState, channel: Arc<dyn EditorChannel>) -> Self {
        Self { state, channel }
    }

    /// Convenience constructor using [`NullEditorChannel`] for tests and
    /// for the warm-up period before a real editor attaches.
    pub fn with_null_channel(state: EditorState) -> Self {
        Self::new(state, Arc::new(NullEditorChannel))
    }

    /// The shared editor-state handle. Adapters clone this to push
    /// snapshots; tools clone it to read them.
    pub fn state(&self) -> &EditorState {
        &self.state
    }

    /// Label of the attached adapter (`"null"` before an editor connects).
    pub fn channel_label(&self) -> &str {
        self.channel.label()
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::EditorState;

    #[test]
    fn server_with_null_channel_builds_and_exposes_state() {
        let state = EditorState::new();
        let server = IdeMcpServer::with_null_channel(state);
        assert_eq!(server.channel_label(), "null");
        assert_eq!(server.state().open_file_count(), 0);
    }
}

// endregion: --- Tests
