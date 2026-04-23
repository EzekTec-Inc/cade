//! CADE IDE MCP bridge.
//!
//! Exposes a connected editor's state (open files, selection, diagnostics,
//! workspace folders, …) to CADE agents over the Model Context Protocol.
//!
//! This is Phase M-IDE-1a of the IDE integration milestone — scaffold
//! and read-only state access. No MCP transport, no editor adapter, no
//! tools yet; later phases add those in separate commits.

// region:    --- Modules

mod adapter_channel;
mod channel;
pub mod protocol;
mod server;
mod state;

pub use adapter_channel::{MessageSink, ProtocolEditorChannel};
pub use channel::{EditorChannel, NullEditorChannel};
pub use server::IdeMcpServer;
pub use state::EditorState;

// endregion: --- Modules
