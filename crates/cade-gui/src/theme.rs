//! CADE dark theme — matches the TUI palette for visual consistency.
//!
//! Colours are extracted from `cade-tui/src/colors.rs  ThemeColors::dark()`.
//! Every value here is a plain `egui::Color32` constant so we pay zero
//! allocation and the theme is fully determined at compile time.

use egui::Color32;

// ── Backgrounds (semantic elevation) ────────────────────────────────────

/// Near-void blue-black — the deepest background.
pub const BG_BASE: Color32 = Color32::from_rgb(12, 13, 20);
/// Card base — panels, sidebar.
pub const BG_SURFACE0: Color32 = Color32::from_rgb(20, 22, 33);
/// Overlay / popup base.
pub const BG_SURFACE1: Color32 = Color32::from_rgb(26, 28, 42);
/// Selection highlight / hover.
pub const BG_SURFACE2: Color32 = Color32::from_rgb(34, 36, 54);
/// Card background for message bubbles.
pub const BG_CARD: Color32 = Color32::from_rgb(20, 22, 34);
/// Input area background.
pub const BG_INPUT: Color32 = Color32::from_rgb(22, 24, 36);

// ── Accent colours ──────────────────────────────────────────────────────

/// Vivid sky blue — primary accent.
pub const PRIMARY: Color32 = Color32::from_rgb(122, 162, 247);
/// Fresh green — success.
pub const SUCCESS: Color32 = Color32::from_rgb(73, 196, 127);
/// Soft coral — error.
pub const ERROR: Color32 = Color32::from_rgb(247, 93, 100);
/// Warm amber — warning / headings.
pub const WARNING: Color32 = Color32::from_rgb(224, 175, 104);
/// Desaturated accent for secondary emphasis.
pub const ACCENT_DIM: Color32 = Color32::from_rgb(64, 102, 168);

// ── Text ────────────────────────────────────────────────────────────────

/// Default text — bright white.
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(205, 214, 244);
/// Mid-grey, blue-tinted — secondary text.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(122, 128, 153);
/// Dark hint text.
pub const TEXT_DIM: Color32 = Color32::from_rgb(72, 78, 98);

// ── Borders ─────────────────────────────────────────────────────────────

/// Barely-visible divider.
pub const BORDER_BASE: Color32 = Color32::from_rgb(41, 44, 64);
/// Focus / active border — matches primary.
pub const BORDER_FOCUS: Color32 = Color32::from_rgb(122, 162, 247);

// ── Markdown / syntax ───────────────────────────────────────────────────

/// Teal — code, types.
pub const TEAL: Color32 = Color32::from_rgb(115, 218, 202);
/// Purple — keywords.
pub const PURPLE: Color32 = Color32::from_rgb(187, 154, 247);

// ── Role colours (for message headers) ──────────────────────────────────

/// User message accent — a warm neutral.
pub const ROLE_USER: Color32 = Color32::from_rgb(205, 214, 244);
/// Assistant message accent — primary blue.
pub const ROLE_ASSISTANT: Color32 = PRIMARY;
/// System message accent — dim.
pub const ROLE_SYSTEM: Color32 = TEXT_MUTED;
/// Tool message accent — teal.
pub const ROLE_TOOL: Color32 = TEAL;

// ── Helpers ─────────────────────────────────────────────────────────────

/// Apply the CADE dark theme to egui's `Visuals`.
///
/// Call this once in `CadeApp::new` (or on every frame if hot-reloading).
pub fn apply_theme(ctx: &egui::Context) {
    use egui::{
        self, style::WidgetVisuals, CornerRadius, Stroke, Visuals,
    };

    let rounding_sm = CornerRadius::same(4);
    let rounding_md = CornerRadius::same(6);

    let widget_base = WidgetVisuals {
        bg_fill: BG_SURFACE1,
        weak_bg_fill: BG_SURFACE0,
        bg_stroke: Stroke::new(1.0, BORDER_BASE),
        corner_radius: rounding_sm,
        fg_stroke: Stroke::new(1.0, TEXT_PRIMARY),
        expansion: 0.0,
    };

    let visuals = Visuals {
        dark_mode: true,
        override_text_color: Some(TEXT_PRIMARY),
        panel_fill: BG_BASE,
        window_fill: BG_SURFACE0,
        window_stroke: Stroke::new(1.0, BORDER_BASE),
        window_corner_radius: rounding_md,

        widgets: egui::style::Widgets {
            noninteractive: WidgetVisuals {
                bg_fill: BG_SURFACE0,
                weak_bg_fill: BG_BASE,
                bg_stroke: Stroke::new(1.0, BORDER_BASE),
                corner_radius: rounding_sm,
                fg_stroke: Stroke::new(1.0, TEXT_MUTED),
                expansion: 0.0,
            },
            inactive: widget_base,
            hovered: WidgetVisuals {
                bg_fill: BG_SURFACE2,
                weak_bg_fill: BG_SURFACE1,
                bg_stroke: Stroke::new(1.0, PRIMARY),
                corner_radius: rounding_sm,
                fg_stroke: Stroke::new(1.0, TEXT_PRIMARY),
                expansion: 1.0,
            },
            active: WidgetVisuals {
                bg_fill: ACCENT_DIM,
                weak_bg_fill: BG_SURFACE2,
                bg_stroke: Stroke::new(1.5, PRIMARY),
                corner_radius: rounding_sm,
                fg_stroke: Stroke::new(1.5, TEXT_PRIMARY),
                expansion: 0.0,
            },
            open: WidgetVisuals {
                bg_fill: BG_SURFACE1,
                weak_bg_fill: BG_SURFACE0,
                bg_stroke: Stroke::new(1.0, BORDER_FOCUS),
                corner_radius: rounding_sm,
                fg_stroke: Stroke::new(1.0, TEXT_PRIMARY),
                expansion: 0.0,
            },
        },

        selection: egui::style::Selection {
            bg_fill: ACCENT_DIM,
            stroke: Stroke::new(1.0, PRIMARY),
        },

        hyperlink_color: PRIMARY,
        faint_bg_color: BG_SURFACE0,
        extreme_bg_color: BG_BASE,
        code_bg_color: BG_SURFACE1,

        warn_fg_color: WARNING,
        error_fg_color: ERROR,

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

        // Keep defaults for the rest.
        ..Visuals::dark()
    };

    ctx.set_visuals(visuals);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_constants_are_non_black() {
        // Smoke test — every named colour should be something visible.
        assert_ne!(PRIMARY, Color32::BLACK);
        assert_ne!(TEXT_PRIMARY, Color32::BLACK);
        assert_ne!(BG_BASE, Color32::BLACK);
        assert_ne!(SUCCESS, Color32::BLACK);
    }

    #[test]
    fn bg_elevation_is_ordered() {
        // Each surface should be visually brighter than the one below it.
        // Compare the R channel as a proxy for luminance.
        assert!(BG_SURFACE0.r() > BG_BASE.r());
        assert!(BG_SURFACE1.r() > BG_SURFACE0.r());
        assert!(BG_SURFACE2.r() > BG_SURFACE1.r());
    }
}
