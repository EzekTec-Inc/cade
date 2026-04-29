//! [`OverlayComponent`] trait — the abstraction that any modal overlay
//! in the CADE TUI must implement.
//!
//! ## Why
//!
//! Today `TuiApp` tracks every overlay as a separate `Option<State>`
//! field (picker, theme picker, command palette, summary, active
//! question).  Adding a new overlay means touching the struct, the
//! render path, and the input dispatcher in lockstep.
//!
//! This trait gives every overlay a uniform shape so the host can
//! eventually route render+input through a single `Vec<Box<dyn
//! OverlayComponent>>` stack.  Adopting a new overlay then becomes
//! "implement the trait + push onto the stack" — no edits to
//! `TuiApp` required.
//!
//! ## Contract
//!
//! - **`render_overlay`** draws the overlay on top of the existing
//!   frame.
//! - **`handle_input`** processes a key event and returns whether it
//!   was consumed.
//! - **`is_dismissed`** signals that the host should pop this overlay
//!   from the stack.
//!
//! Migration is incremental: overlays are migrated one at a time from
//! legacy `Option<...State>` fields into a `Vec<Box<dyn
//! OverlayComponent>>` stack.  The host dispatches input to the
//! topmost overlay, renders all of them bottom-to-top, and pops on
//! dismiss.
//!
//! ## Result channel
//!
//! Some overlays produce a value when they close (e.g. command
//! palette → `/command`, file picker → insertion text).  The
//! [`OverlayComponent::take_result`] method lets the host retrieve
//! that value after a `Dismiss` signal, downcast it via
//! `Box<dyn Any>`, and act accordingly.

use std::any::Any;

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use crate::colors::ThemeColors;

/// Outcome of [`OverlayComponent::handle_input`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayInputResult {
    /// Event was consumed; do not propagate further.
    Consumed,
    /// Event was consumed and the overlay should be dismissed.
    /// The host pops it from the stack and may call
    /// [`OverlayComponent::take_result`] for the return value.
    Dismiss,
    /// Event was not relevant to this overlay; bubble up.
    NotHandled,
}

/// The pluggable overlay interface.
///
/// Implementations encapsulate their own state — the host only knows
/// the trait surface.  This is the seam that makes overlays
/// composable instead of hardcoded into `TuiApp`.
pub trait OverlayComponent: Send + Sync {
    /// Stable identifier for this overlay kind.  Used for logging,
    /// focus tracking, and "is this overlay open?" queries by name.
    fn id(&self) -> &'static str;

    /// Draw the overlay on top of the frame within `area`.
    ///
    /// Implementations should call [`crate::overlay::render_overlay_shell`]
    /// for a consistent look-and-feel.
    fn render_overlay(&mut self, frame: &mut Frame, area: Rect, colors: &ThemeColors);

    /// Process a key event while this overlay is on top of the stack.
    ///
    /// See [`OverlayInputResult`] for outcomes.
    fn handle_input(&mut self, key: KeyEvent) -> OverlayInputResult;

    /// Returns `true` when the overlay has finished its work and the
    /// host should pop it.  Allows overlays to dismiss themselves
    /// asynchronously (e.g. after an awaited future resolves) rather
    /// than only via `OverlayInputResult::Dismiss`.
    fn is_dismissed(&self) -> bool {
        false
    }

    /// Drain the overlay's result value, if any.
    ///
    /// Called by the host after receiving [`OverlayInputResult::Dismiss`].
    /// The concrete type is overlay-specific — callers downcast via
    /// `Box<dyn Any>::downcast::<T>()`.
    ///
    /// Returns `None` for overlays that have no meaningful return
    /// value (e.g. summary viewer).
    fn take_result(&mut self) -> Option<Box<dyn Any>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Minimal overlay stub used to verify the trait is object-safe
    /// and the host can store a heterogeneous stack.
    struct StubOverlay {
        ident: &'static str,
        dismissed: bool,
    }

