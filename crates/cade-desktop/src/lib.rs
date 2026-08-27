// region:    --- Modules

mod error;

pub use error::{Error, Result};

pub mod desktop;

pub use desktop::commander::*;
pub use desktop::tray::*;

// endregion: --- Modules
