//! The MCP server that wraps [`EditorState`] and an [`EditorChannel`]
//! into a single handler. Tools defined via rmcp's `#[tool]` macro read
//! from the state (and, in later phases, call back into the channel to
//! apply edits, run tasks, etc.).

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Json;
use rmcp::{tool, tool_router};
use serde::Serialize;

use crate::channel::{EditorChannel, NullEditorChannel};
use crate::state::EditorState;

/// Top-level MCP server handler — the only type editor adapters
/// instantiate. Holds a shared [`EditorState`] that the adapter pushes
/// snapshots into, and an `Arc<dyn EditorChannel>` through which later
/// phases will issue callbacks into the editor.
///
/// The `tool_router` field is populated from the `#[tool_router]` macro
/// below; rmcp's `ServerHandler` implementation (added in a later cycle
/// once the stdio transport is wired) will route incoming `tools/call`
/// requests through it.
#[derive(Clone)]
pub struct IdeMcpServer {
    state: EditorState,
    channel: Arc<dyn EditorChannel>,
    #[allow(dead_code)] // ServerHandler impl lands in a later TDD cycle
    tool_router: ToolRouter<Self>,
}

impl IdeMcpServer {
    /// Build a server with the supplied state and adapter.
    pub fn new(state: EditorState, channel: Arc<dyn EditorChannel>) -> Self {
        Self {
            state,
            channel,
            tool_router: Self::tool_router(),
        }
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

/// Output of the `get_active_file` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetActiveFileOut {
    /// Path of the currently-focused file, or `None` when no editor is
    /// focused.
    pub path: Option<String>,
}

#[tool_router]
impl IdeMcpServer {
    /// Return the path of the file the user is currently focused on,
    /// or `None` when no editor is focused.
    #[tool(
        name = "get_active_file",
        description = "Return the path of the file the user is currently focused on in the editor."
    )]
    async fn get_active_file(&self) -> Json<GetActiveFileOut> {
        Json(GetActiveFileOut {
            path: self.state.active_file().map(str::to_owned),
        })
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

    #[test]
    fn tool_router_registers_get_active_file() {
        let router = IdeMcpServer::tool_router();
        assert!(
            router.has_route("get_active_file"),
            "expected get_active_file in tool list, got {:?}",
            router.list_all().iter().map(|t| t.name.clone()).collect::<Vec<_>>()
        );
    }
}

// endregion: --- Tests
