//! Adapter ↔ `cade-ide-mcp` wire protocol.
//!
//! The protocol is **newline-delimited JSON** carried over a TCP
//! loopback connection. Every frame is a single JSON object on one
//! line (no embedded newlines), terminated by `\n`.
//!
//! ## Message flow
//!
//! ```text
//! adapter                    cade-ide-mcp
//!   │── Hello ──────────────────────────▶│  adapter identifies itself
//!   │── StateUpdate ─────────────────────▶│  adapter pushes state snapshot
//!   │◀─ CallbackRequest ─────────────────│  MCP tool triggers a callback
//!   │── CallbackResponse ───────────────▶│  adapter returns ok/err
//!   │── StateUpdate ─────────────────────▶│  ...repeats as editor changes
//! ```
//!
//! Messages sent **from the adapter** to `cade-ide-mcp` are [`AdapterMessage`].
//! Messages sent **from `cade-ide-mcp`** to the adapter are [`ServerMessage`].

use serde::{Deserialize, Serialize};

use crate::state::{
    ApplyEditRequest, DebugAction, Diagnostic, OpenFile, Range, Selection, WorkspaceFolder,
};

// ── Adapter → server ────────────────────────────────────────────────────────

/// A complete snapshot of the editor state the adapter wants to sync.
///
/// Adapters send this immediately after `Hello` and again whenever any
/// field changes. The server replaces its `EditorState` atomically from
/// this snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub open_files: Vec<OpenFile>,
    pub active_file: Option<String>,
    pub selection: Option<Selection>,
    pub diagnostics: Vec<Diagnostic>,
    pub workspace_folders: Vec<WorkspaceFolder>,
    pub visible_range: Option<(u32, u32)>,
}

/// Messages the **adapter** sends to `cade-ide-mcp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdapterMessage {
    /// First message after connecting. Identifies the adapter.
    Hello {
        /// Human-readable label, e.g. `"vscode-1.90.0"`.
        label: String,
        /// Protocol version this adapter speaks. Currently `1`.
        protocol_version: u32,
    },

    /// Full editor-state snapshot. Replaces the server's cached state.
    StateUpdate(StateSnapshot),

    /// Response to a [`ServerMessage::CallbackRequest`].
    CallbackResponse {
        /// Mirrors the `id` from the originating `CallbackRequest`.
        id: u64,
        /// `Ok(())` → `{ "ok": null }`, `Err(msg)` → `{ "err": "…" }`.
        result: CallbackResult,
    },
}

/// Outcome of a callback, as reported by the adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallbackResult {
    /// Callback succeeded.
    Ok,
    /// Callback failed; the string is a human-readable error message.
    Err(String),
}

// ── Server → adapter ────────────────────────────────────────────────────────

/// The specific editor operation the adapter must perform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CallbackOp {
    ApplyEdit(ApplyEditRequest),
    RevealFile { path: String },
    SetSelection { path: String, range: Range },
    Save { path: Option<String> },
    RunTask { name: String },
    RunTerminal { command: String },
    DebugControl(DebugAction),
}

