//! /theme command handler.
//!
//! Resolution order:
//!   1. Built-in registry (`ThemeColors::builtin_by_name`) — dark, light, etc.
//!   2. User TOML themes discovered in project `.cade/themes/` + `~/.cade/themes/`
//!
//! Both sources are merged for the picker list so built-ins and custom themes
//! appear together with no duplicates.

use super::Repl;
use crate::Result;

impl Repl {
    pub(crate) async fn cmd_theme(&mut self, theme_arg: Option<String>) -> Result<bool> {
        let new_theme = theme_arg.map(|t| t.trim().to_string()).unwrap_or_default();

        // -- Bare `/theme` → open picker
        if new_theme.is_empty() {
            let agent_dir = self
                .settings
                .lock()
                .global_path()
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));

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

        // -- `/theme list` → print available themes inline
        if new_theme == "list" {
            let agent_dir = self
                .settings
                .lock()
                .global_path()
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));

            let current_name = self
                .settings
                .lock()
                .global_settings_mut()
                .theme
                .clone()
                .unwrap_or_else(|| "dark".to_string());

            let discovered =
                cade_core::resources::discover_themes_with_builtins(&self.cwd, &agent_dir);

            self.tui_hdr("Available themes:");
            for t in &discovered {
                let variant = format!("{:?}", t.meta.variant).to_lowercase();
                let marker = if t.meta.name == current_name {
                    " ◀ active"
                } else {
                    ""
                };
                let desc = t.meta.description.as_deref().unwrap_or("");
                let source = "theme";
                self.tui_ok(format!(
                    "  {:<22} ({variant}, {source}) {desc}{marker}",
                    t.meta.name,
                ));
            }
            return Ok(false);
        }

        // -- `/theme reload` → re-read the current theme from disk
        if new_theme == "reload" {
            let saved_name = self
                .settings
                .lock()
                .global_settings_mut()
                .theme
                .clone()
                .unwrap_or_else(|| "dark".to_string());
            if let Some(tc) = cade_core::resources::get_theme(&saved_name) {
                self.app.lock().apply_theme(tc);
                self.tui_ok(format!("  ✓ Theme '{saved_name}' reloaded"));
            } else {
                self.tui_err(format!("  ✗ Saved theme '{saved_name}' not found"));
            }
            return Ok(false);
        }

        // -- `/theme init <name>` → generate starter theme in .cade/themes/<name>.toml
        if let Some(theme_name) = new_theme.strip_prefix("init ") {
            let theme_name = theme_name.trim();
            if theme_name.is_empty() {
                self.tui_err("  ✗ Usage: /theme init <name>");
                return Ok(false);
            }
            let target_dir = self.cwd.join(".cade").join("themes");
            if let Err(e) = std::fs::create_dir_all(&target_dir) {
                self.tui_err(format!("  ✗ Failed to create theme directory: {e}"));
                return Ok(false);
            }
            let target_file = target_dir.join(format!("{theme_name}.toml"));
            if target_file.exists() {
                self.tui_err(format!(
                    "  ✗ Theme file already exists: {}",
                    target_file.display()
                ));
                return Ok(false);
            }
            let template = cade_core::resources::REFERENCE_THEME_TOML
                .replace("name = \"reference\"", &format!("name = \"{theme_name}\""));
            if let Err(e) = std::fs::write(&target_file, template) {
                self.tui_err(format!("  ✗ Failed to write theme file: {e}"));
                return Ok(false);
            }
            self.tui_ok(format!(
                "  ✓ Starter theme initialized at {}",
                target_file.display()
            ));
            self.tui_ok(format!("    Apply with `/theme {theme_name}` or validate with `/theme validate {theme_name}`"));
            return Ok(false);
        }

        // -- `/theme validate [name_or_path]`
        if new_theme == "validate" || new_theme.starts_with("validate ") {
            let target = new_theme
                .strip_prefix("validate ")
                .map(|s| s.trim())
                .unwrap_or("");
            let agent_dir = self
                .settings
                .lock()
                .global_path()
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let resolver = cade_core::resources::ThemeResolver::new(&self.cwd, &agent_dir);

            let report = if target.is_empty() {
                let current_name = self
                    .settings
                    .lock()
                    .global_settings_mut()
                    .theme
                    .clone()
                    .unwrap_or_else(|| "dark".to_string());
                resolver.validate(&current_name)
            } else {
                resolver.validate(target)
            };

            if report.is_valid {
                self.tui_ok(format!("  ✓ Theme '{}' is valid.", report.name));
            } else {
                self.tui_err(format!("  ✗ Theme '{}' failed validation:", report.name));
                for err in &report.errors {
                    self.tui_err(format!("    - {err}"));
                }
            }

            self.tui_ok(format!(
                "    Defined recommended roles: {}/{}",
                report.defined_tokens,
                cade_core::resources::CANONICAL_RECOMMENDED_ROLES.len()
            ));

            if !report.contrast_warnings.is_empty() {
                self.tui_hdr("    Contrast Diagnostics:");
                for warn in &report.contrast_warnings {
                    self.tui_err(format!("      ! {warn}"));
                }
            }

            if !report.missing_recommended_tokens.is_empty() {
                self.tui_hdr("    Missing Recommended Roles (using fallbacks):");
                for missing in &report.missing_recommended_tokens {
                    self.tui_ok(format!("      • {missing}"));
                }
            }

            return Ok(false);
        }

        // -- `/theme inspect [name]`
        if new_theme == "inspect" || new_theme.starts_with("inspect ") {
            let target = new_theme
                .strip_prefix("inspect ")
                .map(|s| s.trim())
                .unwrap_or("");
            let agent_dir = self
                .settings
                .lock()
                .global_path()
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let resolver = cade_core::resources::ThemeResolver::new(&self.cwd, &agent_dir);

            let inspect_name = if target.is_empty() {
                self.settings
                    .lock()
                    .global_settings_mut()
                    .theme
                    .clone()
                    .unwrap_or_else(|| "dark".to_string())
            } else {
                target.to_string()
            };

            let Some(theme) = resolver.resolve(&inspect_name) else {
                self.tui_err(format!("  ✗ Theme '{inspect_name}' not found"));
                return Ok(false);
            };

            self.tui_hdr(format!("Theme Token Resolution for '{}':", theme.meta.name));

            let specs: Vec<(&str, &[&str])> = vec![
                ("bg.base", &[]),
                ("bg.panel", &["cade.user_message_bg"]),
                ("bg.elevated", &["cade.tool_success_bg"]),
                ("bg.highlight", &["cade.selected_bg"]),
                ("bg.selection", &[]),
                ("text.primary", &[]),
                ("text.muted", &[]),
                ("text.dim", &[]),
                ("accent.primary", &[]),
                ("accent.secondary", &[]),
                ("border.unfocused", &["cade.border"]),
                ("border.focused", &["cade.border_accent"]),
                ("success", &["cade.success"]),
                ("warning", &["cade.warning"]),
                ("error", &["cade.error"]),
                ("code.keyword", &["cade.syntax_keyword"]),
                ("code.string", &["cade.syntax_string"]),
                ("code.comment", &["cade.syntax_comment"]),
                ("code.function", &["cade.syntax_function"]),
                ("code.number", &["cade.syntax_number"]),
                ("code.type", &["cade.syntax_type"]),
            ];

            let inspected = resolver.inspect_tokens(&theme, &specs);
            for token in inspected {
                match token {
                    cade_core::resources::ThemeToken::Exact { token, r, g, b } => {
                        self.tui_ok(format!(
                            "  {:<20} #{:02x}{:02x}{:02x}  (exact match)",
                            token, r, g, b
                        ));
                    }
                    cade_core::resources::ThemeToken::Fallback {
                        requested,
                        resolved_token,
                        r,
                        g,
                        b,
                    } => {
                        self.tui_ok(format!(
                            "  {:<20} #{:02x}{:02x}{:02x}  (derived from {})",
                            requested, r, g, b, resolved_token
                        ));
                    }
                    cade_core::resources::ThemeToken::Missing { requested } => {
                        self.tui_err(format!(
                            "  {:<20} [MISSING - using hardcoded default]",
                            requested
                        ));
                    }
                }
            }
            return Ok(false);
        }

        // -- `/theme <name>` → resolve + apply
        let name = new_theme;
        let (target_theme_colors, found_name) =
            if let Some(tc) = cade_core::resources::get_theme(&name) {
                (tc, name.clone())
            } else {
                let agent_dir = self
                    .settings
                    .lock()
                    .global_path()
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let discovered = cade_core::resources::discover_themes(&self.cwd, &agent_dir);
                if let Some(t) = discovered.iter().find(|t| t.meta.name == name) {
                    (t.clone(), t.meta.name.clone())
                } else {
                    // U9: case-insensitive substring fallback — try builtins first
                    let name_lower = name.to_lowercase();
                    let builtins = cade_core::resources::list_available_themes();
                    if let Some(bn) = builtins.iter().find(|n| {
                        n.name.to_lowercase().contains(&name_lower)
                            || n.display_name.to_lowercase().contains(&name_lower)
                    }) {
                        (
                            cade_core::resources::get_theme(&bn.name).unwrap_or_default(),
                            bn.name.to_string(),
                        )
                    } else if let Some(t) = discovered
                        .iter()
                        .find(|t| t.meta.name.to_lowercase().contains(&name_lower))
                    {
                        (t.clone(), t.meta.name.clone())
                    } else {
                        (cade_core::resources::Theme::default(), String::new())
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
