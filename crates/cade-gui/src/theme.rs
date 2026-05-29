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

fn resolve_fallback(theme: &CoreThemeColors, primary: &str, fallback: &str) -> Color32 {
    let c = theme.color(primary);
    if c.r == 128 && c.g == 128 && c.b == 128 {
        theme.color(fallback).to_egui_color()
    } else {
        c.to_egui_color()
    }
}

impl EguiThemeExt for CoreThemeColors {
    fn bg_base(&self) -> Color32 {
        self.color("bg.base").to_egui_color()
    }
    fn bg_surface0(&self) -> Color32 {
        resolve_fallback(self, "bg.panel", "cade.user_message_bg")
    }
    fn bg_surface1(&self) -> Color32 {
        resolve_fallback(self, "bg.elevated", "cade.tool_success_bg")
    }
    fn bg_surface2(&self) -> Color32 {
        resolve_fallback(self, "bg.highlight", "cade.selected_bg")
    }
    fn bg_card(&self) -> Color32 {
        resolve_fallback(self, "bg.elevated", "cade.tool_success_bg")
    }
    fn bg_input(&self) -> Color32 {
        resolve_fallback(self, "bg.panel", "cade.user_message_bg")
    }
    fn border_base(&self) -> Color32 {
        resolve_fallback(self, "border.unfocused", "cade.border")
    }
    fn border_focus(&self) -> Color32 {
        resolve_fallback(self, "border.focused", "cade.border_accent")
    }
    fn primary(&self) -> Color32 {
        self.color("accent.primary").to_egui_color()
    }
    fn success(&self) -> Color32 {
        resolve_fallback(self, "success", "cade.success")
    }
    fn error(&self) -> Color32 {
        resolve_fallback(self, "error", "cade.error")
    }
    fn warning(&self) -> Color32 {
        resolve_fallback(self, "warning", "cade.warning")
    }
    fn accent_dim(&self) -> Color32 {
        self.color("text.dim").to_egui_color()
    }
    fn text_primary(&self) -> Color32 {
        self.color("text.primary").to_egui_color()
    }
    fn text_muted(&self) -> Color32 {
        self.color("text.muted").to_egui_color()
    }
    fn text_dim(&self) -> Color32 {
        self.color("text.dim").to_egui_color()
    }
    fn teal(&self) -> Color32 {
        resolve_fallback(self, "code.type", "cade.syntax_type")
    }
    fn purple(&self) -> Color32 {
        resolve_fallback(self, "code.keyword", "cade.syntax_keyword")
    }
    fn diff_added(&self) -> Color32 {
        resolve_fallback(self, "success", "cade.success")
    }
    fn diff_removed(&self) -> Color32 {
        resolve_fallback(self, "error", "cade.error")
    }

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

    // Dashboard UI: slight rounding for modern web feel.
    let rounding = CornerRadius::same(4);

    let widget_base = WidgetVisuals {
        bg_fill: theme.bg_surface1(),
        weak_bg_fill: theme.bg_surface0(),
        bg_stroke: Stroke::new(1.0, Color32::from_gray(60)),
        corner_radius: rounding,
        fg_stroke: Stroke::new(1.0, theme.text_primary()),
        expansion: 0.0,
    };

    let visuals = Visuals {
        dark_mode: matches!(theme.meta.variant, opaline::ThemeVariant::Dark),
        override_text_color: Some(Color32::from_rgb(230, 230, 230)),
        panel_fill: Color32::from_rgb(23, 23, 23), // #171717 Sidebar/TopBar
        window_fill: Color32::from_rgb(28, 28, 28), // #1C1C1C
        window_stroke: Stroke::new(1.0, Color32::from_gray(60)),
        window_corner_radius: rounding,

        widgets: egui::style::Widgets {
            noninteractive: WidgetVisuals {
                bg_fill: Color32::from_rgb(28, 28, 28),
                weak_bg_fill: Color32::from_rgb(23, 23, 23),
                bg_stroke: Stroke::new(1.0, Color32::from_gray(60)),
                corner_radius: rounding,
                fg_stroke: Stroke::new(1.0, Color32::from_gray(150)),
                expansion: 0.0,
            },
            inactive: widget_base,
            hovered: WidgetVisuals {
                bg_fill: Color32::from_rgb(40, 40, 40),
                weak_bg_fill: Color32::from_rgb(35, 35, 35),
                bg_stroke: Stroke::new(1.0, Color32::from_gray(100)),
                corner_radius: rounding,
                fg_stroke: Stroke::new(1.0, Color32::from_rgb(255, 255, 255)),
                expansion: 0.0,
            },
            active: WidgetVisuals {
                bg_fill: Color32::from_rgb(50, 50, 50),
                weak_bg_fill: Color32::from_rgb(45, 45, 45),
                bg_stroke: Stroke::new(1.0, Color32::from_gray(120)),
                corner_radius: rounding,
                fg_stroke: Stroke::new(1.0, Color32::from_rgb(255, 255, 255)),
                expansion: 0.0,
            },
            open: WidgetVisuals {
                bg_fill: Color32::from_rgb(35, 35, 35),
                weak_bg_fill: Color32::from_rgb(30, 30, 30),
                bg_stroke: Stroke::new(1.0, Color32::from_gray(80)),
                corner_radius: rounding,
                fg_stroke: Stroke::new(1.0, Color32::from_rgb(255, 255, 255)),
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

        // Dashboard shadows
        window_shadow: egui::Shadow {
            offset: [0, 8],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(96),
        },
        popup_shadow: egui::Shadow {
            offset: [0, 4],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(96),
        },

        ..if matches!(theme.meta.variant, opaline::ThemeVariant::Dark) {
            Visuals::dark()
        } else {
            Visuals::light()
        }
    };

    ctx.set_visuals(visuals);

    // Modern dashboard spacing
    let mut style = (*ctx.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(8);
    style.spacing.indent = 16.0;
    ctx.set_global_style(style);
}
