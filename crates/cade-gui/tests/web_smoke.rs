//! Browser-side smoke tests for the cade-gui render layer.
//!
//! These compile only for wasm32 and run under `wasm-bindgen-test` harnesses
//! (e.g. `wasm-pack test --headless --firefox`).  They are intentionally
//! minimal at this milestone: the render code is a thin adapter over
//! `crate::login::LoginState`, whose behaviour is fully covered by native
//! tests.  The tests here exist to verify that the wasm-bindgen-test harness
//! is wired up correctly so subsequent milestones can add real render-loop
//! assertions without re-plumbing the test setup.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

/// Sanity test — the crate links and the harness executes.
/// This is the "hello world" of wasm-bindgen-test.  A real render-loop
/// assertion (e.g. mounting a headless canvas and checking egui frame
/// output) lands in a later milestone when the runner is wired into CI.
#[wasm_bindgen_test]
fn harness_runs_in_browser() {
    // If this function is reached and returns, the harness works.
    assert_eq!(2 + 2, 4);
}

/// The pure state machine is also reachable in the browser target, not
/// just native — defends against accidentally gating it behind cfg(not(wasm)).
#[wasm_bindgen_test]
fn login_state_machine_is_usable_in_wasm() {
    let mut s = cade_gui::login::LoginState::new();
    s.on_input("tok");
    s.on_submit();
    match s {
        cade_gui::login::LoginState::Submitted { key } => assert_eq!(key, "tok"),
        other => panic!("expected Submitted, got {other:?}"),
    }
}
