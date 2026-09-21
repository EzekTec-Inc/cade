//! Embedded WASM dashboard assets produced by `trunk build` in `cade-gui`.
//!
//! In **release** builds the files are baked into the binary at compile time.
//! In **debug** builds `rust-embed` reads them from the filesystem, so you
//! can iterate on the GUI without recompiling cade-server.
//!
//! The `folder` path is relative to the cade-server crate root (where its
//! `Cargo.toml` lives).

use rust_embed::Embed;
use std::borrow::Cow;

/// Embedded GUI assets compiled from `crates/cade-gui/dist/`.
#[derive(Embed)]
#[folder = "../cade-gui/dist/"]
#[allow_missing = "true"]
struct RawDistAssets;

const FALLBACK_INDEX_HTML: &[u8] = include_bytes!("../../../../cade-gui/dist/index.html");

/// An embedded dashboard asset containing the asset's binary content.
pub struct DashboardAsset {
    pub data: Cow<'static, [u8]>,
}

/// Deep module providing access to static GUI dashboard assets.
///
/// If `crates/cade-gui/dist/` has not been compiled yet (or is missing in CI),
/// this module guarantees the invariant that at least a fallback `index.html`
/// exposing `#cade_gui_canvas` is served and discoverable via `iter()`.
pub struct DashboardAssets;

impl DashboardAssets {
    /// Retrieve an asset by relative path.
    ///
    /// If `CADE_DEV=1` or `CADE_DASHBOARD_DIR` is set in the environment,
    /// dynamic loading from the filesystem is checked first. This enables
    /// rapid GUI iteration without recompiling the release server binary.
    pub fn get(file_path: &str) -> Option<DashboardAsset> {
        Self::get_with_base_dir(file_path, None)
    }

    /// Retrieve an asset by relative path, optionally checking a custom base directory first.
    pub fn get_with_base_dir(
        file_path: &str,
        override_dir: Option<&std::path::Path>,
    ) -> Option<DashboardAsset> {
        // 1. Check explicit override or development-mode filesystem overrides
        let dev_enabled = override_dir.is_some()
            || std::env::var("CADE_DEV")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false)
            || std::env::var("CADE_DASHBOARD_DIR").is_ok();

        if dev_enabled {
            let base_dir = if let Some(dir) = override_dir {
                dir.to_path_buf()
            } else if let Ok(dir) = std::env::var("CADE_DASHBOARD_DIR") {
                std::path::PathBuf::from(dir)
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .join("crates/cade-gui/dist")
            };

            let candidate = base_dir.join(file_path);
            if candidate.is_file()
                && let Ok(bytes) = std::fs::read(&candidate)
            {
                return Some(DashboardAsset {
                    data: Cow::Owned(bytes),
                });
            }
        }

        // 2. Embedded assets compiled into binary via rust-embed
        if let Some(file) = RawDistAssets::get(file_path) {
            return Some(DashboardAsset { data: file.data });
        }

        // 3. Fallback for index.html when dist/ is missing or empty
        if file_path == "index.html" {
            return Some(DashboardAsset {
                data: Cow::Borrowed(FALLBACK_INDEX_HTML),
            });
        }

        None
    }

    /// Iterate over all available dashboard asset paths.
    pub fn iter() -> impl Iterator<Item = Cow<'static, str>> {
        let dist_iter = RawDistAssets::iter();
        let has_index = RawDistAssets::get("index.html").is_some();
        let fallback = if has_index {
            None
        } else {
            Some(Cow::Borrowed("index.html"))
        };

        dist_iter.chain(fallback)
    }
}
