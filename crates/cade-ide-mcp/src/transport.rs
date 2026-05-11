//! TCP loopback transport for the adapter ↔ `cade-ide-mcp` protocol.
//!
//! # What lives here
//!
//! * [`TcpSink`] — a [`MessageSink`] that serialises [`ServerMessage`]
//!   frames as newline-delimited JSON and writes them over the write
//!   half of a [`tokio::net::TcpStream`].
//!
//! * [`run_accept_loop`] — binds an ephemeral TCP port on `127.0.0.1`,
//!   writes a discovery file to `~/.cade/ide/<pid>.json` so the VS Code
//!   extension knows which port to connect on, then accepts **one**
//!   adapter connection. For each connection it:
//!   1. Reads the adapter's `Hello`, sends `HelloAck`.
//!   2. Installs a [`ProtocolEditorChannel`] as the live channel.
//!   3. Reads subsequent frames:
//!      - `StateUpdate` → applies the snapshot to [`EditorState`].
//!      - `CallbackResponse` → delivers it to the pending-map.
//!   4. On disconnect, reverts to [`NullEditorChannel`].
//!
//! # Discovery file
//!
//! ```json
//! { "pid": 12345, "addr": "127.0.0.1:54321" }
//! ```
//!
//! Written to `~/.cade/ide/<pid>.json` on bind, removed on clean exit.
//! The VS Code extension reads this file to know which port to connect on.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::watch,
};

use crate::{
    EditorChannel, EditorState, NullEditorChannel, ProtocolEditorChannel,
    adapter_channel::MessageSink,
    protocol::{AdapterMessage, ServerMessage, StateSnapshot},
};

// ── TcpSink ──────────────────────────────────────────────────────────────────

/// A [`MessageSink`] backed by the write half of a [`TcpStream`].
///
/// Each call to [`send`] serialises the message as a single JSON line
/// (no embedded newlines) and appends `\n`.
pub struct TcpSink {
    writer: tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>,
}

impl TcpSink {
    pub fn new(write_half: tokio::net::tcp::OwnedWriteHalf) -> Arc<Self> {
        Arc::new(Self {
            writer: tokio::sync::Mutex::new(write_half),
        })
    }
}

#[async_trait]
impl MessageSink for TcpSink {
    async fn send(&self, msg: ServerMessage) -> Result<(), String> {
        let mut line = serde_json::to_string(&msg).map_err(|e| format!("serialize error: {e}"))?;
        line.push('\n');
        self.writer
            .lock()
            .await
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("write error: {e}"))
    }
}

// ── Discovery file ────────────────────────────────────────────────────────────

/// Shape of the discovery JSON written to disk.
#[derive(Serialize)]
struct DiscoveryInfo {
    pid: u32,
    addr: String,
}

/// Returns `~/.cade/ide/<pid>.json`.
fn discovery_path(pid: u32) -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())?;
    let dir = PathBuf::from(home).join(".cade").join("ide");
    Some(dir.join(format!("{pid}.json")))
}

/// Write the discovery file. Creates parent dirs if needed.
fn write_discovery(addr: SocketAddr) -> Option<PathBuf> {
    let pid = std::process::id();
    let path = discovery_path(pid)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let info = DiscoveryInfo {
        pid,
        addr: addr.to_string(),
    };
    let json = serde_json::to_string_pretty(&info).ok()?;
    std::fs::write(&path, json).ok()?;
    Some(path)
}

fn remove_discovery(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
}

// ── Channel-slot (Arc<dyn EditorChannel>) ────────────────────────────────────

/// A shared, swappable channel slot. The MCP server holds one clone and
/// reads the current channel; the accept loop writes a new channel when
/// an adapter connects, and reverts to `NullEditorChannel` on disconnect.
#[derive(Clone)]
pub struct ChannelSlot(Arc<tokio::sync::RwLock<Arc<dyn EditorChannel>>>);

