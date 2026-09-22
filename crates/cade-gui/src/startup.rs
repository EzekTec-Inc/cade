//! Browser startup readiness seam for the WASM dashboard.
//!
//! The HTML shell owns the actual loader implementation. Rust only crosses a
//! tiny interface: mark the dashboard ready once Dioxus has mounted far enough
//! for users to interact with it.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// Mark the dashboard as ready in the browser loader, if that loader is present.
///
/// This is intentionally a no-op on non-WASM targets so native checks/tests do
/// not need a browser runtime.
pub fn mark_dashboard_ready(reason: &str) {
    mark_dashboard_ready_impl(reason);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = "
export function cade_dashboard_ready(reason) {
  const startup = globalThis.cadeDashboardStartup;
  if (startup && typeof startup.ready === 'function') {
    startup.ready(reason);
  } else if (typeof globalThis.cadeLog === 'function') {
    globalThis.cadeLog('Dashboard ready before startup module was available: ' + reason);
  }
}
")]
extern "C" {
    fn cade_dashboard_ready(reason: &str);
}

#[cfg(target_arch = "wasm32")]
fn mark_dashboard_ready_impl(reason: &str) {
    cade_dashboard_ready(reason);
}

#[cfg(not(target_arch = "wasm32"))]
fn mark_dashboard_ready_impl(_reason: &str) {}
