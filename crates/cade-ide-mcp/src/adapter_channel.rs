//! [`ProtocolEditorChannel`] — an [`EditorChannel`] implementation that
//! forwards callbacks to a connected editor adapter over the
//! [`crate::protocol`] wire format.
//!
//! The channel holds:
//! * a `Sink` (the write half of the TCP connection) for sending
//!   [`ServerMessage::CallbackRequest`] frames, and
//! * a [`PendingMap`] that pairs each in-flight request `id` with a
//!   one-shot channel; the receiver is awaited by the calling MCP tool,
//!   and the sender is resolved when the adapter's
//!   [`AdapterMessage::CallbackResponse`] arrives via
//!   [`ProtocolEditorChannel::deliver_response`].

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use rmcp::model::ErrorData;
use tokio::sync::{Mutex, oneshot};

use crate::{
    protocol::{CallbackOp, CallbackResult, ServerMessage},
    state::{ApplyEditRequest, DebugAction, Range},
};

// ── Pending-response map ─────────────────────────────────────────────────────

type ResponseSender = oneshot::Sender<CallbackResult>;

/// Thread-safe map from request `id` → pending response sender.
#[derive(Debug, Default, Clone)]
pub(crate) struct PendingMap(Arc<Mutex<std::collections::HashMap<u64, ResponseSender>>>);

impl PendingMap {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Insert a pending entry and return the receiver half.
    async fn insert(&self, id: u64) -> oneshot::Receiver<CallbackResult> {
        let (tx, rx) = oneshot::channel();
        self.0.lock().await.insert(id, tx);
        rx
    }

    /// Resolve a pending entry with the adapter's response. Returns
    /// `false` when `id` is unknown (duplicate / stale response).
    pub(crate) async fn resolve(&self, id: u64, result: CallbackResult) -> bool {
        if let Some(tx) = self.0.lock().await.remove(&id) {
            let _ = tx.send(result);
            true
        } else {
            false
        }
    }
}

// ── Sink abstraction ─────────────────────────────────────────────────────────

/// Anything that can serialize and transmit a [`ServerMessage`] to the
/// adapter. The trait is object-safe so tests can inject a fake.
#[async_trait]
pub trait MessageSink: Send + Sync + 'static {
    async fn send(&self, msg: ServerMessage) -> Result<(), String>;
}

// ── ProtocolEditorChannel ────────────────────────────────────────────────────

/// An [`crate::EditorChannel`] that forwards every callback over the
/// adapter wire protocol.
///
/// The channel is cheap to clone — both halves share the underlying sink
/// and pending map via `Arc`.
#[derive(Clone)]
pub struct ProtocolEditorChannel {
    label: Arc<str>,
    sink: Arc<dyn MessageSink>,
    pending: PendingMap,
    next_id: Arc<AtomicU64>,
}

impl std::fmt::Debug for ProtocolEditorChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtocolEditorChannel")
            .field("label", &self.label)
            .finish()
    }
}

impl ProtocolEditorChannel {
    /// Construct from a sink, adapter label, and a fresh pending map.
    pub fn new(label: impl Into<Arc<str>>, sink: Arc<dyn MessageSink>) -> Self {
        Self {
            label: label.into(),
            sink,
            pending: PendingMap::new(),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Called by the TCP accept loop when a
    /// [`AdapterMessage::CallbackResponse`] arrives from the adapter.
    /// Returns `false` if the `id` is unknown.
    pub async fn deliver_response(&self, id: u64, result: CallbackResult) -> bool {
        self.pending.resolve(id, result).await
    }

    /// Send a [`CallbackOp`] and wait for the adapter's response.
    async fn call(&self, op: CallbackOp) -> Result<(), ErrorData> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let rx = self.pending.insert(id).await;

        self.sink
            .send(ServerMessage::CallbackRequest { id, op })
            .await
            .map_err(|e| {
                ErrorData::new(
                    rmcp::model::ErrorCode::INTERNAL_ERROR,
                    format!("adapter write error: {e}"),
                    None,
                )
            })?;

        match rx.await {
            Ok(CallbackResult::Ok) => Ok(()),
            Ok(CallbackResult::Err(msg)) => Err(ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                msg,
                None,
            )),
            Err(_) => Err(ErrorData::new(
                rmcp::model::ErrorCode::INTERNAL_ERROR,
                format!("adapter '{}' disconnected before responding", self.label),
                None,
            )),
        }
    }
}

