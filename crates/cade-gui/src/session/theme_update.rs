//! Theme-update state for [`super::SessionState`].

use super::*;

impl SessionState {
    pub fn on_theme_update(&mut self, theme: crate::theme::ThemeColors) {
        if let Self::Connected { theme_update, .. } = self {
            *theme_update = Some(theme);
        }
    }

}
