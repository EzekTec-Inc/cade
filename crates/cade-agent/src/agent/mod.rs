// region:    --- Modules

pub mod client;
pub mod session;
pub mod tools;
pub mod stagnation;

pub use client::HttpTransport;
pub use stagnation::{DoomLoopDetector, StagnationResult};

// endregion: --- Modules
