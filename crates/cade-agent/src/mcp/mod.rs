/// Re-export the standalone `cade-mcp` crate so existing `crate::mcp::*` paths
/// throughout `cade-agent` and downstream crates resolve unchanged.
// region:    --- Modules

pub use cade_mcp::*;

// endregion: --- Modules
