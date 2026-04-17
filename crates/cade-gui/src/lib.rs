//! CADE web GUI — egui/eframe client served at `/dashboard` by cade-server.
//!
//! Public modules:
//! - `config`    — pure boot-time configuration parser (native + wasm).
//! - `login`     — pure login-screen state machine (native + wasm).
//! - `api`       — pure HTTP URL/header builders and response parsers.
//! - `sse`       — pure SSE frame parser (native + wasm).
//! - `app`       — `eframe::App` login-screen renderer (wasm-only).
//! - `http_wasm` — thin gloo-net adapter issuing real fetches (wasm-only).
//!
//! The crate exposes a `#[wasm_bindgen(start)]` entry that mounts the
//! `CadeApp` on the `#cade_gui_canvas` element.  The browser-side code is
//! intentionally thin — all testable behaviour lives in `config` and
//! `login`, both of which are covered by native `cargo test`.

pub mod api;
pub mod config;
pub mod login;
pub mod sse;

#[cfg(target_arch = "wasm32")]
pub mod app;

#[cfg(target_arch = "wasm32")]
pub mod http_wasm;

#[cfg(target_arch = "wasm32")]
mod boot {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::prelude::*;

    /// ID of the `<canvas>` element the dashboard page hosts.
    /// Keep in sync with `cade-server/src/server/api/dashboard.rs`.
    const CANVAS_ID: &str = "cade_gui_canvas";

    /// Invoked automatically by the browser after the WASM module is
    /// instantiated.  Mounts `CadeApp` on the dashboard canvas.
    #[wasm_bindgen(start)]
    pub async fn start() -> Result<(), JsValue> {
        // Forward Rust panics to the browser console for visibility during
        // development.  Silent in release builds with panic=abort.
        console_error_panic_hook_lite();

        let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
        let document = window
            .document()
            .ok_or_else(|| JsValue::from_str("no document"))?;
        let canvas = document
            .get_element_by_id(CANVAS_ID)
            .ok_or_else(|| JsValue::from_str("missing #cade_gui_canvas element"))?
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .map_err(|_| JsValue::from_str("#cade_gui_canvas is not a <canvas>"))?;

        let runner = eframe::WebRunner::new();
        runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(crate::app::CadeApp::new(cc)))),
            )
            .await
    }

    /// Minimal stand-in for the `console_error_panic_hook` crate — avoids a
    /// new dependency by forwarding the panic message to `console.error`.
    fn console_error_panic_hook_lite() {
        std::panic::set_hook(Box::new(|info| {
            let msg = format!("{info}");
            web_sys::console::error_1(&JsValue::from_str(&msg));
        }));
    }
}
