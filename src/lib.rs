// Re-export workspace crates so existing `cade::*` paths in binaries still work.
pub use cade_core::permissions;
pub use cade_core::settings;
pub use cade_core::toolsets;
pub use cade_core::skills;
pub use cade_core::hooks;

pub use cade_desktop::desktop;

pub use cade_server::server;

pub use cade_agent::agent;
pub use cade_agent::tools;
pub use cade_agent::subagents;
pub use cade_agent::mcp;

pub use cade_cli::cli;
pub use cade_cli::ui;