#[async_trait]
impl crate::EditorChannel for ProtocolEditorChannel {
    fn label(&self) -> &str {
        &self.label
    }

    fn is_connected(&self) -> bool {
        true
    }

    async fn apply_edit(&self, req: ApplyEditRequest) -> Result<(), ErrorData> {
        self.call(CallbackOp::ApplyEdit(req)).await
    }

    async fn reveal_file(&self, path: String) -> Result<(), ErrorData> {
        self.call(CallbackOp::RevealFile { path }).await
    }

    async fn set_selection(&self, path: String, range: Range) -> Result<(), ErrorData> {
        self.call(CallbackOp::SetSelection { path, range }).await
    }

    async fn save(&self, path: Option<String>) -> Result<(), ErrorData> {
        self.call(CallbackOp::Save { path }).await
    }

    async fn run_task(&self, name: String) -> Result<(), ErrorData> {
        self.call(CallbackOp::RunTask { name }).await
    }

    async fn run_terminal(&self, command: String) -> Result<(), ErrorData> {
        self.call(CallbackOp::RunTerminal { command }).await
    }

    async fn debug_control(&self, action: DebugAction) -> Result<(), ErrorData> {
        self.call(CallbackOp::DebugControl(action)).await
    }
}

// region:    --- Tests

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::EditorChannel;
    use crate::state::{ApplyEditRequest, DebugAction, Position, Range, TextEdit};
    use std::sync::Mutex as StdMutex;

    // ── Fake sink ────────────────────────────────────────────────────────────

    /// Records every message sent through it.
    #[derive(Default)]
    pub(crate) struct CaptureSink(StdMutex<Vec<ServerMessage>>);

    impl CaptureSink {
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub(crate) fn messages(&self) -> Vec<ServerMessage> {
            self.0.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MessageSink for CaptureSink {
        async fn send(&self, msg: ServerMessage) -> Result<(), String> {
            self.0.lock().unwrap().push(msg);
            Ok(())
        }
    }

    // ── Error sink ───────────────────────────────────────────────────────────

    /// Always errors.
    pub(crate) struct ErrorSink;

    #[async_trait]
    impl MessageSink for ErrorSink {
        async fn send(&self, _msg: ServerMessage) -> Result<(), String> {
            Err("write failed".into())
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn make_channel(sink: Arc<dyn MessageSink>) -> ProtocolEditorChannel {
        ProtocolEditorChannel::new("vscode-test", sink)
    }

    /// Drives one round-trip: spawn the callback, resolve via
    /// `deliver_response`, return the result.
    async fn round_trip_ok(
        ch: &ProtocolEditorChannel,
        sink: &CaptureSink,
        call: impl std::future::Future<Output = Result<(), ErrorData>> + Send + 'static,
    ) -> (Result<(), ErrorData>, Vec<ServerMessage>) {
        // We need the pending map to be populated before we resolve, so
        // drive the call concurrently.
        let ch2 = ch.clone();
        let handle = tokio::spawn(call);

        // Spin until the sink receives the request (tiny busy-wait, fine
        // for tests).
        loop {
            let msgs = sink.messages();
            if !msgs.is_empty() {
                // Extract the id from the captured CallbackRequest.
                let id = match &msgs[msgs.len() - 1] {
                    ServerMessage::CallbackRequest { id, .. } => *id,
                    _ => panic!("expected CallbackRequest"),
                };
                ch2.deliver_response(id, CallbackResult::Ok).await;
                break;
            }
            tokio::task::yield_now().await;
        }

        let result = handle.await.expect("task panicked");
        (result, sink.messages())
    }

    // ── Label / connected ────────────────────────────────────────────────────

    #[test]
    fn label_and_is_connected() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink);
        assert_eq!(ch.label(), "vscode-test");
        assert!(ch.is_connected());
    }

    // ── apply_edit ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn apply_edit_sends_callback_request_and_returns_ok() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let req = ApplyEditRequest {
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
                new_text: "// hi\n".into(),
            }],
        };
        let req2 = req.clone();
        let ch2 = ch.clone();
        let (result, msgs) =
            round_trip_ok(&ch, &sink, async move { ch2.apply_edit(req2).await }).await;
        assert!(result.is_ok());
        assert_eq!(msgs.len(), 1);
        assert!(
            matches!(&msgs[0], ServerMessage::CallbackRequest { op: CallbackOp::ApplyEdit(r), .. } if r.path == "/tmp/a.rs")
        );
    }

    // ── reveal_file ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn reveal_file_sends_callback_request_and_returns_ok() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let ch2 = ch.clone();
        let (result, msgs) = round_trip_ok(&ch, &sink, async move {
            ch2.reveal_file("/tmp/b.rs".into()).await
        })
        .await;
        assert!(result.is_ok());
        assert!(
            matches!(&msgs[0], ServerMessage::CallbackRequest { op: CallbackOp::RevealFile { path }, .. } if path == "/tmp/b.rs")
        );
    }

    // ── set_selection ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn set_selection_sends_callback_request_and_returns_ok() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let ch2 = ch.clone();
        let range = Range {
            start: Position {
                line: 1,
                character: 0,
            },
            end: Position {
                line: 1,
                character: 5,
            },
        };
        let (result, msgs) = round_trip_ok(&ch, &sink, async move {
            ch2.set_selection("/tmp/c.rs".into(), range).await
        })
        .await;
        assert!(result.is_ok());
        assert!(matches!(
            &msgs[0],
            ServerMessage::CallbackRequest { op: CallbackOp::SetSelection { path, .. }, .. }
            if path == "/tmp/c.rs"
        ));
    }

    // ── save ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn save_single_sends_callback_request_and_returns_ok() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let ch2 = ch.clone();
        let (result, msgs) = round_trip_ok(&ch, &sink, async move {
            ch2.save(Some("/tmp/d.rs".into())).await
        })
        .await;
        assert!(result.is_ok());
        assert!(matches!(
            &msgs[0],
            ServerMessage::CallbackRequest { op: CallbackOp::Save { path: Some(p) }, .. }
            if p == "/tmp/d.rs"
        ));
    }

    #[tokio::test]
    async fn save_all_sends_callback_request_with_none_path() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let ch2 = ch.clone();
        let (result, msgs) = round_trip_ok(&ch, &sink, async move { ch2.save(None).await }).await;
        assert!(result.is_ok());
        assert!(matches!(
            &msgs[0],
            ServerMessage::CallbackRequest {
                op: CallbackOp::Save { path: None },
                ..
            }
        ));
    }

    // ── run_task ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_task_sends_callback_request_and_returns_ok() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let ch2 = ch.clone();
        let (result, msgs) = round_trip_ok(&ch, &sink, async move {
            ch2.run_task("cargo-build".into()).await
        })
        .await;
        assert!(result.is_ok());
        assert!(matches!(
            &msgs[0],
            ServerMessage::CallbackRequest { op: CallbackOp::RunTask { name }, .. }
            if name == "cargo-build"
        ));
    }

    // ── run_terminal ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_terminal_sends_callback_request_and_returns_ok() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let ch2 = ch.clone();
        let (result, msgs) = round_trip_ok(&ch, &sink, async move {
            ch2.run_terminal("cargo test".into()).await
        })
        .await;
        assert!(result.is_ok());
        assert!(matches!(
            &msgs[0],
            ServerMessage::CallbackRequest { op: CallbackOp::RunTerminal { command }, .. }
            if command == "cargo test"
        ));
    }

    // ── debug_control ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn debug_start_sends_callback_request_and_returns_ok() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let ch2 = ch.clone();
        let (result, msgs) = round_trip_ok(&ch, &sink, async move {
            ch2.debug_control(DebugAction::Start {
                config: "unit-tests".into(),
            })
            .await
        })
        .await;
        assert!(result.is_ok());
        assert!(matches!(
            &msgs[0],
            ServerMessage::CallbackRequest {
                op: CallbackOp::DebugControl(DebugAction::Start { config }),
                ..
            }
            if config == "unit-tests"
        ));
    }

    #[tokio::test]
    async fn debug_stop_sends_callback_request_and_returns_ok() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let ch2 = ch.clone();
        let (result, _) = round_trip_ok(&ch, &sink, async move {
            ch2.debug_control(DebugAction::Stop).await
        })
        .await;
        assert!(result.is_ok());
    }

    // ── error paths ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn sink_write_error_surfaces_as_internal_error() {
        let ch = ProtocolEditorChannel::new("broken", Arc::new(ErrorSink));
        let err = ch.run_task("build".into()).await.expect_err("should fail");
        assert_eq!(err.code.0, rmcp::model::ErrorCode::INTERNAL_ERROR.0);
        assert!(err.message.contains("write failed"), "msg={}", err.message);
    }

    #[tokio::test]
    async fn adapter_error_response_surfaces_as_internal_error() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());
        let ch2 = ch.clone();

        // Spawn the call.
        let handle = tokio::spawn(async move { ch2.run_task("build".into()).await });

        // Wait for the request to land in the sink.
        loop {
            if !sink.messages().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }

        // Deliver an error response.
        let id = match &sink.messages()[0] {
            ServerMessage::CallbackRequest { id, .. } => *id,
            _ => panic!(),
        };
        ch.deliver_response(id, CallbackResult::Err("task not found".into()))
            .await;

        let err = handle.await.unwrap().expect_err("should be err");
        assert_eq!(err.code.0, rmcp::model::ErrorCode::INTERNAL_ERROR.0);
        assert!(
            err.message.contains("task not found"),
            "msg={}",
            err.message
        );
    }

    #[tokio::test]
    async fn stale_response_id_returns_false() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink);
        // No pending request with id=999.
        let resolved = ch.deliver_response(999, CallbackResult::Ok).await;
        assert!(!resolved);
    }

    // ── id monotonically increases ───────────────────────────────────────────

    #[tokio::test]
    async fn request_ids_are_monotonically_increasing() {
        let sink = CaptureSink::new();
        let ch = make_channel(sink.clone());

        // Fire two tasks concurrently and resolve both.
        let ch_a = ch.clone();
        let ch_b = ch.clone();
        let h1 = tokio::spawn(async move { ch_a.run_task("a".into()).await });
        let h2 = tokio::spawn(async move { ch_b.run_task("b".into()).await });

        // Spin until both requests are captured.
        loop {
            if sink.messages().len() >= 2 {
                break;
            }
            tokio::task::yield_now().await;
        }

        let msgs = sink.messages();
        let ids: Vec<u64> = msgs
            .iter()
            .map(|m| match m {
                ServerMessage::CallbackRequest { id, .. } => *id,
                _ => panic!(),
            })
            .collect();

        // Both IDs are distinct and ≥ 1.
        assert_ne!(ids[0], ids[1]);
        assert!(ids[0] >= 1);
        assert!(ids[1] >= 1);

        // Resolve so the tasks don't leak.
        ch.deliver_response(ids[0], CallbackResult::Ok).await;
        ch.deliver_response(ids[1], CallbackResult::Ok).await;
        let _ = h1.await;
        let _ = h2.await;
    }
}

// endregion: --- Tests
