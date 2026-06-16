#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::items_after_test_module)]
// region:    --- Modules

mod error;

pub use error::{Error, Result};

#[cfg(not(target_arch = "wasm32"))]
pub mod agent_env;
#[cfg(not(target_arch = "wasm32"))]
pub mod askpass;
#[cfg(not(target_arch = "wasm32"))]
pub mod bootstrap_token;
#[cfg(not(target_arch = "wasm32"))]
pub mod capabilities;
#[cfg(not(target_arch = "wasm32"))]
pub mod hooks;
#[cfg(not(target_arch = "wasm32"))]
pub mod permissions;
pub mod resources;
#[cfg(not(target_arch = "wasm32"))]
pub mod settings;
#[cfg(not(target_arch = "wasm32"))]
pub mod shell;
#[cfg(not(target_arch = "wasm32"))]
pub mod skills;
pub mod structured_patch;
#[cfg(not(target_arch = "wasm32"))]
pub mod tool_ids;
#[cfg(not(target_arch = "wasm32"))]
pub mod toolsets;

// endregion: --- Modules
