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

    // Pre-compute the discovery file path for signal-based cleanup.
    let pid = std::process::id();
    let discovery_file = cade_ide_mcp::transport::discovery_path(pid);

    let state = EditorState::new();
    let slot = ChannelSlot::null();

    // Spawn the adapter accept loop (M-IDE-1c transport).
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let mut accept_handle = {
        let s = state.clone();
        let sl = slot.clone();
        tokio::spawn(async move {
            if let Err(e) = cade_ide_mcp::transport::run_accept_loop(s, sl, shutdown_rx, None).await
            {
                tracing::error!("adapter accept loop failed: {e}");
            }
        })
    };

    // Build the MCP server backed by the live channel slot.
    let server = IdeMcpServer::with_channel_slot(state, slot);
    let running = server.serve(stdio()).await?;

    // Wait for any of: MCP stdio close, accept-loop panic, or signal.
    tokio::select! {
        biased;

        // Ctrl-C / SIGINT.
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received SIGINT — shutting down");
        }

        // Unix SIGTERM (covers `kill <pid>` and container stop).
        _ = async {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{SignalKind, signal};
                let mut sigterm = signal(SignalKind::terminate())
                    .expect("failed to register SIGTERM handler");
                sigterm.recv().await;
            }
            #[cfg(not(unix))]
            {
                // On non-unix, fall back to ctrl_c (already handled above).
                std::future::pending::<()>().await;
            }
        } => {
            tracing::info!("received SIGTERM — shutting down");
        }

        // Accept loop panicked or returned early.
        result = &mut accept_handle => {
            match result {
                Ok(()) => tracing::info!("accept loop exited cleanly"),
                Err(e) => tracing::error!("accept loop panicked: {e}"),
            }
        }

        // Normal exit: MCP stdio transport closed.
        result = running.waiting() => {
            if let Err(e) = result {
                tracing::error!("MCP stdio transport error: {e}");
            }
        }
    }

    // Signal the accept loop to shut down (no-op if already exited).
    let _ = shutdown_tx.send(());

    // Clean up discovery file — covers signal exits where the accept
    // loop's own cleanup never ran.
    if let Some(ref path) = discovery_file {
        cade_ide_mcp::transport::remove_discovery(path);
        tracing::info!(path = %path.display(), "discovery file cleaned up");
    }

    Ok(())
}
