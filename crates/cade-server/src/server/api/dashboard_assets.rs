//! Embedded WASM dashboard assets produced by `trunk build` in `cade-gui`.
//!
//! In **release** builds the files are baked into the binary at compile time.
//! In **debug** builds `rust-embed` reads them from the filesystem, so you
//! can iterate on the GUI without recompiling cade-server.
//!
//! The `folder` path is relative to the cade-server crate root (where its
//! `Cargo.toml` lives).

use rust_embed::Embed;

/// All files under `crates/cade-gui/dist/` (built by `trunk build`).
///
/// The `allow_missing` attribute lets `cargo build -p cade-server` succeed
/// even when the `dist/` directory is empty (i.e. trunk has not been run).
/// At runtime, requests for missing assets simply return 404.
#[derive(Embed)]
#[folder = "../cade-gui/dist/"]
#[allow_missing = "true"]
pub struct DashboardAssets;
