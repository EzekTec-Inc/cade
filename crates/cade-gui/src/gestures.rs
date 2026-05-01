//! Gesture recognition for mobile/touch interfaces.

use egui;

/// Represents a recognized touch gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    /// Swipe from left to right
    SwipeRight,
    /// Swipe from right to left
    SwipeLeft,
    /// Swipe from top to bottom
    SwipeDown,
    /// Swipe from bottom to top
    SwipeUp,
}

/// Detects swipe gestures from egui's input state.
/// Returns `Some(Gesture)` if a swipe was completed this frame.
pub fn detect_swipe(ctx: &egui::Context) -> Option<Gesture> {
    ctx.input(|i| {
        if i.pointer.any_released() {
            let velocity = i.pointer.velocity();
            let threshold = 500.0; // Points per second to trigger a swipe

            if velocity.x.abs() > velocity.y.abs() {
                // Horizontal swipe
                if velocity.x > threshold {
                    return Some(Gesture::SwipeRight);
                } else if velocity.x < -threshold {
                    return Some(Gesture::SwipeLeft);
                }
            } else {
                // Vertical swipe
                if velocity.y > threshold {
                    return Some(Gesture::SwipeDown);
                } else if velocity.y < -threshold {
                    return Some(Gesture::SwipeUp);
                }
            }
        }
        None
    })
}
