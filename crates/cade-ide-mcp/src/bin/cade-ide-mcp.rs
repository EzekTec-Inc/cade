//! `cade-ide-mcp` stdio entry point.
//!
//! Launches an [`IdeMcpServer`] over stdio so CADE agents (and any other
//! MCP-capable client) can introspect and drive the connected editor.
//!
//! ## Startup sequence
//!
//! 1. Bind an ephemeral TCP port on `127.0.0.1` and write the discovery
//!    file to `~/.cade/ide/<pid>.json`.
//! 2. Spawn the adapter accept loop (M-IDE-1c transport).
//! 3. Serve the MCP stdio transport; tools read state and route callbacks
//!    through the live [`ChannelSlot`] — updated whenever an editor
//!    adapter connects or disconnects.
//!
//! Logging goes to **stderr only** — the MCP protocol owns stdout.

use cade_ide_mcp::{ChannelSlot, EditorState, IdeMcpServer};
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;
use tokio::sync::watch;
use tracing_subscriber::{EnvFilter, fmt};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    // Structured logging to stderr so stdout stays protocol-clean.
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "cade-ide-mcp starting");

    let state = EditorState::new();
    let slot = ChannelSlot::null();

    // Spawn the adapter accept loop (M-IDE-1c transport).
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    {
        let s = state.clone();
        let sl = slot.clone();
        tokio::spawn(async move {
            if let Err(e) = cade_ide_mcp::transport::run_accept_loop(s, sl, shutdown_rx, None).await
            {
                tracing::error!("adapter accept loop failed: {e}");
            }
        });
    }

    // Build the MCP server backed by the live channel slot.
    let server = IdeMcpServer::with_channel_slot(state, slot);
    let running = server.serve(stdio()).await?;
    running.waiting().await?;

    // Signal the accept loop to shut down.
    let _ = shutdown_tx.send(());
    Ok(())
}
