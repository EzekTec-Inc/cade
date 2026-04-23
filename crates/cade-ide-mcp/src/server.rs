//! The MCP server that wraps [`EditorState`] and an [`EditorChannel`]
//! into a single handler. Tools defined via rmcp's `#[tool]` macro read
//! from the state (and, in later phases, call back into the channel to
//! apply edits, run tasks, etc.).

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{ErrorData, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

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
    // Read by rmcp's `#[tool_handler]` expansion through `Self::tool_router()`;
    // the compiler can't see that indirection and warns the field is unused.
    #[allow(dead_code)]
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

/// One entry in the `get_open_files` tool response.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct OpenFileSummary {
    /// Absolute filesystem path. `None` for unsaved scratch buffers.
    pub path: Option<String>,
}

/// Output of the `get_open_files` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetOpenFilesOut {
    pub files: Vec<OpenFileSummary>,
}

/// Output of the `get_selection` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetSelectionOut {
    /// `None` when the user has no active selection.
    pub selection: Option<crate::state::Selection>,
}

/// Output of the `get_diagnostics` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetDiagnosticsOut {
    pub diagnostics: Vec<crate::state::Diagnostic>,
}

/// Output of the `get_workspace_folders` tool.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetWorkspaceFoldersOut {
    pub folders: Vec<crate::state::WorkspaceFolder>,
}

/// Output of the `get_visible_range` tool.
///
/// `start_line` and `end_line` are 0-indexed inclusive. Both fields are
/// `None` when no editor is focused.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetVisibleRangeOut {
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

/// Input of the `get_file_content` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetFileContentIn {
    /// Absolute filesystem path of an open file to read.
    pub path: String,
}

/// Output of the `get_file_content` tool. Mirrors the LSP
/// `TextDocumentItem` shape for the single file that was requested.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GetFileContentOut {
    pub path: String,
    pub text: String,
    pub language_id: String,
    pub version: u64,
    pub is_dirty: bool,
}

/// Output of the `apply_edit` tool. Empty on success — the editor is
/// the source of truth for the resulting buffer state, which the
/// adapter will push back through `EditorState` as a follow-up update.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ApplyEditOut {}

/// Input of the `open_file` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OpenFileIn {
    /// Absolute filesystem path of the file to open and reveal.
    pub path: String,
}

/// Output of the `open_file` tool. Empty on success.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct OpenFileOut {}

impl IdeMcpServer {
    /// Test-friendly accessor behind `get_active_file`. The `#[tool]`
    /// method delegates here so unit tests can drive the logic without
    /// constructing a `ToolCallContext`.
    async fn get_active_file_impl(&self) -> GetActiveFileOut {
        GetActiveFileOut {
            path: self.state.active_file().await,
        }
    }

    /// Test-friendly accessor behind `get_open_files`.
    async fn get_open_files_impl(&self) -> GetOpenFilesOut {
        let open = self.state.open_files_snapshot().await;
        GetOpenFilesOut {
            files: open
                .into_iter()
                .map(|f| OpenFileSummary { path: f.path })
                .collect(),
        }
    }

    /// Test-friendly accessor behind `get_selection`.
    async fn get_selection_impl(&self) -> GetSelectionOut {
        GetSelectionOut {
            selection: self.state.selection().await,
        }
    }

    /// Test-friendly accessor behind `get_diagnostics`.
    async fn get_diagnostics_impl(&self) -> GetDiagnosticsOut {
        GetDiagnosticsOut {
            diagnostics: self.state.diagnostics().await,
        }
    }

    /// Test-friendly accessor behind `get_workspace_folders`.
    async fn get_workspace_folders_impl(&self) -> GetWorkspaceFoldersOut {
        GetWorkspaceFoldersOut {
            folders: self.state.workspace_folders().await,
        }
    }

    /// Test-friendly accessor behind `get_visible_range`.
    async fn get_visible_range_impl(&self) -> GetVisibleRangeOut {
        match self.state.visible_range().await {
            Some((s, e)) => GetVisibleRangeOut {
                start_line: Some(s),
                end_line: Some(e),
            },
            None => GetVisibleRangeOut {
                start_line: None,
                end_line: None,
            },
        }
    }

    /// Test-friendly accessor behind `get_file_content`.
    ///
    /// Returns `ErrorData::invalid_params` (JSON-RPC -32602) when the
    /// path is not currently open — the agent should not trigger a
    /// filesystem read fallback; the adapter owns buffer state.
    async fn get_file_content_impl(
        &self,
        path: String,
    ) -> Result<GetFileContentOut, ErrorData> {
        let open = self.state.open_files_snapshot().await;
        let hit = open
            .into_iter()
            .find(|f| f.path.as_deref() == Some(path.as_str()));
        match hit {
            Some(f) => Ok(GetFileContentOut {
                path,
                text: f.text,
                language_id: f.language_id,
                version: f.version,
                is_dirty: f.is_dirty,
            }),
            None => Err(ErrorData::invalid_params(
                format!("file not open in editor: {path}"),
                None,
            )),
        }
    }