/// Messages `cade-ide-mcp` sends **to the adapter**.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Acknowledges a valid `Hello`. The adapter may begin sending
    /// `StateUpdate` messages immediately.
    HelloAck {
        /// Protocol version the server will use. Currently `1`.
        protocol_version: u32,
    },

    /// Request the adapter to perform an editor operation on behalf of
    /// an in-flight MCP tool call. The adapter must reply with a
    /// [`AdapterMessage::CallbackResponse`] carrying the same `id`.
    CallbackRequest {
        /// Monotonically-increasing request identifier. Unique per
        /// connection lifetime.
        id: u64,
        /// The operation to perform.
        #[serde(flatten)]
        op: CallbackOp,
    },
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ApplyEditRequest, DebugAction, Position, Range, TextEdit};

    fn round_trip<T>(msg: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de> + std::fmt::Debug + PartialEq,
    {
        let json = serde_json::to_string(msg).expect("serialize");
        // Must be a single line (no embedded newlines) so the
        // newline-delimited framing works correctly.
        assert!(
            !json.contains('\n'),
            "serialized message must not contain embedded newlines: {json}"
        );
        serde_json::from_str(&json).expect("deserialize")
    }

    // ── AdapterMessage::Hello ────────────────────────────────────────────────

    #[test]
    fn adapter_hello_round_trips() {
        let msg = AdapterMessage::Hello {
            label: "vscode-1.90.0".into(),
            protocol_version: 1,
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn adapter_hello_serializes_type_tag() {
        let msg = AdapterMessage::Hello {
            label: "test".into(),
            protocol_version: 1,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"hello""#), "json={json}");
        assert!(json.contains(r#""label":"test""#), "json={json}");
        assert!(json.contains(r#""protocol_version":1"#), "json={json}");
    }

    // ── AdapterMessage::StateUpdate ──────────────────────────────────────────

    #[test]
    fn adapter_state_update_empty_round_trips() {
        let msg = AdapterMessage::StateUpdate(StateSnapshot {
            open_files: vec![],
            active_file: None,
            selection: None,
            diagnostics: vec![],
            workspace_folders: vec![],
            visible_range: None,
        });
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn adapter_state_update_with_data_round_trips() {
        use crate::state::{DiagnosticSeverity, OpenFile, Selection, WorkspaceFolder};
        let msg = AdapterMessage::StateUpdate(StateSnapshot {
            open_files: vec![OpenFile {
                path: Some("/tmp/a.rs".into()),
                text: "fn main() {}\n".into(),
                language_id: "rust".into(),
                version: 3,
                is_dirty: true,
            }],
            active_file: Some("/tmp/a.rs".into()),
            selection: Some(Selection {
                path: "/tmp/a.rs".into(),
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 4,
                    },
                },
                text: "fn m".into(),
            }),
            diagnostics: vec![crate::state::Diagnostic {
                path: "/tmp/a.rs".into(),
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 2,
                    },
                },
                severity: DiagnosticSeverity::Warning,
                message: "unused import".into(),
                source: Some("rustc".into()),
                code: Some("W0001".into()),
            }],
            workspace_folders: vec![WorkspaceFolder {
                path: "/tmp".into(),
                name: "tmp".into(),
            }],
            visible_range: Some((0, 40)),
        });
        assert_eq!(round_trip(&msg), msg);
    }

    // ── AdapterMessage::CallbackResponse ─────────────────────────────────────

    #[test]
    fn callback_response_ok_round_trips() {
        let msg = AdapterMessage::CallbackResponse {
            id: 42,
            result: CallbackResult::Ok,
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_response_err_round_trips() {
        let msg = AdapterMessage::CallbackResponse {
            id: 7,
            result: CallbackResult::Err("file not open".into()),
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_response_serializes_type_tag() {
        let msg = AdapterMessage::CallbackResponse {
            id: 1,
            result: CallbackResult::Ok,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(
            json.contains(r#""type":"callback_response""#),
            "json={json}"
        );
        assert!(json.contains(r#""id":1"#), "json={json}");
    }

    // ── ServerMessage::HelloAck ──────────────────────────────────────────────

    #[test]
    fn server_hello_ack_round_trips() {
        let msg = ServerMessage::HelloAck {
            protocol_version: 1,
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn server_hello_ack_serializes_type_tag() {
        let msg = ServerMessage::HelloAck {
            protocol_version: 1,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"hello_ack""#), "json={json}");
    }

    // ── ServerMessage::CallbackRequest ───────────────────────────────────────

    #[test]
    fn callback_request_apply_edit_round_trips() {
        let msg = ServerMessage::CallbackRequest {
            id: 1,
            op: CallbackOp::ApplyEdit(ApplyEditRequest {
                path: "/tmp/a.rs".into(),
                text_edits: vec![TextEdit {
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                    new_text: "// header\n".into(),
                }],
            }),
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_request_reveal_file_round_trips() {
        let msg = ServerMessage::CallbackRequest {
            id: 2,
            op: CallbackOp::RevealFile {
                path: "/tmp/b.rs".into(),
            },
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_request_set_selection_round_trips() {
        let msg = ServerMessage::CallbackRequest {
            id: 3,
            op: CallbackOp::SetSelection {
                path: "/tmp/c.rs".into(),
                range: Range {
                    start: Position {
                        line: 5,
                        character: 2,
                    },
                    end: Position {
                        line: 5,
                        character: 10,
                    },
                },
            },
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_request_save_single_round_trips() {
        let msg = ServerMessage::CallbackRequest {
            id: 4,
            op: CallbackOp::Save {
                path: Some("/tmp/d.rs".into()),
            },
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_request_save_all_round_trips() {
        let msg = ServerMessage::CallbackRequest {
            id: 5,
            op: CallbackOp::Save { path: None },
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_request_run_task_round_trips() {
        let msg = ServerMessage::CallbackRequest {
            id: 6,
            op: CallbackOp::RunTask {
                name: "cargo-build".into(),
            },
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_request_run_terminal_round_trips() {
        let msg = ServerMessage::CallbackRequest {
            id: 7,
            op: CallbackOp::RunTerminal {
                command: "cargo test".into(),
            },
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_request_debug_start_round_trips() {
        let msg = ServerMessage::CallbackRequest {
            id: 8,
            op: CallbackOp::DebugControl(DebugAction::Start {
                config: "unit-tests".into(),
            }),
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_request_debug_stop_round_trips() {
        let msg = ServerMessage::CallbackRequest {
            id: 9,
            op: CallbackOp::DebugControl(DebugAction::Stop),
        };
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn callback_request_serializes_type_tag_and_id() {
        let msg = ServerMessage::CallbackRequest {
            id: 99,
            op: CallbackOp::RunTask {
                name: "build".into(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"callback_request""#), "json={json}");
        assert!(json.contains(r#""id":99"#), "json={json}");
    }
}

// endregion: --- Tests
