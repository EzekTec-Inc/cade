#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::items_after_test_module)]
// region:    --- Modules

mod error;

pub use error::{Error, Result};

pub mod agent_env;
pub mod hooks;
pub mod permissions;
pub mod settings;
pub mod skills;
pub mod toolsets;

// endregion: --- Modules
