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
//! directory (built by `trunk build`).  In debug builds, `rust-embed` reads
//! from the filesystem so you can iterate on the GUI without recompiling.

use axum::{
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

use super::dashboard_assets::DashboardAssets;

/// Infer a MIME type from a file extension.
///
/// Covers the file types trunk produces.  Unknown extensions fall back to
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

/// Serve an embedded file or 404.
fn serve_embedded(path: &str) -> Response {
    match DashboardAssets::get(path) {
        Some(file) => {
            let mime = mime_for(path);
            // Cache-bust files with a hash in their name (trunk adds hashes).
            // index.html must be revalidated on every request so browsers pick
            // up new asset hashes after a rebuild.
            let cache = if path == "index.html" {
                "no-cache"
            } else {
                "public, max-age=31536000, immutable"
            };
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)],
                file.data.to_vec(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// `GET /dashboard` — serves the embedded `index.html`.
pub async fn get_dashboard() -> Response {
    serve_embedded("index.html")
}

/// `GET /dashboard/*path` — serves JS, WASM, and other trunk-built assets.
pub async fn get_dashboard_asset(Path(path): Path<String>) -> Response {
    serve_embedded(&path)
}

#[cfg(test)]
#[path = "dashboard_test.rs"]
mod tests;
