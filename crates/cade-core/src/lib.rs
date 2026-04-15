#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::items_after_test_module)]
// region:    --- Modules

mod error;

pub use error::{Error, Result};

pub mod agent_env;
pub mod capabilities;
pub mod hooks;
pub mod permissions;
pub mod resources;
pub mod settings;
pub mod shell;
pub mod skills;
pub mod tool_ids;
pub mod toolsets;

// endregion: --- Modules
