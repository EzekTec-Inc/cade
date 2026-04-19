//! CADE dynamic theme.
//!
//! Converts `cade_core::resources::themes::ColorDef` to `egui::Color32`.
//!
use egui::Color32;
use cade_core::resources::themes::{ColorDef, ThemeColors as CoreThemeColors};

pub type ThemeColors = CoreThemeColors;

pub trait EguiColorExt {
    fn to_egui(self) -> Color32;
}

impl EguiColorExt for ColorDef {
    fn to_egui(self) -> Color32 {
        match self {
            ColorDef::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
            ColorDef::Reset => Color32::from_rgb(205, 214, 244),
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
}

impl EguiThemeExt for CoreThemeColors {
    fn bg_base(&self) -> Color32 { self.bg_base.to_egui() }
    fn bg_surface0(&self) -> Color32 { self.bg_surface0.to_egui() }
    fn bg_surface1(&self) -> Color32 { self.bg_surface1.to_egui() }
    fn bg_surface2(&self) -> Color32 { self.bg_surface2.to_egui() }
    fn bg_card(&self) -> Color32 { self.bg_card.to_egui() }
    fn bg_input(&self) -> Color32 { self.bg_input.to_egui() }
    fn border_base(&self) -> Color32 { self.border_base.to_egui() }
    fn border_focus(&self) -> Color32 { self.border_focus.to_egui() }
    fn primary(&self) -> Color32 { self.primary.to_egui() }
    fn success(&self) -> Color32 { self.success.to_egui() }
    fn error(&self) -> Color32 { self.error.to_egui() }
    fn warning(&self) -> Color32 { self.warning.to_egui() }
    fn accent_dim(&self) -> Color32 { self.accent_dim.to_egui() }
    fn text_primary(&self) -> Color32 { self.text_primary.to_egui() }
    fn text_muted(&self) -> Color32 { self.text_muted.to_egui() }
    fn text_dim(&self) -> Color32 { self.text_dim.to_egui() }
    fn teal(&self) -> Color32 { self.syntax_type.to_egui() }
    fn purple(&self) -> Color32 { self.syntax_keyword.to_egui() }
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
    use egui::{style::WidgetVisuals, CornerRadius, Stroke, Visuals};

    let rounding_sm = CornerRadius::same(4);
    let rounding_md = CornerRadius::same(6);

    let widget_base = WidgetVisuals {
        bg_fill: theme.bg_surface1(),
        weak_bg_fill: theme.bg_surface0(),
        bg_stroke: Stroke::new(1.0, theme.border_base()),
        corner_radius: rounding_sm,
        fg_stroke: Stroke::new(1.0, theme.text_primary()),
        expansion: 0.0,
    };

    let visuals = Visuals {
        dark_mode: true,
        override_text_color: Some(theme.text_primary()),
        panel_fill: theme.bg_base(),
        window_fill: theme.bg_surface0(),
        window_stroke: Stroke::new(1.0, theme.border_base()),
        window_corner_radius: rounding_md,

        widgets: egui::style::Widgets {
            noninteractive: WidgetVisuals {
                bg_fill: theme.bg_surface0(),
                weak_bg_fill: theme.bg_base(),
                bg_stroke: Stroke::new(1.0, theme.border_base()),
                corner_radius: rounding_sm,
                fg_stroke: Stroke::new(1.0, theme.text_muted()),
                expansion: 0.0,
            },
            inactive: widget_base,
            hovered: WidgetVisuals {
                bg_fill: theme.bg_surface2(),
                weak_bg_fill: theme.bg_surface1(),
                bg_stroke: Stroke::new(1.0, theme.primary()),
                corner_radius: rounding_sm,
                fg_stroke: Stroke::new(1.0, theme.text_primary()),
                expansion: 1.0,
            },
            active: WidgetVisuals {
                bg_fill: theme.accent_dim(),
                weak_bg_fill: theme.bg_surface2(),
                bg_stroke: Stroke::new(1.5, theme.primary()),
                corner_radius: rounding_sm,
                fg_stroke: Stroke::new(1.5, theme.text_primary()),
                expansion: 0.0,
            },
            open: WidgetVisuals {
                bg_fill: theme.bg_surface1(),
                weak_bg_fill: theme.bg_surface0(),
                bg_stroke: Stroke::new(1.0, theme.border_focus()),
                corner_radius: rounding_sm,
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

        window_shadow: egui::Shadow {
            offset: [0, 4],
            blur: 12,
            spread: 0,
            color: Color32::from_black_alpha(80),
        },
        popup_shadow: egui::Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(60),
        },

        ..Visuals::dark()
    };

    ctx.set_visuals(visuals);
}
