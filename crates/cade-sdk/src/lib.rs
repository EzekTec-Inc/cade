// region:    --- Modules

pub mod client_sdk;
pub mod embedded;
mod error;
pub mod events;
#[cfg(not(target_arch = "wasm32"))]
pub mod ffi;
pub mod rpc;
pub mod session;
pub mod team;

pub use client_sdk::CadeClientSdk;
pub use embedded::{EmbeddedSession, EmbeddedSessionBuilder};
pub use error::{Error, Result};
pub use events::CadeStreamEvent;
pub use session::{AgentSession, SessionOptions};
pub use team::{TeamSession, TeamSessionBuilder};

// endregion: --- Modules
