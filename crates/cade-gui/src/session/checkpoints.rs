//! Checkpoint overlay state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Open the checkpoints overlay.  Caller is expected to spawn a
    /// fetch; this just marks the panel as loading and clears error.
    pub fn open_checkpoints_overlay(&mut self) {
        if let Self::Connected {
            checkpoints_open,
            checkpoints_loading,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_open = true;
            *checkpoints_loading = true;
            *checkpoints_error = None;
            *checkpoints_notice = None;
        }
    }

    /// Close the checkpoints overlay.  Retains the cached list so a
    /// reopen is instant; clears transient flags.
    pub fn close_checkpoints_overlay(&mut self) {
        if let Self::Connected {
            checkpoints_open,
            checkpoints_busy,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_open = false;
            *checkpoints_busy = false;
            *checkpoints_error = None;
            *checkpoints_notice = None;
        }
    }

    /// Whether the checkpoints overlay is currently visible.
    pub fn is_checkpoints_open(&self) -> bool {
        matches!(
            self,
            Self::Connected {
                checkpoints_open: true,
                ..
            }
        )
    }

    /// Feed the result of a successful checkpoints fetch.
    pub fn on_checkpoints_loaded(&mut self, rows: Vec<crate::api::CheckpointRow>) {
        if let Self::Connected {
            checkpoints,
            checkpoints_loading,
            checkpoints_error,
            ..
        } = self
        {
            *checkpoints_loading = false;
            *checkpoints_error = None;
            *checkpoints = rows;
        }
    }

    /// Feed an error from a checkpoint fetch or action.  Clears
    /// loading + busy flags so the UI becomes interactable again.
    pub fn on_checkpoints_error(&mut self, err: &str) {
        if let Self::Connected {
            checkpoints_loading,
            checkpoints_busy,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_loading = false;
            *checkpoints_busy = false;
            *checkpoints_error = Some(err.to_string());
            *checkpoints_notice = None;
        }
    }

    /// Mark a restore/create/delete request as in-flight.
    pub fn on_checkpoints_action_start(&mut self) {
        if let Self::Connected {
            checkpoints_busy,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_busy = true;
            *checkpoints_error = None;
            *checkpoints_notice = None;
        }
    }

    /// Mark an action as completed successfully with a transient notice.
    pub fn on_checkpoints_action_ok(&mut self, notice: &str) {
        if let Self::Connected {
            checkpoints_busy,
            checkpoints_error,
            checkpoints_notice,
            ..
        } = self
        {
            *checkpoints_busy = false;
            *checkpoints_error = None;
            *checkpoints_notice = Some(notice.to_string());
        }
    }

    /// Read-only snapshot of the cached checkpoint list, for tests +
    /// the renderer.  Returns `&[]` when not connected.
    pub fn checkpoints_snapshot(&self) -> &[crate::api::CheckpointRow] {
        if let Self::Connected { checkpoints, .. } = self {
            checkpoints
        } else {
            &[]
        }
    }

    /// Read the current notice string (e.g. "Restored cp-abc…").
    pub fn checkpoints_notice(&self) -> Option<&str> {
        if let Self::Connected {
            checkpoints_notice: Some(n),
            ..
        } = self
        {
            Some(n.as_str())
        } else {
            None
        }
    }

}
