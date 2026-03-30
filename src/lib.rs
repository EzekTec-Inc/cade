// region:    --- Modules

mod error;

pub use error::{Error, Result};

/// Default memory block labels, tiers, and limits — shared between `main.rs`
/// (which seeds agents) and regression tests.
/// (label, initial_value, description, max_chars, tier)
pub const DEFAULT_MEMORY_BLOCKS: &[(&str, &str, &str, usize, &str)] = &[
    (
        "persona",
        "",
        "Who I am, what I value, and how I approach working with people",
        2_000,
        "pinned",
    ),
    (
        "human",
        "",
        "What I know about the person I'm working with — their name, preferences, and working style",
        3_000,
        "pinned",
    ),
    (
        "project",
        "",
        "Current project context, tech stack, conventions, and ongoing work",
        5_000,
        "pinned",
    ),
    (
        "working_set",
        "",
        "Active task, files currently being edited, recent changes, and immediate next steps.",
        3_000,
        "short",
    ),
];

// Re-export workspace crates so existing `cade::*` paths in binaries still work.
pub use cade_core::hooks;
pub use cade_core::permissions;
pub use cade_core::settings;
pub use cade_core::skills;
pub use cade_core::toolsets;

#[cfg(feature = "desktop")]
pub use cade_desktop::desktop;

pub use cade_server::server;

pub use cade_agent::agent;
pub use cade_agent::mcp;
pub use cade_agent::subagents;
pub use cade_agent::tools;

pub use cade_cli::cli;
pub use cade_cli::support;
pub use cade_cli::ui;

// endregion: --- Modules
