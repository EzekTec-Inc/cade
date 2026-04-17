//! `/dashboard` — static login page for the future WASM `cade-gui` client.
//!
//! Security contract:
//! - This route is **exempt from `auth_middleware`** (see `auth.rs`), so the
//!   browser can fetch the page without a bearer token.
//! - The served HTML **never** embeds the server's `api_key`. The user pastes
//!   their key into the form; the WASM app (future M2+) holds it in memory
//!   only. This keeps the auth boundary intact against drive-by GETs.
//! - GET is a "safe method" per RFC 9110 §9.2.1, so the CSRF middleware does
//!   not interfere.

use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

/// Minimal static HTML login page.
///
/// Kept inline (not `rust-embed`) at this milestone to hold the change
/// surface to one file. When real WASM assets arrive in M2+, this moves to
/// an embedded `index.html` + `assets/` folder.
const DASHBOARD_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>CADE Dashboard</title>
<style>
  :root { color-scheme: dark; }
  body {
    font-family: ui-sans-serif, system-ui, -apple-system, sans-serif;
    background: #0f1115;
    color: #e5e7eb;
    display: grid;
    place-items: center;
    min-height: 100vh;
    margin: 0;
  }
  main { max-width: 28rem; width: 90%; }
  h1 { font-size: 1.25rem; margin: 0 0 1rem; }
  p  { color: #9ca3af; font-size: 0.9rem; margin: 0 0 1rem; }
  label { display: block; font-size: 0.8rem; margin: 0 0 0.35rem; color: #9ca3af; }
  input[type=password] {
    width: 100%;
    padding: 0.6rem 0.75rem;
    border: 1px solid #374151;
    background: #111827;
    color: #f3f4f6;
    border-radius: 6px;
    font: inherit;
    box-sizing: border-box;
  }
  button {
    margin-top: 0.75rem;
    padding: 0.6rem 1rem;
    background: #2563eb;
    color: #fff;
    border: 0;
    border-radius: 6px;
    cursor: pointer;
    font: inherit;
  }
  button:hover { background: #1d4ed8; }
  .note { font-size: 0.75rem; color: #6b7280; margin-top: 1rem; }
</style>
</head>
<body>
<main>
  <h1>CADE Dashboard</h1>
  <p>Paste your CADE API key to connect. The key is held in browser memory only; it is never stored or sent anywhere except this server.</p>
  <form id="f" autocomplete="off" onsubmit="event.preventDefault();">
    <label for="k">API key</label>
    <input id="k" type="password" placeholder="CADE_API_KEY" required>
    <button type="submit">Connect</button>
  </form>
  <p class="note">UI coming soon. This page reserves the route.</p>
</main>
</body>
</html>
"#;

/// `GET /dashboard` — serves the static login page.
pub async fn get_dashboard() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        DASHBOARD_HTML,
    )
        .into_response()
}

#[cfg(test)]
#[path = "dashboard_test.rs"]
mod tests;
