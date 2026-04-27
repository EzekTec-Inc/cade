//! Artifacts overlay state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Open the artifacts overlay.  Caller is expected to spawn a list
    /// fetch; this marks the panel as loading and clears error/selection.
    pub fn open_artifacts_overlay(&mut self) {
        if let Self::Connected {
            artifacts_open,
            artifacts_loading,
            artifacts_error,
            artifact_selection,
            artifact_detail,
            ..
        } = self
        {
            *artifacts_open = true;
            *artifacts_loading = true;
            *artifacts_error = None;
            *artifact_selection = None;
            *artifact_detail = None;
        }
    }

    /// Close the artifacts overlay.  Retains cached list for instant
    /// reopen; clears transient flags.
    pub fn close_artifacts_overlay(&mut self) {
        if let Self::Connected {
            artifacts_open,
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_open = false;
            *artifacts_busy = false;
            *artifacts_error = None;
        }
    }

    /// Whether the artifacts overlay is currently visible.
    pub fn is_artifacts_open(&self) -> bool {
        matches!(
            self,
            Self::Connected {
                artifacts_open: true,
                ..
            }
        )
    }

    /// Feed the result of a successful artifacts-list fetch.
    pub fn on_artifacts_loaded(&mut self, rows: Vec<crate::api::ArtifactInfo>) {
        if let Self::Connected {
            artifacts,
            artifacts_loading,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_loading = false;
            *artifacts_error = None;
            *artifacts = rows;
        }
    }

    /// Feed an error from an artifact fetch or action.
    pub fn on_artifacts_error(&mut self, err: &str) {
        if let Self::Connected {
            artifacts_loading,
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_loading = false;
            *artifacts_busy = false;
            *artifacts_error = Some(err.to_string());
        }
    }

    /// Mark a detail/delete request as in-flight.
    pub fn on_artifacts_action_start(&mut self) {
        if let Self::Connected {
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_busy = true;
            *artifacts_error = None;
        }
    }

    /// Select an artifact row.  Clears stale detail so the renderer
    /// shows a loading indicator while the per-id fetch runs.  Returns
    /// the selected artifact id (so the spawn helper can issue the GET)
    /// or `None` when the index is out of bounds / not connected.
    pub fn select_artifact(&mut self, idx: usize) -> Option<String> {
        if let Self::Connected {
            artifacts,
            artifact_selection,
            artifact_detail,
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            let id = artifacts.get(idx).map(|a| a.id.clone());
            if id.is_some() {
                *artifact_selection = Some(idx);
                *artifact_detail = None;
                *artifacts_busy = true;
                *artifacts_error = None;
            }
            id
        } else {
            None
        }
    }

    /// Feed full detail after a successful per-id fetch.
    pub fn on_artifact_detail_loaded(&mut self, detail: crate::api::ArtifactDetail) {
        if let Self::Connected {
            artifact_detail,
            artifacts_busy,
            artifacts_error,
            ..
        } = self
        {
            *artifacts_busy = false;
            *artifacts_error = None;
            *artifact_detail = Some(detail);
        }
    }

    /// Return the id of the artifact currently selected, if any.  Used
    /// by the delete button to pass the right id to the spawn helper.
    pub fn selected_artifact_id(&self) -> Option<String> {
        if let Self::Connected {
            artifacts,
            artifact_selection: Some(idx),
            ..
        } = self
        {
            artifacts.get(*idx).map(|a| a.id.clone())
        } else {
            None
        }
    }

    /// Read-only snapshot of the cached artifact list.
    pub fn artifacts_snapshot(&self) -> &[crate::api::ArtifactInfo] {
        if let Self::Connected { artifacts, .. } = self {
            artifacts
        } else {
            &[]
        }
    }

    /// Read-only access to the currently-loaded artifact detail (if any).
    pub fn artifact_detail(&self) -> Option<&crate::api::ArtifactDetail> {
        if let Self::Connected {
            artifact_detail, ..
        } = self
        {
            artifact_detail.as_ref()
        } else {
            None
        }
    }

}
