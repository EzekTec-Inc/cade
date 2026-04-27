//! Context-breakdown state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Start loading context breakdown.
    pub fn start_context_breakdown_loading(&mut self) {
        if let Self::Connected {
            context_breakdown_loading,
            ..
        } = self
        {
            *context_breakdown_loading = true;
        }
    }

    /// Store fetched context breakdown.
    pub fn on_context_breakdown(&mut self, breakdown: crate::api::ContextBreakdown) {
        if let Self::Connected {
            context_breakdown,
            context_breakdown_loading,
            ..
        } = self
        {
            *context_breakdown = Some(breakdown);
            *context_breakdown_loading = false;
        }
    }

    /// Clear context breakdown on error.
    pub fn on_context_breakdown_error(&mut self) {
        if let Self::Connected {
            context_breakdown_loading,
            ..
        } = self
        {
            *context_breakdown_loading = false;
        }
    }

    /// Read-only access to context breakdown.
    pub fn context_breakdown(&self) -> Option<&crate::api::ContextBreakdown> {
        if let Self::Connected {
            context_breakdown, ..
        } = self
        {
            context_breakdown.as_ref()
        } else {
            None
        }
    }

    /// Whether a context breakdown fetch is in progress.
    pub fn is_context_breakdown_loading(&self) -> bool {
        if let Self::Connected {
            context_breakdown_loading,
            ..
        } = self
        {
            *context_breakdown_loading
        } else {
            false
        }
    }
}
