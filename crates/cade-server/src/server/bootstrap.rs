//! Server bootstrap utilities.
//!
//! Thin re-export of [`cade_core::bootstrap_token`] so the server code can
//! continue to refer to `crate::server::bootstrap::*`.  The actual helpers
//! live in `cade-core` to avoid a cyclic dep when the CLI needs the same
//! token path.

pub use cade_core::bootstrap_token::{
    default_token_path, load_or_create_token, read_existing_token,
};
