// region:    --- Modules

mod error;
pub mod manifest;
pub mod marketplace;
pub mod engine;
pub mod registry;

pub use engine::{MockPluginEngine, NativePluginEngine, PluginEngine, PluginReport};
pub use error::{Error, Result};
pub use manifest::PluginManifest;
pub use registry::PluginRegistry;

// endregion: --- Modules
