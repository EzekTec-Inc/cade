// region:    --- Modules

pub mod client;
pub mod session;
pub mod stagnation;
pub mod tools;

pub use client::HttpTransport;
pub use stagnation::{DoomLoopDetector, StagnationResult};

// endregion: --- Modules
