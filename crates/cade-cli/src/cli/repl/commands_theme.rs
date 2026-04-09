//! /theme command handler.

use crate::Result;
use super::Repl;

impl Repl {
    pub(crate) async fn cmd_theme(
        &mut self,
        theme_arg: Option<String>,
    ) -> Result<bool> {
            let new_theme = if let Some(t) = theme_arg {
                t.trim().to_string()
            } else {
                String::new()
            };
            let name = if new_theme.is_empty() {
                let agent_dir = self
                    .settings
                    .lock()
                    .global_path()
                    .parent()
                    .unwrap()
                    .to_path_buf();
                let mut discovered =
                    cade_core::resources::discover_themes(&self.cwd, &agent_dir);
                if !discovered.iter().any(|t| t.name == "dark") {
                    discovered.insert(
                        0,
                        cade_core::resources::Theme {
                            name: "dark".to_string(),
                            vars: Default::default(),
                            colors: Default::default(),
                            source: std::path::PathBuf::from("builtin"),
                        },
                    );
                }
                if !discovered.iter().any(|t| t.name == "light") {
                    discovered.insert(
                        1,
                        cade_core::resources::Theme {
                            name: "light".to_string(),
                            vars: Default::default(),
                            colors: Default::default(),
                            source: std::path::PathBuf::from("builtin"),
                        },
                    );
                }
                let current_colors =
                    self.app.lock().colors.clone();
                self.app
                    .lock()
                    .open_theme_picker(discovered, current_colors);
                return Ok(false);
            } else {
                new_theme
            };
            let (target_theme_colors, found_name) = if name == "dark" {
                (cade_tui::ThemeColors::dark(), "dark".to_string())
            } else if name == "light" {
                (cade_tui::ThemeColors::light(), "light".to_string())
            } else {
                let agent_dir = self
                    .settings
                    .lock()
                    .global_path()
                    .parent()
                    .unwrap()
                    .to_path_buf();
                let discovered =
                    cade_core::resources::discover_themes(&self.cwd, &agent_dir);
                if let Some(t) = discovered.iter().find(|t| t.name == name) {
                    (cade_tui::ThemeColors::from_theme(t), t.name.clone())
                } else {
                    (cade_tui::ThemeColors::dark(), String::new())
                }
            };
            if found_name.is_empty() {
                self.tui_err(format!("  ✗ Theme '{name}' not found."));
            } else {
                // Apply it dynamically
                {
                    let mut app = self.app.lock();
                    app.apply_theme(target_theme_colors);
                }
                // Save to settings
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