    /// Test-friendly accessor behind `apply_edit`. Forwards the request
    /// to the attached `EditorChannel`; errors bubble up as-is (e.g.
    /// `method_not_found` from `NullEditorChannel`).
    async fn apply_edit_impl(
        &self,
        req: crate::state::ApplyEditRequest,
    ) -> Result<ApplyEditOut, ErrorData> {
        self.channel.apply_edit(req).await?;
        Ok(ApplyEditOut {})
    }

    /// Test-friendly accessor behind `open_file`. Forwards `path` to
    /// `EditorChannel::reveal_file`; errors bubble up unchanged.
    async fn open_file_impl(&self, path: String) -> Result<OpenFileOut, ErrorData> {
        self.channel.reveal_file(path).await?;
        Ok(OpenFileOut {})
    }
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
        Json(self.get_active_file_impl().await)
    }

    /// Return the list of files currently open in editor tabs.
    #[tool(
        name = "get_open_files",
        description = "Return the list of files currently open in editor tabs."
    )]
    async fn get_open_files(&self) -> Json<GetOpenFilesOut> {
        Json(self.get_open_files_impl().await)
    }

    /// Return the user's current text selection in the active editor.
    #[tool(
        name = "get_selection",
        description = "Return the user's current text selection in the active editor, or null if no selection exists."
    )]
    async fn get_selection(&self) -> Json<GetSelectionOut> {
        Json(self.get_selection_impl().await)
    }

    /// Return all diagnostics (compile errors, lint warnings, …) across the workspace.
    #[tool(
        name = "get_diagnostics",
        description = "Return all diagnostics (compile errors, lint warnings, info, hints) currently reported across the workspace by the editor's language services."
    )]
    async fn get_diagnostics(&self) -> Json<GetDiagnosticsOut> {
        Json(self.get_diagnostics_impl().await)
    }

    /// Return the list of workspace roots the editor currently has open.
    #[tool(
        name = "get_workspace_folders",
        description = "Return the list of workspace roots (repo checkouts, project folders) currently open in the editor."
    )]
    async fn get_workspace_folders(&self) -> Json<GetWorkspaceFoldersOut> {
        Json(self.get_workspace_folders_impl().await)
    }

    /// Return the line range visible in the active editor's viewport.
    #[tool(
        name = "get_visible_range",
        description = "Return the (start_line, end_line) range currently visible in the active editor's viewport, 0-indexed inclusive. Both fields are null when no editor is focused."
    )]
    async fn get_visible_range(&self) -> Json<GetVisibleRangeOut> {
        Json(self.get_visible_range_impl().await)
    }

    /// Return the full text of a single open file.
    #[tool(
        name = "get_file_content",
        description = "Return the full buffer text of a single open file, identified by its absolute path. Errors if the path is not currently open in the editor — the agent should not trigger a filesystem read; the editor adapter owns buffer state."
    )]
    async fn get_file_content(
        &self,
        Parameters(GetFileContentIn { path }): Parameters<GetFileContentIn>,
    ) -> Result<Json<GetFileContentOut>, ErrorData> {
        self.get_file_content_impl(path).await.map(Json)
    }

    /// Apply a batch of text edits to a single open file.
    #[tool(
        name = "apply_edit",
        description = "Apply a batch of text edits (LSP TextEdit shape) to a single open file. Errors with method_not_found if no editor adapter is attached, or with invalid_params when the path is not open."
    )]
    async fn apply_edit(
        &self,
        Parameters(req): Parameters<crate::state::ApplyEditRequest>,
    ) -> Result<Json<ApplyEditOut>, ErrorData> {
        self.apply_edit_impl(req).await.map(Json)
    }

    /// Open a file in the editor and bring it into focus.
    #[tool(
        name = "open_file",
        description = "Open the file at `path` in the editor and bring it into focus, creating a tab if it is not already open. Errors with method_not_found if no editor adapter is attached."
    )]
    async fn open_file(
        &self,
        Parameters(OpenFileIn { path }): Parameters<OpenFileIn>,
    ) -> Result<Json<OpenFileOut>, ErrorData> {
        self.open_file_impl(path).await.map(Json)
    }
}

