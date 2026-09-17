// region:    --- Modules

pub mod dev;
pub mod engine;
mod error;
pub mod manifest;
pub mod marketplace;
pub mod registry;

pub use dev::{init_plugin, pack_plugin, validate_plugin, PackedPlugin, PluginValidationReport};
pub use engine::{MockPluginEngine, NativePluginEngine, PluginEngine, PluginReport};
pub use error::{Error, Result};
pub use manifest::PluginManifest;
pub use registry::PluginRegistry;

// endregion: --- Modules

#[cfg(test)]
mod tests;
