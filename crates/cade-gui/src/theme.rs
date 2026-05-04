//! CADE dynamic theme.
//!
//! Converts `cade_core::resources::themes::ColorDef` to `egui::Color32`.
//!
use cade_core::resources::themes::{ColorDef, ThemeColors as CoreThemeColors};
use egui::Color32;

pub type ThemeColors = CoreThemeColors;

pub trait EguiColorExt {
    fn to_egui(self) -> Color32;
}

impl EguiColorExt for ColorDef {
    fn to_egui(self) -> Color32 {
        match self {
            ColorDef::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
            // `ColorDef::Reset` means "inherit terminal default" in TUI land.
            // In the GUI there is no terminal default, so we pick a neutral
            // grey (RGB 130) that remains readable on both dark and light
            // surfaces.  Themes that care about contrast on white should
            // use explicit `ColorDef::Rgb(..)` instead — see the
            // `light_theme_text_colors_are_readable_on_white` regression.
            ColorDef::Reset => Color32::from_rgb(130, 130, 130),
        }
    }
}

pub trait EguiThemeExt {
    fn bg_base(&self) -> Color32;
    fn bg_surface0(&self) -> Color32;
    fn bg_surface1(&self) -> Color32;
    fn bg_surface2(&self) -> Color32;
    fn bg_card(&self) -> Color32;
    fn bg_input(&self) -> Color32;
    fn border_base(&self) -> Color32;
    fn border_focus(&self) -> Color32;
    fn primary(&self) -> Color32;
    fn success(&self) -> Color32;
    fn error(&self) -> Color32;
    fn warning(&self) -> Color32;
    fn accent_dim(&self) -> Color32;
    fn text_primary(&self) -> Color32;
    fn text_muted(&self) -> Color32;
    fn text_dim(&self) -> Color32;
    fn teal(&self) -> Color32;
    fn purple(&self) -> Color32;

    // -- Diff-specific colors (use theme's diff_* slots instead of success/error)
    fn diff_added(&self) -> Color32;
    fn diff_removed(&self) -> Color32;

    /// Return a low-alpha (10-20 range) version of a base color, useful
    /// for highlight backgrounds (diff rows, subagent cards, etc.).
    fn tinted_bg(&self, base: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha)
    }

    /// Semi-transparent overlay backdrop derived from the theme's base colour.
    ///
    /// Dark themes get a near-black backdrop; light themes get a near-white one.
    /// Both use ~55% opacity (alpha ≈ 140).
    fn overlay_backdrop(&self) -> Color32;
}

impl EguiThemeExt for CoreThemeColors {
    fn bg_base(&self) -> Color32 {
        self.bg_base.to_egui()
    }
    fn bg_surface0(&self) -> Color32 {
        self.bg_surface0.to_egui()
    }
    fn bg_surface1(&self) -> Color32 {
        self.bg_surface1.to_egui()
    }
    fn bg_surface2(&self) -> Color32 {
        self.bg_surface2.to_egui()
    }
    fn bg_card(&self) -> Color32 {
        self.bg_card.to_egui()
    }
    fn bg_input(&self) -> Color32 {
        self.bg_input.to_egui()
    }
    fn border_base(&self) -> Color32 {
        self.border_base.to_egui()
    }
    fn border_focus(&self) -> Color32 {
        self.border_focus.to_egui()
    }
    fn primary(&self) -> Color32 {
        self.primary.to_egui()
    }
    fn success(&self) -> Color32 {
        self.success.to_egui()
    }
    fn error(&self) -> Color32 {
        self.error.to_egui()
    }
    fn warning(&self) -> Color32 {
        self.warning.to_egui()
    }
    fn accent_dim(&self) -> Color32 {
        self.accent_dim.to_egui()
    }
    fn text_primary(&self) -> Color32 {
        self.text_primary.to_egui()
    }
    fn text_muted(&self) -> Color32 {
        self.text_muted.to_egui()
    }
    fn text_dim(&self) -> Color32 {
        self.text_dim.to_egui()
    }
    fn teal(&self) -> Color32 {
        self.syntax_type.to_egui()
    }
    fn purple(&self) -> Color32 {
        self.syntax_keyword.to_egui()
    }
    fn diff_added(&self) -> Color32 {
        self.diff_added.to_egui()
    }
    fn diff_removed(&self) -> Color32 {
        self.diff_removed.to_egui()
    }
    fn overlay_backdrop(&self) -> Color32 {
        let base = self.bg_base.to_egui();
        Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 140)
    }
}

/// Fraction of the context window consumed by `total_tokens`, clamped 0.0-1.0.
pub fn context_fill_fraction(total_tokens: u64, window: u64) -> f32 {
    if window == 0 {
        return 0.0;
    }
    (total_tokens as f32 / window as f32).clamp(0.0, 1.0)
}