/// Expose [`IdeMcpServer`] as an MCP `ServerHandler`.
///
/// The `tool_router` field is used automatically by `#[tool_handler]`
/// to route `tools/call` requests; we override `get_info()` so the
/// crate name, version, capabilities, and instructions are advertised
/// via the MCP `initialize` response.
#[tool_handler]
impl ServerHandler for IdeMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut server_info = Implementation::default();
        server_info.name = "cade-ide-mcp".into();
        server_info.version = env!("CARGO_PKG_VERSION").into();

        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::LATEST;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = server_info;
        info.instructions = Some(
            "CADE IDE MCP bridge — exposes the connected editor's state \
             (open files, selection, diagnostics, workspace folders, …) \
             to CADE agents as MCP tools."
                .into(),
        );
        info
    }
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::EditorState;

    #[tokio::test]
    async fn server_with_null_channel_builds_and_exposes_state() {
        let state = EditorState::new();
        let server = IdeMcpServer::with_null_channel(state);
        assert_eq!(server.channel_label(), "null");
        assert_eq!(server.state().open_file_count().await, 0);
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

    #[tokio::test]
    async fn get_open_files_returns_adapter_pushed_list() {
        let state = EditorState::new();
        let server = IdeMcpServer::with_null_channel(state.clone());

        state
            .replace_open_files(vec![
                crate::state::OpenFile {
                    path: Some("/tmp/a.rs".into()),
                    text: String::new(),
                    language_id: "rust".into(),
                    version: 1,
                    is_dirty: false,
                },
                crate::state::OpenFile {
                    path: Some("/tmp/b.rs".into()),
                    text: String::new(),
                    language_id: "rust".into(),
                    version: 1,
                    is_dirty: false,
                },
            ])
            .await;

        let out = server.get_open_files_impl().await;
        assert_eq!(out.files.len(), 2);
        assert_eq!(out.files[0].path.as_deref(), Some("/tmp/a.rs"));
        assert_eq!(out.files[1].path.as_deref(), Some("/tmp/b.rs"));
    }

    #[test]
    fn tool_router_registers_get_open_files() {
        assert!(IdeMcpServer::tool_router().has_route("get_open_files"));
    }

    #[tokio::test]
    async fn get_selection_returns_adapter_pushed_selection() {
        use crate::state::{Position, Range, Selection};

        let state = EditorState::new();
        let server = IdeMcpServer::with_null_channel(state.clone());

        let sel = Selection {
            path: "/tmp/a.rs".into(),
            range: Range {
                start: Position { line: 1, character: 0 },
                end:   Position { line: 1, character: 5 },
            },
            text: "hello".into(),
        };
        state.set_selection(Some(sel.clone())).await;

        let out = server.get_selection_impl().await;
        assert_eq!(out.selection, Some(sel));
    }

    #[test]
    fn tool_router_registers_get_selection() {
        assert!(IdeMcpServer::tool_router().has_route("get_selection"));
    }

    #[tokio::test]
    async fn get_diagnostics_returns_adapter_pushed_list() {
        use crate::state::{Diagnostic, DiagnosticSeverity, Position, Range};

        let state = EditorState::new();
        let server = IdeMcpServer::with_null_channel(state.clone());

        let d = Diagnostic {
            path: "/tmp/a.rs".into(),
            range: Range {
                start: Position { line: 0, character: 0 },
                end:   Position { line: 0, character: 4 },
            },
            severity: DiagnosticSeverity::Warning,
            message: "unused import".into(),
            source: Some("rustc".into()),
            code: Some("W0001".into()),
        };
        state.replace_diagnostics(vec![d.clone()]).await;

        let out = server.get_diagnostics_impl().await;
        assert_eq!(out.diagnostics, vec![d]);
    }

    #[test]
    fn tool_router_registers_get_diagnostics() {
        assert!(IdeMcpServer::tool_router().has_route("get_diagnostics"));
    }

    #[tokio::test]
    async fn get_workspace_folders_returns_adapter_pushed_list() {
        use crate::state::WorkspaceFolder;

        let state = EditorState::new();
        let server = IdeMcpServer::with_null_channel(state.clone());

        let f = WorkspaceFolder {
            path: "/home/eng/proj".into(),
            name: "proj".into(),
        };
        state.replace_workspace_folders(vec![f.clone()]).await;

        let out = server.get_workspace_folders_impl().await;
        assert_eq!(out.folders, vec![f]);
    }

    #[test]
    fn tool_router_registers_get_workspace_folders() {
        assert!(IdeMcpServer::tool_router().has_route("get_workspace_folders"));
    }

    #[tokio::test]
    async fn get_visible_range_returns_adapter_pushed_range() {
        let state = EditorState::new();
        let server = IdeMcpServer::with_null_channel(state.clone());
        state.set_visible_range(Some((5, 42))).await;

        let out = server.get_visible_range_impl().await;
        assert_eq!(out.start_line, Some(5));
        assert_eq!(out.end_line, Some(42));
    }

    #[test]
    fn tool_router_registers_get_visible_range() {
        assert!(IdeMcpServer::tool_router().has_route("get_visible_range"));
    }

    #[tokio::test]
    async fn get_file_content_returns_text_for_matching_path() {
        let state = EditorState::new();
        let server = IdeMcpServer::with_null_channel(state.clone());

        state
            .replace_open_files(vec![crate::state::OpenFile {
                path: Some("/tmp/a.rs".into()),
                text: "fn main() {}\n".into(),
                language_id: "rust".into(),
                version: 3,
                is_dirty: true,
            }])
            .await;

        let out = server
            .get_file_content_impl("/tmp/a.rs".to_string())
            .await
            .expect("expected file to be found");
        assert_eq!(out.text, "fn main() {}\n");
        assert_eq!(out.language_id, "rust");
        assert_eq!(out.version, 3);
        assert!(out.is_dirty);
    }

    #[tokio::test]
    async fn get_file_content_errors_when_path_not_open() {
        let server = IdeMcpServer::with_null_channel(EditorState::new());
        let err = server
            .get_file_content_impl("/nope.rs".to_string())
            .await
            .expect_err("expected not-found error");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("/nope.rs"),
            "expected path echoed in error, got: {msg}"
        );
    }

    #[test]
    fn tool_router_registers_get_file_content() {
        assert!(IdeMcpServer::tool_router().has_route("get_file_content"));
    }

    #[tokio::test]
    async fn apply_edit_forwards_request_to_channel() {
        use crate::channel::EditorChannel;
        use crate::state::{ApplyEditRequest, Position, Range, TextEdit};
        use async_trait::async_trait;
        use std::sync::{Arc, Mutex};

        struct RecordingChannel {
            calls: Mutex<Vec<ApplyEditRequest>>,
        }

        #[async_trait]
        impl EditorChannel for RecordingChannel {
            fn label(&self) -> &str {
                "recording"
            }
            fn is_connected(&self) -> bool {
                true
            }
            async fn apply_edit(
                &self,
                req: ApplyEditRequest,
            ) -> Result<(), rmcp::model::ErrorData> {
                self.calls.lock().unwrap().push(req);
                Ok(())
            }
        }

        let channel = Arc::new(RecordingChannel {
            calls: Mutex::new(Vec::new()),
        });
        let state = EditorState::new();
        let server = IdeMcpServer::new(state, channel.clone());

        let req = ApplyEditRequest {
            path: "/tmp/a.rs".into(),
            text_edits: vec![TextEdit {
                range: Range {
                    start: Position { line: 0, character: 0 },
                    end:   Position { line: 0, character: 0 },
                },
                new_text: "// hi\n".into(),
            }],
        };
        server
            .apply_edit_impl(req.clone())
            .await
            .expect("apply_edit should succeed");

        let calls = channel.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], req);
    }

    #[test]
    fn tool_router_registers_apply_edit() {
        assert!(IdeMcpServer::tool_router().has_route("apply_edit"));
    }

    #[tokio::test]
    async fn open_file_forwards_path_to_channel() {
        use crate::channel::EditorChannel;
        use async_trait::async_trait;
        use std::sync::{Arc, Mutex};

        struct RecordingChannel {
            calls: Mutex<Vec<String>>,
        }

        #[async_trait]
        impl EditorChannel for RecordingChannel {
            fn label(&self) -> &str {
                "recording"
            }
            fn is_connected(&self) -> bool {
                true
            }
            async fn reveal_file(&self, path: String) -> Result<(), rmcp::model::ErrorData> {
                self.calls.lock().unwrap().push(path);
                Ok(())
            }
        }

        let channel = Arc::new(RecordingChannel {
            calls: Mutex::new(Vec::new()),
        });
        let server = IdeMcpServer::new(EditorState::new(), channel.clone());

        server
            .open_file_impl("/tmp/a.rs".to_string())
            .await
            .expect("open_file should succeed");

        let calls = channel.calls.lock().unwrap();
        assert_eq!(calls.as_slice(), &["/tmp/a.rs".to_string()]);
    }

    #[test]
    fn tool_router_registers_open_file() {
        assert!(IdeMcpServer::tool_router().has_route("open_file"));
    }

    #[test]
    fn server_implements_server_handler_with_expected_name() {
        fn assert_impl<T: rmcp::ServerHandler>() {}
        assert_impl::<IdeMcpServer>();

        let s = IdeMcpServer::with_null_channel(EditorState::new());
        let info = rmcp::ServerHandler::get_info(&s);
        assert_eq!(info.server_info.name, "cade-ide-mcp");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(info.capabilities.tools.is_some());
    }
}

// endregion: --- Tests
