// region:    --- Modules

mod error;
pub mod manifest;
pub mod registry;

pub use error::{Error, Result};
pub use manifest::PluginManifest;
pub use registry::PluginRegistry;

// endregion: --- Modules