impl ChannelSlot {
    pub fn null() -> Self {
        Self(Arc::new(tokio::sync::RwLock::new(
            Arc::new(NullEditorChannel) as Arc<dyn EditorChannel>,
        )))
    }

    pub async fn set(&self, ch: Arc<dyn EditorChannel>) {
        *self.0.write().await = ch;
    }

    pub async fn get(&self) -> Arc<dyn EditorChannel> {
        self.0.read().await.clone()
    }
}

// ── State-update application ─────────────────────────────────────────────────

async fn apply_snapshot(state: &EditorState, snap: StateSnapshot) {
    state.replace_open_files(snap.open_files).await;
    state.set_active_file(snap.active_file).await;
    state.set_selection(snap.selection).await;
    state.replace_diagnostics(snap.diagnostics).await;
    state
        .replace_workspace_folders(snap.workspace_folders)
        .await;
    state.set_visible_range(snap.visible_range).await;
}

// ── Per-connection read loop ──────────────────────────────────────────────────

async fn handle_connection(stream: TcpStream, state: EditorState, slot: ChannelSlot) {
    let peer = stream.peer_addr().ok();
    tracing::info!(peer = ?peer, "adapter connected");

    let (read_half, write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    // ── Step 1: expect Hello ────────────────────────────────────────────────
    let first_line = match lines.next_line().await {
        Ok(Some(l)) => l,
        _ => {
            tracing::warn!("adapter disconnected before Hello");
            return;
        }
    };
    let hello: AdapterMessage = match serde_json::from_str(&first_line) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("malformed Hello frame: {e}");
            return;
        }
    };
    let label = match hello {
        AdapterMessage::Hello {
            label,
            protocol_version,
        } => {
            tracing::info!(label, protocol_version, "received Hello");
            label
        }
        other => {
            tracing::warn!(?other, "expected Hello, got something else");
            return;
        }
    };

    // ── Step 2: build channel + send HelloAck ───────────────────────────────
    let sink = TcpSink::new(write_half);
    let channel = ProtocolEditorChannel::new(label.as_str(), sink.clone());

    // Send HelloAck before installing the channel.
    if let Err(e) = sink
        .send(ServerMessage::HelloAck {
            protocol_version: 1,
        })
        .await
    {
        tracing::warn!("failed to send HelloAck: {e}");
        return;
    }

    slot.set(Arc::new(channel.clone())).await;
    tracing::info!("adapter '{}' installed as active channel", label);

    // ── Step 3: read loop ───────────────────────────────────────────────────
    while let Ok(Some(line)) = lines.next_line().await {
        let msg: AdapterMessage = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(line = %line, "malformed frame from adapter: {e}");
                continue;
            }
        };
        match msg {
            AdapterMessage::StateUpdate(snap) => {
                apply_snapshot(&state, snap).await;
            }
            AdapterMessage::CallbackResponse { id, result } => {
                let resolved = channel.deliver_response(id, result).await;
                if !resolved {
                    tracing::warn!(id, "received response for unknown request id");
                }
            }
            AdapterMessage::Hello { .. } => {
                tracing::warn!("received duplicate Hello — ignoring");
            }
        }
    }

    // ── Step 4: revert to null channel on disconnect ────────────────────────
    tracing::info!(
        "adapter '{}' disconnected — reverting to NullEditorChannel",
        label
    );
    slot.set(Arc::new(NullEditorChannel)).await;
}

// ── Public accept loop ────────────────────────────────────────────────────────

