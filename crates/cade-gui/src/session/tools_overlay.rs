//! Tools overlay state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Open the tools overlay.  Caller spawns the fetch.
    pub fn open_tools_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                tools_open,
                tools_loading,
                tools_error,
                ..
            } = &mut **session;
            *tools_open = true;
            *tools_loading = true;
            *tools_error = None;
        }
    }

    /// Close the tools overlay.
    pub fn close_tools_overlay(&mut self) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                tools_open,
                tools_error,
                ..
            } = &mut **session;
            *tools_open = false;
            *tools_error = None;
        }
    }

    /// Whether the tools overlay is currently visible.
    pub fn is_tools_open(&self) -> bool {
        matches!(self, Self::Connected(session) if matches!(&**session, crate::session::ConnectedSession {
           tools_open: true,
           ..
        }))
    }

    /// Feed the result of a successful tools fetch.
    pub fn on_tools_loaded(&mut self, rows: Vec<crate::api::AgentTool>) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                tools,
                tools_loading,
                tools_error,
                ..
            } = &mut **session;
            *tools_loading = false;
            *tools_error = None;
            *tools = rows;
        }
    }

    /// Feed an error from a tools fetch.
    pub fn on_tools_error(&mut self, err: &str) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession {
                tools_loading,
                tools_error,
                ..
            } = &mut **session;
            *tools_loading = false;
            *tools_error = Some(err.to_string());
        }
    }

    /// Read-only snapshot of the cached tool list.
    pub fn tools_snapshot(&self) -> &[crate::api::AgentTool] {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { tools, .. } = &**session;
            tools
        } else {
            &[]
        }
    }
}