/// Colour for the context-window progress bar based on fill fraction.
pub fn context_fill_color(fraction: f32, theme: &CoreThemeColors) -> Color32 {
    if fraction >= 0.85 {
        theme.error()
    } else if fraction >= 0.60 {
        theme.warning()
    } else {
        theme.success()
    }
}

pub fn apply_theme(ctx: &egui::Context, theme: &CoreThemeColors) {
    use egui::{CornerRadius, Stroke, Visuals, style::WidgetVisuals};

    // TUI-fication: zero rounding for sharp, terminal-like edges.
    let rounding_none = CornerRadius::ZERO;

    let widget_base = WidgetVisuals {
        bg_fill: theme.bg_surface1(),
        weak_bg_fill: theme.bg_surface0(),
        bg_stroke: Stroke::new(1.0, theme.border_base()),
        corner_radius: rounding_none,
        fg_stroke: Stroke::new(1.0, theme.text_primary()),
        expansion: 0.0,
    };

    let visuals = Visuals {
        dark_mode: !theme.is_light(),
        override_text_color: Some(theme.text_primary()),
        panel_fill: theme.bg_base(),
        window_fill: theme.bg_surface0(),
        window_stroke: Stroke::new(1.0, theme.border_base()),
        window_corner_radius: rounding_none,

        widgets: egui::style::Widgets {
            noninteractive: WidgetVisuals {
                bg_fill: theme.bg_surface0(),
                weak_bg_fill: theme.bg_base(),
                bg_stroke: Stroke::new(1.0, theme.border_base()),
                corner_radius: rounding_none,
                fg_stroke: Stroke::new(1.0, theme.text_muted()),
                expansion: 0.0,
            },
            inactive: widget_base,
            hovered: WidgetVisuals {
                bg_fill: theme.bg_surface2(),
                weak_bg_fill: theme.bg_surface1(),
                bg_stroke: Stroke::new(1.0, theme.primary()),
                corner_radius: rounding_none,
                fg_stroke: Stroke::new(1.0, theme.text_primary()),
                expansion: 0.0, // no hover expansion — keeps it tight like TUI
            },
            active: WidgetVisuals {
                bg_fill: theme.accent_dim(),
                weak_bg_fill: theme.bg_surface2(),
                bg_stroke: Stroke::new(1.0, theme.primary()),
                corner_radius: rounding_none,
                fg_stroke: Stroke::new(1.0, theme.text_primary()),
                expansion: 0.0,
            },
            open: WidgetVisuals {
                bg_fill: theme.bg_surface1(),
                weak_bg_fill: theme.bg_surface0(),
                bg_stroke: Stroke::new(1.0, theme.border_focus()),
                corner_radius: rounding_none,
                fg_stroke: Stroke::new(1.0, theme.text_primary()),
                expansion: 0.0,
            },
        },

        selection: egui::style::Selection {
            bg_fill: theme.accent_dim(),
            stroke: Stroke::new(1.0, theme.primary()),
        },

        hyperlink_color: theme.primary(),
        faint_bg_color: theme.bg_surface0(),
        extreme_bg_color: theme.bg_base(),
        code_bg_color: theme.bg_surface1(),

        warn_fg_color: theme.warning(),
        error_fg_color: theme.error(),

        // TUI-fication: no shadows — flat, terminal-like appearance.
        window_shadow: egui::Shadow::NONE,
        popup_shadow: egui::Shadow::NONE,

        ..if theme.is_light() { Visuals::light() } else { Visuals::dark() }
    };

    ctx.set_visuals(visuals);

    // TUI-fication: tighten spacing to match character-cell density.
    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(4.0, 2.0);
    style.spacing.button_padding = egui::vec2(4.0, 2.0);
    style.spacing.window_margin = egui::Margin::same(4);
    style.spacing.indent = 12.0;
    ctx.set_global_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dark theme's error color must come from the theme system, not a
    /// hardcoded literal.  Previously `ConnectionFailed` used
    /// `Color32::from_rgb(220, 50, 50)` — this test ensures that value is
    /// NOT what the theme produces, proving the render path uses themed
    /// colors.
    #[test]
    fn dark_theme_error_is_not_old_hardcoded_value() {
        let theme = ThemeColors::dark();
        let themed_error = theme.error();
        let old_hardcoded = Color32::from_rgb(220, 50, 50);
        assert_ne!(
            themed_error, old_hardcoded,
            "error color should come from the theme, not the old hardcoded (220,50,50)"
        );
    }

    /// The themed error color must be the core theme's error field converted
    /// to egui, ensuring `EguiThemeExt::error()` is wired correctly.
    #[test]
    fn themed_error_matches_core_color_def() {
        let theme = ThemeColors::dark();
        let expected = theme.error.to_egui();
        let actual = theme.error();
        assert_eq!(actual, expected);
    }
}
