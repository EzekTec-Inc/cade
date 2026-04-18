//! Build script for `cade-server`.
//!
//! Its sole job is to tell Cargo to rerun this crate's compilation whenever
//! any file under `crates/cade-gui/dist/` changes. Those files are embedded
//! into the binary via `rust-embed` in `src/server/api/dashboard_assets.rs`.
//!
//! `rust-embed` 8.x's `#[folder = "..."]` attribute does **not** emit
//! `cargo:rerun-if-changed` directives for the files it embeds, which means
//! Cargo considers the crate "Fresh" even when the dashboard WASM bundle is
//! rebuilt by `trunk`. Without this build script, a developer has to manually
//! `touch` a source file in `cade-server` (or `cargo clean`) after every
//! `trunk build`, which is easy to forget and leads to the symptom of
//! "I don't see the new dashboard" when the old bundle is still baked in.
//!
//! We emit `rerun-if-changed` for the dist directory itself *and* for each
//! top-level file inside it — Cargo watches directory mtimes for additions
//! and removals, and per-file entries catch in-place modifications of the
//! hashed WASM/JS outputs.
//!
//! If the directory does not exist yet (fresh checkout, trunk never run),
//! we still emit the directory watch so Cargo will re-invoke us once it
//! appears.

use std::path::{Path, PathBuf};

fn main() {
    // Path is relative to this build script (which lives at
    // `crates/cade-server/build.rs`). The cade-gui dist dir is a sibling
    // crate's output folder.
    let dist_dir: PathBuf = Path::new("..").join("cade-gui").join("dist");

    // Always watch the directory itself. Cargo interprets a rerun-if-changed
    // on a directory as "rerun if the directory's mtime changes", which
    // covers file creation and deletion inside it (e.g. when trunk writes
    // a new hashed bundle name).
    println!("cargo:rerun-if-changed={}", dist_dir.display());

    // Also watch each individual file so in-place modifications (same
    // filename, new content — possible if the hash happens to stay stable)
    // trigger a rebuild.
    if let Ok(entries) = std::fs::read_dir(&dist_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    // Watch this build script itself (Cargo does this by default, but being
    // explicit removes ambiguity if someone edits it).
    println!("cargo:rerun-if-changed=build.rs");
}