    impl OverlayComponent for StubOverlay {
        fn id(&self) -> &'static str {
            self.ident
        }
        fn render_overlay(&mut self, _f: &mut Frame, _a: Rect, _c: &ThemeColors) {}
        fn handle_input(&mut self, key: KeyEvent) -> OverlayInputResult {
            if matches!(key.code, KeyCode::Esc) {
                self.dismissed = true;
                OverlayInputResult::Dismiss
            } else {
                OverlayInputResult::Consumed
            }
        }
        fn is_dismissed(&self) -> bool {
            self.dismissed
        }
    }

    #[test]
    fn overlay_component_is_object_safe() {
        // If this stops compiling, the trait gained a non-dispatchable
        // method (e.g. a generic without where Self: Sized).
        let _stack: Vec<Box<dyn OverlayComponent>> = vec![];
    }

    #[test]
    fn overlay_input_result_variants_are_distinct() {
        assert_ne!(OverlayInputResult::Consumed, OverlayInputResult::Dismiss);
        assert_ne!(
            OverlayInputResult::Consumed,
            OverlayInputResult::NotHandled
        );
    }

    #[test]
    fn stub_overlay_routes_esc_to_dismiss() {
        let mut o = StubOverlay {
            ident: "stub",
            dismissed: false,
        };
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(o.handle_input(key), OverlayInputResult::Dismiss);
        assert!(o.is_dismissed());
    }

    #[test]
    fn stub_overlay_consumes_other_keys() {
        let mut o = StubOverlay {
            ident: "stub",
            dismissed: false,
        };
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(o.handle_input(key), OverlayInputResult::Consumed);
        assert!(!o.is_dismissed());
    }

    #[test]
    fn host_can_dispatch_to_top_of_stack() {
        // Simulate the host's intended dispatch loop.
        let mut stack: Vec<Box<dyn OverlayComponent>> = vec![
            Box::new(StubOverlay {
                ident: "bottom",
                dismissed: false,
            }),
            Box::new(StubOverlay {
                ident: "top",
                dismissed: false,
            }),
        ];
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let res = stack.last_mut().unwrap().handle_input(key);
        assert_eq!(res, OverlayInputResult::Dismiss);
        // Simulate the pop the host would perform.
        if matches!(res, OverlayInputResult::Dismiss) {
            stack.pop();
        }
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0].id(), "bottom");
    }

    #[test]
    fn overlay_ids_are_stable_for_lookup() {
        let o = StubOverlay {
            ident: "command_palette",
            dismissed: false,
        };
        assert_eq!(o.id(), "command_palette");
    }

    /// Overlay stub that produces a typed result on dismiss.
    struct ResultOverlay {
        result: Option<String>,
    }

    impl OverlayComponent for ResultOverlay {
        fn id(&self) -> &'static str {
            "result_stub"
        }
        fn render_overlay(&mut self, _f: &mut Frame, _a: Rect, _c: &ThemeColors) {}
        fn handle_input(&mut self, key: KeyEvent) -> OverlayInputResult {
            if matches!(key.code, KeyCode::Enter) {
                OverlayInputResult::Dismiss
            } else {
                OverlayInputResult::Consumed
            }
        }
        fn take_result(&mut self) -> Option<Box<dyn Any>> {
            self.result.take().map(|s| Box::new(s) as Box<dyn Any>)
        }
    }

    #[test]
    fn take_result_returns_typed_value() {
        let mut o = ResultOverlay {
            result: Some("/help".into()),
        };
        let r = o.take_result().unwrap();
        let s = r.downcast::<String>().unwrap();
        assert_eq!(*s, "/help");
        // Second call returns None (drained).
        assert!(o.take_result().is_none());
    }

    #[test]
    fn take_result_default_is_none() {
        let mut o = StubOverlay {
            ident: "no_result",
            dismissed: false,
        };
        assert!(o.take_result().is_none());
    }
}
