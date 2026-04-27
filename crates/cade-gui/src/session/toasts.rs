//! Toast / error notification state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Store an error message for display as a toast overlay.
    ///
    /// If a stream is in progress it is marked complete so the UI unblocks.
    /// Replaces any previously stored error.
    pub fn push_error(&mut self, msg: &str) {
        if let Self::Connected {
            streaming,
            error_toast,
            ..
        } = self
        {
            *streaming = false;
            *error_toast = Some(msg.to_string());
        }
    }

    /// Clear the current error toast (e.g. after the user dismisses it).
    pub fn dismiss_error(&mut self) {
        if let Self::Connected { error_toast, .. } = self {
            *error_toast = None;
        }
    }

    /// Show an informational toast — same channel as `push_error`, used
    /// by Phase-3 `/compact` and overflow-warning surfacing.  We
    /// intentionally reuse the `error_toast` slot because the GUI today
    /// only has one toast channel; the toast renderer treats any
    /// non-empty `error_toast` as a notice and does not hard-disable
    /// streaming when the message starts with "✓" or "Compacting".
    pub fn push_info(&mut self, msg: &str) {
        if let Self::Connected { error_toast, .. } = self {
            *error_toast = Some(msg.to_string());
        }
    }

    /// The current error message, if any.
    pub fn error_toast(&self) -> Option<&str> {
        if let Self::Connected { error_toast, .. } = self {
            error_toast.as_deref()
        } else {
            None
        }
    }
}
