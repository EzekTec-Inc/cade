//! `cade-askpass` library — protocol primitives and (under the
//! `server` feature) the tokio-based IPC server that hosts the
//! password channel.
//!
//! The binary at `src/main.rs` consumes only [`encode_line`] /
//! [`decode_line`] and stays std-only so the helper executable
//! remains small and dependency-free.  `cade-agent` enables the
//! `server` feature to run the listener inside its tokio runtime.

pub mod protocol;

#[cfg(feature = "server")]
pub mod server;

pub use protocol::{decode_line, encode_line};

/// Environment variable that carries the loopback socket address
/// (e.g. `127.0.0.1:38271`) from the agent's IPC server to the
/// askpass binary spawned by `sudo -A` / `ssh` / `git`.
pub const ENV_SOCKET: &str = "CADE_ASKPASS_SOCKET";

/// Environment variable that carries the per-session auth token.
/// The askpass binary reads this and prefixes the `PROMPT` line with
/// it; the server rejects any connection that does not present the
/// matching token.  Stored as a hex-encoded random 32-byte string.
pub const ENV_TOKEN: &str = "CADE_ASKPASS_TOKEN";
