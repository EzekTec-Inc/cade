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

            // Built-ins + on-disk themes, merged via the canonical helper
            // so the picker list cannot drift from other surfaces.
            let discovered =
                cade_core::resources::discover_themes_with_builtins(&self.cwd, &agent_dir);

            let current_colors = self.app.lock().colors.clone();
            self.app
                .lock()
                .open_theme_picker(discovered, current_colors);
            return Ok(false);
        }

        // -- `/theme reload` → re-read the current theme from disk
        if new_theme == "reload" {
            let current_source = self.app.lock().colors.source_path.clone();
            if let Some(path) = current_source {
                if path.exists() {
                    match cade_core::resources::themes::load_theme(&path) {
                        Ok(theme) => {
                            let colors = ThemeColors::from_theme(&theme);
                            self.app.lock().apply_theme(colors);
                            self.tui_ok(format!(
                                "  ✓ Theme reloaded from {}",
                                path.display()
                            ));
                        }
                        Err(e) => {
                            self.tui_err(format!("  ✗ Failed to reload theme: {e}"));
                        }
                    }
                } else {
                    self.tui_err("  ✗ Theme source file no longer exists on disk.");
                }
            } else {
                // Built-in theme — re-resolve from settings
                let saved_name = self
                    .settings
                    .lock()
                    .global_settings_mut()
                    .theme
                    .clone()
                    .unwrap_or_else(|| "dark".to_string());
                if let Some(tc) = ThemeColors::builtin_by_name(&saved_name) {
                    self.app.lock().apply_theme(tc);
                    self.tui_ok(format!("  ✓ Theme '{saved_name}' reloaded"));
                } else {
                    self.tui_err(format!("  ✗ Saved theme '{saved_name}' not found"));
                }
            }
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
                // U9: case-insensitive substring fallback — try builtins first
                let name_lower = name.to_lowercase();
                if let Some(&bn) = ThemeColors::builtin_names()
                    .iter()
                    .find(|n| n.to_lowercase().contains(&name_lower))
                {
                    (ThemeColors::builtin_by_name(bn).unwrap(), bn.to_string())
                } else if let Some(t) = discovered
                    .iter()
                    .find(|t| t.name.to_lowercase().contains(&name_lower))
                {
                    (ThemeColors::from_theme(t), t.name.clone())
                } else {
                    (ThemeColors::dark(), String::new())
                }
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
