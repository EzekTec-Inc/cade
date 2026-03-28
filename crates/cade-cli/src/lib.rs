#![allow(clippy::await_holding_lock)]
#![allow(clippy::too_many_arguments)]
// region:    --- Modules

mod error;

pub use error::{Error, Result};

pub mod cli;
pub mod ui;
pub mod support;

// endregion: --- Modules