/// Bind an ephemeral TCP port on `127.0.0.1`, write the discovery file,
/// and loop accepting adapter connections (one at a time).
///
/// `shutdown` is a watch channel; send any value to trigger a clean exit.
///
/// If `addr_tx` is `Some`, the bound [`SocketAddr`] is sent on it before
/// the first `accept` call — useful in tests to avoid polling the
/// discovery file.
///
/// Returns an error if the bind fails. Discovery-file failures are logged
/// as warnings but do not abort.
pub async fn run_accept_loop(
    state: EditorState,
    slot: ChannelSlot,
    mut shutdown: watch::Receiver<()>,
    addr_tx: Option<tokio::sync::oneshot::Sender<SocketAddr>>,
) -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, "adapter transport listening");

    let discovery = write_discovery(addr);
    match &discovery {
        Some(p) => tracing::info!(path = %p.display(), "discovery file written"),
        None => tracing::warn!("could not write discovery file"),
    }

    // Notify caller of the bound address (used in tests).
    if let Some(tx) = addr_tx {
        let _ = tx.send(addr);
    }

    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => {
                tracing::info!("accept loop shutting down");
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let s = state.clone();
                        let sl = slot.clone();
                        tokio::spawn(handle_connection(stream, s, sl));
                    }
                    Err(e) => tracing::warn!("accept error: {e}"),
                }
            }
        }
    }

    if let Some(ref p) = discovery {
        remove_discovery(p);
        tracing::info!("discovery file removed");
    }

    Ok(())
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::{AdapterMessage, CallbackOp, CallbackResult, ServerMessage},
        state::OpenFile,
    };
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    // ── TcpSink ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn tcp_sink_writes_newline_delimited_json() {
        // Bind a listener, connect a client, capture what we send.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = TcpStream::connect(addr).await.unwrap();
        let (server_stream, _) = listener.accept().await.unwrap();
        let (server_read, server_write) = server_stream.into_split();

        let sink = TcpSink::new(server_write);
        sink.send(ServerMessage::HelloAck {
            protocol_version: 1,
        })
        .await
        .unwrap();

        // Read one line from the client side.
        let mut reader = BufReader::new(client);
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();

        let msg: ServerMessage = serde_json::from_str(line.trim()).unwrap();
        assert!(matches!(
            msg,
            ServerMessage::HelloAck {
                protocol_version: 1
            }
        ));
        drop(server_read); // silence unused warning
    }

    // ── discovery_path ────────────────────────────────────────────────────────

    #[test]
    fn discovery_path_contains_pid_and_json_extension() {
        let pid = std::process::id();
        if let Some(p) = discovery_path(pid) {
            let name = p.file_name().unwrap().to_string_lossy();
            assert!(name.contains(&pid.to_string()), "name={name}");
            assert!(name.ends_with(".json"), "name={name}");
        }
        // If HOME is unset the function returns None — that's acceptable.
    }

    // ── ChannelSlot ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn channel_slot_starts_as_null_and_can_be_replaced() {
        let slot = ChannelSlot::null();
        assert_eq!(slot.get().await.label(), "null");
        assert!(!slot.get().await.is_connected());
    }

    // ── apply_snapshot ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn apply_snapshot_populates_editor_state() {
        let state = EditorState::new();
        let snap = StateSnapshot {
            open_files: vec![OpenFile {
                path: Some("/tmp/a.rs".into()),
                text: "fn main() {}\n".into(),
                language_id: "rust".into(),
                version: 1,
                is_dirty: false,
            }],
            active_file: Some("/tmp/a.rs".into()),
            selection: None,
            diagnostics: vec![],
            workspace_folders: vec![],
            visible_range: Some((0, 20)),
        };
        apply_snapshot(&state, snap).await;
        assert_eq!(state.open_file_count().await, 1);
        assert_eq!(state.active_file().await.as_deref(), Some("/tmp/a.rs"));
        assert_eq!(state.visible_range().await, Some((0, 20)));
    }

    // ── full handshake (integration) ──────────────────────────────────────────

    /// Drives an entire Hello → HelloAck → StateUpdate round-trip over a
    /// real in-process TCP connection using the accept loop.
    #[tokio::test]
    async fn accept_loop_handshake_and_state_update() {
        let state = EditorState::new();
        let slot = ChannelSlot::null();
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let (addr_tx, addr_rx) = tokio::sync::oneshot::channel();

        // Spawn the accept loop.
        let s = state.clone();
        let sl = slot.clone();
        let loop_handle = tokio::spawn(run_accept_loop(s, sl, shutdown_rx, Some(addr_tx)));

        // Wait for the bound address.
        let addr = addr_rx.await.expect("addr");

        // Connect as an adapter.
        let stream = TcpStream::connect(addr).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half).lines();

        // Send Hello.
        let hello = serde_json::to_string(&AdapterMessage::Hello {
            label: "test-adapter".into(),
            protocol_version: 1,
        })
        .unwrap()
            + "\n";
        write_half.write_all(hello.as_bytes()).await.unwrap();

        // Expect HelloAck.
        let ack_line = reader.next_line().await.unwrap().unwrap();
        let ack: ServerMessage = serde_json::from_str(&ack_line).unwrap();
        assert!(matches!(
            ack,
            ServerMessage::HelloAck {
                protocol_version: 1
            }
        ));

        // Send a StateUpdate.
        let update = serde_json::to_string(&AdapterMessage::StateUpdate(StateSnapshot {
            open_files: vec![OpenFile {
                path: Some("/tmp/hello.rs".into()),
                text: "fn main() {}\n".into(),
                language_id: "rust".into(),
                version: 1,
                is_dirty: false,
            }],
            active_file: Some("/tmp/hello.rs".into()),
            selection: None,
            diagnostics: vec![],
            workspace_folders: vec![],
            visible_range: None,
        }))
        .unwrap()
            + "\n";
        write_half.write_all(update.as_bytes()).await.unwrap();

        // Give the accept loop time to process the state update.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        assert_eq!(state.open_file_count().await, 1);
        assert_eq!(state.active_file().await.as_deref(), Some("/tmp/hello.rs"));

        // Verify the slot now holds the real channel.
        assert_eq!(slot.get().await.label(), "test-adapter");
        assert!(slot.get().await.is_connected());

        // Shut down the accept loop.
        let _ = shutdown_tx.send(());
        let _ = loop_handle.await;
    }

    // ── callback round-trip ───────────────────────────────────────────────────

    #[tokio::test]
    async fn callback_round_trip_over_tcp() {
        let state = EditorState::new();
        let slot = ChannelSlot::null();
        let (shutdown_tx, shutdown_rx) = watch::channel(());
        let (addr_tx, addr_rx) = tokio::sync::oneshot::channel();

        let s = state.clone();
        let sl = slot.clone();
        let _loop = tokio::spawn(run_accept_loop(s, sl, shutdown_rx, Some(addr_tx)));

        let addr = addr_rx.await.expect("addr");

        let stream = TcpStream::connect(addr).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half).lines();

        // Handshake.
        write_half
            .write_all(
                (serde_json::to_string(&AdapterMessage::Hello {
                    label: "cb-test".into(),
                    protocol_version: 1,
                })
                .unwrap()
                    + "\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let _ = reader.next_line().await.unwrap(); // consume HelloAck

        // Wait for the slot to be populated.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        // Grab the channel from the slot and fire a callback concurrently.
        let channel = slot.get().await;
        let ch_clone = channel.clone();

        let call_handle =
            tokio::spawn(async move { ch_clone.run_task("cargo-build".into()).await });

        // Adapter side: read the CallbackRequest, send CallbackResponse.
        let req_line = reader.next_line().await.unwrap().unwrap();
        let req: ServerMessage = serde_json::from_str(&req_line).unwrap();
        let id = match req {
            ServerMessage::CallbackRequest {
                id,
                op: CallbackOp::RunTask { ref name },
                ..
            } if name == "cargo-build" => id,
            other => panic!("unexpected: {other:?}"),
        };

        let resp = serde_json::to_string(&AdapterMessage::CallbackResponse {
            id,
            result: CallbackResult::Ok,
        })
        .unwrap()
            + "\n";
        write_half.write_all(resp.as_bytes()).await.unwrap();

        // The tool call should complete successfully.
        let result = call_handle.await.unwrap();
        assert!(result.is_ok(), "callback should succeed: {result:?}");

        let _ = shutdown_tx.send(());
    }
}

// endregion: --- Tests
