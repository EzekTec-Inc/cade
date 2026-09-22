//! `/dashboard` — serves the WASM `cade-gui` client and its assets.
//!
//! Security contract:
//! - These routes are **exempt from `auth_middleware`** (see `auth.rs`), so the
//!   browser can fetch the page and assets without a bearer token.
//! - The served HTML **never** embeds the server's `api_key`. The user pastes
//!   their key into the egui login form; the WASM app holds it in memory
//!   only. This keeps the auth boundary intact against drive-by GETs.
//! - GET is a "safe method" per RFC 9110 §9.2.1, so the CSRF middleware does
//!   not interfere.
//!
//! Assets are embedded at compile time by `rust-embed` from the `cade-gui/dist/`
//! directory (built by `trunk build`). In development, `DashboardAssets` can
//! also read the dist directory from disk so GUI edits do not require rebuilding
//! `cade-server`.

use axum::{
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

use super::dashboard_assets::DashboardAssets;

const CACHE_CONTROL_REVALIDATE: &str = "no-cache";
const INDEX_ASSET: &str = "index.html";
const SNIPPETS_PREFIX: &str = "snippets/";

/// Deep module for serving the browser dashboard.
///
/// Interface: callers only choose `index()` or `asset(path)`. The implementation
/// owns every route invariant: safe relative paths, `/snippets/*` fallback,
/// MIME inference, cache policy, and conversion into Axum responses.
pub(crate) struct DashboardSite;

impl DashboardSite {
    /// Serve the dashboard HTML shell.
    pub(crate) fn index() -> Response {
        Self::serve_asset(INDEX_ASSET, AssetLookup::Exact)
    }

    /// Serve a dashboard asset captured from `/dashboard/*path` or
    /// `/snippets/*path`.
    pub(crate) fn asset(path: &str) -> Response {
        Self::serve_asset(path, AssetLookup::WithSnippetFallback)
    }

    fn serve_asset(path: &str, lookup: AssetLookup) -> Response {
        let Some(path) = normalize_asset_path(path) else {
            return not_found();
        };

        let asset = match lookup {
            AssetLookup::Exact => DashboardAssets::get(path),
            AssetLookup::WithSnippetFallback => DashboardAssets::get(path).or_else(|| {
                path.strip_prefix(SNIPPETS_PREFIX)
                    .is_none()
                    .then(|| DashboardAssets::get(&format!("{SNIPPETS_PREFIX}{path}")))
                    .flatten()
            }),
        };

        match asset {
            Some(file) => (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime_for(path)),
                    (header::CACHE_CONTROL, CACHE_CONTROL_REVALIDATE),
                ],
                file.data.to_vec(),
            )
                .into_response(),
            None => not_found(),
        }
    }
}

enum AssetLookup {
    Exact,
    WithSnippetFallback,
}

/// Infer a MIME type from a file extension.
///
/// Covers the file types trunk produces. Unknown extensions fall back to
/// `application/octet-stream`.
fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// Return a safe dashboard asset path relative to `cade-gui/dist/`.
///
/// The public dashboard routes are unauthenticated, so path traversal and path
/// syntax from other platforms must be rejected before reaching the asset layer.
fn normalize_asset_path(path: &str) -> Option<&str> {
    let path = path.trim_start_matches("./");
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        None
    } else {
        Some(path)
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// `GET /dashboard` — serves the embedded `index.html`.
pub async fn get_dashboard() -> Response {
    DashboardSite::index()
}

/// `GET /dashboard/*path` or `GET /snippets/*path` — serves JS, WASM,
/// snippets, and other trunk-built assets.
pub async fn get_dashboard_asset(Path(path): Path<String>) -> Response {
    DashboardSite::asset(&path)
}

#[cfg(test)]
#[path = "dashboard_test.rs"]
mod tests;
