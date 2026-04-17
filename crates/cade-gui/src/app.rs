//! WASM-only eframe app placeholder.
//!
//! Intentionally empty at this milestone — the render loop, panels, SSE
//! client, and markdown rendering land in later stop-and-ask milestones.
//! This file exists so `pub mod app` resolves on wasm32 and the crate
//! compiles cleanly, proving the approved dep set (eframe/egui/wasm-bindgen/
//! gloo-net/serde-wasm-bindgen/egui_commonmark/web-sys) is compatible.
