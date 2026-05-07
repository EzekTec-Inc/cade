//! CADE dynamic theme.
//!
//! Exposes opaline::Theme methods as egui::Color32 for UI components.
//!
use cade_core::resources::Theme as CoreThemeColors;
use egui::Color32;

pub type ThemeColors = CoreThemeColors;

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


trait OpalineColorEguiExt {
    fn to_egui_color(&self) -> Color32;
}

impl OpalineColorEguiExt for opaline::color::OpalineColor {
    fn to_egui_color(&self) -> Color32 {
        Color32::from_rgb(self.r, self.g, self.b)
    }
}

impl EguiThemeExt for CoreThemeColors {
    fn bg_base(&self) -> Color32 { self.color("bg.base").to_egui_color() }
    fn bg_surface0(&self) -> Color32 { self.color("bg.panel").to_egui_color() }
    fn bg_surface1(&self) -> Color32 { self.color("bg.elevated").to_egui_color() }
    fn bg_surface2(&self) -> Color32 { self.color("bg.highlight").to_egui_color() }
    fn bg_card(&self) -> Color32 { self.color("cade.bg_card").to_egui_color() }
    fn bg_input(&self) -> Color32 { self.color("bg.panel").to_egui_color() }
    fn border_base(&self) -> Color32 { self.color("border.unfocused").to_egui_color() }
    fn border_focus(&self) -> Color32 { self.color("border.focused").to_egui_color() }
    fn primary(&self) -> Color32 { self.color("accent.primary").to_egui_color() }
    fn success(&self) -> Color32 { self.color("success").to_egui_color() }
    fn error(&self) -> Color32 { self.color("error").to_egui_color() }
    fn warning(&self) -> Color32 { self.color("warning").to_egui_color() }
    fn accent_dim(&self) -> Color32 { self.color("text.dim").to_egui_color() }
    fn text_primary(&self) -> Color32 { self.color("text.primary").to_egui_color() }
    fn text_muted(&self) -> Color32 { self.color("text.muted").to_egui_color() }
    fn text_dim(&self) -> Color32 { self.color("text.dim").to_egui_color() }
    fn teal(&self) -> Color32 { self.color("cade.syntax_type").to_egui_color() }
    fn purple(&self) -> Color32 { self.color("cade.syntax_keyword").to_egui_color() }
    fn diff_added(&self) -> Color32 { self.color("cade.tool_diff_added").to_egui_color() }
    fn diff_removed(&self) -> Color32 { self.color("cade.tool_diff_removed").to_egui_color() }
    
    fn overlay_backdrop(&self) -> Color32 {
        let base = self.bg_base();
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
        dark_mode: match theme.meta.variant {
            opaline::ThemeVariant::Dark => true,
            _ => false,
        },
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

        ..if match theme.meta.variant { opaline::ThemeVariant::Dark => true, _ => false } { Visuals::dark() } else { Visuals::light() }
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