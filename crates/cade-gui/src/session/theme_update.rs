//! Theme-update state for [`super::SessionState`].

use super::*;

impl SessionState {
    pub fn on_theme_update(&mut self, theme: String) {
        if let Self::Connected(session) = self {
            let crate::session::ConnectedSession { theme_update, .. } = &mut **session;
            *theme_update = Some(theme);
        }
    }
}
