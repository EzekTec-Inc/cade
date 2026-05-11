//! Responsive layout and styling for different viewports.

use egui::Context;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Viewport {
    /// Phone size (< 640px)
    Mobile,
    /// Tablet size (640px to 1024px)
    Tablet,
    /// Desktop size (>= 1024px)
    Desktop,
}

impl Viewport {
    pub fn is_mobile(self) -> bool {
        matches!(self, Self::Mobile)
    }

    pub fn is_tablet(self) -> bool {
        matches!(self, Self::Tablet)
    }

    pub fn is_desktop(self) -> bool {
        matches!(self, Self::Desktop)
    }
}

/// Detect the viewport type from the current screen width.
pub fn detect(ctx: &Context) -> Viewport {
    let width = ctx.content_rect().width();
    detect_from_width(width)
}

fn detect_from_width(width: f32) -> Viewport {
    if width < 640.0 {
        Viewport::Mobile
    } else if width < 1024.0 {
        Viewport::Tablet
    } else {
        Viewport::Desktop
    }
}

/// Apply responsive spacing and typography tweaks.
/// Call this once per frame before drawing UI.
pub fn apply_style(ctx: &Context, vp: Viewport) {
    let mut style = (*ctx.global_style()).clone();

    // Touch-friendly spacing, font sizes, button heights per breakpoint
    match vp {
        Viewport::Mobile | Viewport::Tablet => {
            // Larger touch targets and padding for touch devices
            style.spacing.interact_size.y = 28.0;
            style.spacing.button_padding = egui::vec2(8.0, 6.0);

            // Boost body text legibility on small screens
            if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Body) {
                if font_id.size < 17.0 {
                    font_id.size = 17.0;
                }
            }
            if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Button) {
                if font_id.size < 17.0 {
                    font_id.size = 17.0;
                }
            }
        }
        Viewport::Desktop => {
            // Standard desktop sizes (egui defaults)
            style.spacing.interact_size.y = 22.0;
            style.spacing.button_padding = egui::vec2(4.0, 2.0);

            if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Body) {
                font_id.size = 14.0; // Typical egui default
            }
            if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Button) {
                font_id.size = 14.0;
            }
        }
    }

    ctx.set_global_style(style);
}

/// Calculate the appropriate rectangle for an overlay modal based on viewport.
pub fn overlay_rect(
    ctx: &Context,
    default_w: f32,
    default_h: f32,
    custom_desktop_y: Option<f32>,
) -> egui::Rect {
    let screen = ctx.content_rect();
    let vp = detect(ctx);
    match vp {
        Viewport::Mobile => screen,
        Viewport::Tablet => {
            let w = screen.width() * 0.9;
            let h = screen.height() * 0.9;
            let x = screen.min.x + (screen.width() - w) / 2.0;
            let y = screen.min.y + (screen.height() - h) / 2.0;
            egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
        }
        Viewport::Desktop => {
            let w = default_w.min(screen.width() - 40.0);
            let h = default_h.min(screen.height() - 40.0);
            let x = screen.min.x + (screen.width() - w) / 2.0;
            let y = custom_desktop_y.unwrap_or_else(|| screen.min.y + (screen.height() - h) / 2.0);
            egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_from_width() {
        assert_eq!(detect_from_width(375.0), Viewport::Mobile);
        assert_eq!(detect_from_width(639.9), Viewport::Mobile);
        assert_eq!(detect_from_width(640.0), Viewport::Tablet);
        assert_eq!(detect_from_width(768.0), Viewport::Tablet);
        assert_eq!(detect_from_width(1023.9), Viewport::Tablet);
        assert_eq!(detect_from_width(1024.0), Viewport::Desktop);
        assert_eq!(detect_from_width(1440.0), Viewport::Desktop);
    }
}
