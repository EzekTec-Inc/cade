use super::Repl;

impl Repl {
    /// Push a success line (green) to the TUI.
    pub(crate) fn tui_ok(&self, msg: impl Into<String>) {
        let _ = self
            .app
            .lock()
            .push(crate::ui::RenderLine::SuccessMsg(msg.into()));
    }
    /// Push an error line (red) to the TUI.
    pub(crate) fn tui_err(&self, msg: impl Into<String>) {
        let _ = self
            .app
            .lock()
            .push(crate::ui::RenderLine::ErrorMsg(msg.into()));
    }
    /// Push a section header (cyan bold) to the TUI.
    pub(crate) fn tui_hdr(&self, msg: impl Into<String>) {
        let _ = self
            .app
            .lock()
            .push(crate::ui::RenderLine::InfoHeader(msg.into()));
    }
    /// Push a dim hint / secondary text to the TUI.
    pub(crate) fn tui_dim(&self, msg: impl Into<String>) {
        let _ = self
            .app
            .lock()
            .push(crate::ui::RenderLine::DimMsg(msg.into()));
    }
    /// Push a plain system message (gray) to the TUI.
    pub(crate) fn tui_sys(&self, msg: impl Into<String>) {
        let _ = self
            .app
            .lock()
            .push(crate::ui::RenderLine::SystemMsg(msg.into()));
    }
    /// Push a blank line to the TUI.
    pub(crate) fn tui_blank(&self) {
        let _ = self.app.lock().push(crate::ui::RenderLine::Blank);
    }
}
