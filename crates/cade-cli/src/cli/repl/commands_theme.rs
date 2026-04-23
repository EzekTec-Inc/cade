//! /theme command handler.
//!
//! Resolution order:
//!   1. Built-in registry (`ThemeColors::builtin_by_name`) — dark, light, etc.
//!   2. User JSON themes discovered in project `.cade/themes/` + `~/.cade/themes/`
//!
//! Both sources are merged for the picker list so built-ins and custom themes
//! appear together with no duplicates.

use crate::Result;
use super::Repl;
use cade_core::resources::themes::ThemeColors;

impl Repl {
    pub(crate) async fn cmd_theme(
        &mut self,
        theme_arg: Option<String>,
    ) -> Result<bool> {
        let new_theme = theme_arg.map(|t| t.trim().to_string()).unwrap_or_default();

        // -- Bare `/theme` → open picker
        if new_theme.is_empty() {
            let agent_dir = self
                .settings
                .lock()
                .global_path()
                .parent()
                .unwrap()
                .to_path_buf();

            // Discovered on-disk themes
            let mut discovered = cade_core::resources::discover_themes(&self.cwd, &agent_dir);

            // Merge built-ins that aren't already on disk — using the
            // canonical `builtin_listing()` registry from cade-core so the
            // CLI picker cannot drift from other surfaces.
            for (idx, (name, desc, variant)) in
                ThemeColors::builtin_listing().iter().enumerate()
            {
                if !discovered.iter().any(|t| t.name == *name) {
                    discovered.insert(
                        idx,
                        cade_core::resources::Theme {
                            name: name.to_string(),
                            description: Some(desc.to_string()),
                            author: Some("CADE".to_string()),
                            variant: Some(variant.to_string()),
                            vars: Default::default(),
                            colors: Default::default(),
                            source: std::path::PathBuf::from("builtin"),
                        },
                    );
                }
            }

            let current_colors = self.app.lock().colors.clone();
            self.app
                .lock()
                .open_theme_picker(discovered, current_colors);
            return Ok(false);
        }

        // -- `/theme <name>` → resolve + apply
        let name = new_theme;
        let (target_theme_colors, found_name) = if let Some(tc) =
            ThemeColors::builtin_by_name(&name)
        {
            (tc, name.clone())
        } else {
            let agent_dir = self
                .settings
                .lock()
                .global_path()
                .parent()
                .unwrap()
                .to_path_buf();
            let discovered = cade_core::resources::discover_themes(&self.cwd, &agent_dir);
            if let Some(t) = discovered.iter().find(|t| t.name == name) {
                (ThemeColors::from_theme(t), t.name.clone())
            } else {
                (ThemeColors::dark(), String::new())
            }
        };

        if found_name.is_empty() {
            self.tui_err(format!("  ✗ Theme '{name}' not found."));
        } else {
            {
                let mut app = self.app.lock();
                app.apply_theme(target_theme_colors);
            }
            {
                let mut s = self.settings.lock();
                s.global_settings_mut().theme = Some(found_name.clone());
                let _ = s.save_global();
            }
            self.tui_ok(format!("  ✓ Theme changed to '{found_name}'"));
        }
        Ok(false)
    }
}
