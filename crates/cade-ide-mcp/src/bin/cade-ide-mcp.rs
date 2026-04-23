//! `cade-ide-mcp` stdio entry point.
//!
//! Launches an [`IdeMcpServer`] over stdio so editor adapters (the
//! future VS Code extension, the JetBrains plugin, tests) can spawn
//! this binary as a subprocess and speak MCP over stdin/stdout.
//!
//! The adapter transport layer (the `EditorChannel` implementation that
//! drives the shared `EditorState` from editor events) lands in a later
//! milestone. For now the binary serves the tool surface against a
//! fresh, empty state with [`NullEditorChannel`], which exercises the
//! read-only tool path.
//!
//! Logging goes to **stderr only** — the MCP protocol owns stdout.

use cade_ide_mcp::{EditorState, IdeMcpServer};
use rmcp::ServiceExt;
use rmcp::transport::io::stdio;
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

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "cade-ide-mcp starting on stdio"
    );

    let server = IdeMcpServer::with_null_channel(EditorState::new());
    let running = server.serve(stdio()).await?;
    running.waiting().await?;

    Ok(())
}
