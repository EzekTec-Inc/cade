//! Dynamic plugin system for CADE agents.
//!
//! Provides runtimes for JavaScript/TypeScript (QuickJS via `rquickjs`)
//! and WebAssembly (WASI sandbox via `wasmtime`) that can extend CADE
//! with third-party tools written in those languages.
//!
//! Feature-gated:
//! - `plugin-js`  → enables `js_runtime`
//! - `plugin-wasm` → enables `wasm_runtime`

#[cfg(feature = "plugin-js")]
pub mod js_runtime;
#[cfg(feature = "plugin-wasm")]
pub mod wasm_runtime;
