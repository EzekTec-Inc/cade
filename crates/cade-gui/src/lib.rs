//! CADE web GUI — egui/eframe client served at `/dashboard` by cade-server.
//!
//! At this milestone the crate exposes only the pure `config` module and a
//! placeholder `app` module.  Browser entry wiring (`#[wasm_bindgen(start)]`,
//! `eframe::WebRunner`) comes in a later, separately-approved milestone once
//! the config primitive is green on native.

pub mod config;

#[cfg(target_arch = "wasm32")]
pub mod app;
