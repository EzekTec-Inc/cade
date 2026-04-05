/// Theme color palette for the CADE TUI.
///
/// All hardcoded `RC::Rgb(...)` values in `app.rs` derive from one of these
/// tokens.  The `ThemeColors::dark()` and `ThemeColors::light()` constructors
/// reproduce the current defaults so no visual change occurs unless the user
/// explicitly selects a custom theme.
use ratatui::style::Color as RC;

// region:    --- ThemeColors

/// Resolved, Ratatui-ready color palette for the TUI.
///
/// Populated from a user-supplied JSON theme (via
/// `cade_core::resources::themes::Theme`) or from the built-in defaults.
#[derive(Clone, Debug)]
pub struct ThemeColors {
    // -- Core
    pub source_path: Option<std::path::PathBuf>,
    pub accent: RC,
    pub border: RC,
    pub border_accent: RC,
    pub border_muted: RC,
    pub success: RC,
    pub error: RC,
    pub warning: RC,
    pub muted: RC,
    pub dim: RC,
    pub text: RC,
    pub thinking_text: RC,

    // -- Tool boxes
    pub tool_pending_bg: RC,
    pub tool_success_bg: RC,
    pub tool_error_bg: RC,
    pub tool_title: RC,
    pub tool_output: RC,

    // -- User / custom message areas
    pub user_message_bg: RC,
    pub user_message_text: RC,
    pub custom_message_bg: RC,
    pub custom_message_text: RC,
    pub custom_message_label: RC,
    pub selected_bg: RC,

    // -- Modern UI surfaces
    pub overlay_bg: RC,
    pub overlay_border: RC,
    pub overlay_title: RC,
    pub overlay_section: RC,
    pub overlay_hint: RC,
    pub overlay_selected_bg: RC,
    pub overlay_selected_fg: RC,
    pub badge_bg: RC,
    pub badge_fg: RC,
    pub tool_badge_fg: RC,
    pub assistant_accent: RC,
    pub reasoning_bg: RC,

    // -- Diffs
    pub diff_added: RC,
    pub diff_removed: RC,
    pub diff_context: RC,

    // -- Markdown
    pub md_heading: RC,
    pub md_link: RC,
    pub md_link_url: RC,
    pub md_code: RC,
    pub md_code_block: RC,
    pub md_code_block_border: RC,
    pub md_quote: RC,
    pub md_quote_border: RC,
    pub md_hr: RC,
    pub md_list_bullet: RC,

    // -- Syntax highlighting
    pub syntax_comment: RC,
    pub syntax_keyword: RC,
    pub syntax_function: RC,
    pub syntax_variable: RC,
    pub syntax_string: RC,
    pub syntax_number: RC,
    pub syntax_type: RC,
    pub syntax_operator: RC,
    pub syntax_punctuation: RC,

    // -- Thinking level borders
    pub thinking_off: RC,
    pub thinking_minimal: RC,
    pub thinking_low: RC,
    pub thinking_medium: RC,
    pub thinking_high: RC,
    pub thinking_xhigh: RC,

    // -- Bash mode editor border
    pub bash_mode: RC,

    #[cfg(feature = "syntax-highlighting")]
    pub syntect_theme: Option<std::sync::Arc<syntect::highlighting::Theme>>,
}

impl ThemeColors {
    // -- Built-in themes

    /// Dark theme (current default — all values match the previous hardcoded colors).
    pub fn dark() -> Self {
        Self {
            source_path: None,
            #[cfg(feature = "syntax-highlighting")]
            syntect_theme: None,
            accent: RC::Rgb(100, 180, 255),
            border: RC::Rgb(60, 70, 90),
            border_accent: RC::Rgb(100, 180, 255),
            border_muted: RC::Rgb(45, 50, 65),
            success: RC::Rgb(80, 200, 120),
            error: RC::Rgb(220, 80, 80),
            warning: RC::Rgb(240, 180, 60),
            muted: RC::Rgb(130, 140, 160),
            dim: RC::Rgb(80, 88, 110),
            text: RC::Reset,
            thinking_text: RC::Rgb(130, 140, 160),

            tool_pending_bg: RC::Rgb(22, 26, 40),
            tool_success_bg: RC::Rgb(18, 32, 24),
            tool_error_bg: RC::Rgb(38, 18, 18),
            tool_title: RC::Rgb(100, 180, 255),
            tool_output: RC::Reset,

            user_message_bg: RC::Rgb(28, 32, 48),
            user_message_text: RC::Reset,
            custom_message_bg: RC::Rgb(22, 28, 44),
            custom_message_text: RC::Reset,
            custom_message_label: RC::Rgb(100, 180, 255),
            selected_bg: RC::Rgb(38, 42, 60),

            overlay_bg: RC::Rgb(10, 10, 18),
            overlay_border: RC::Rgb(60, 70, 90),
            overlay_title: RC::Rgb(100, 180, 255),
            overlay_section: RC::Rgb(240, 180, 60),
            overlay_hint: RC::Rgb(100, 108, 128),
            overlay_selected_bg: RC::Rgb(28, 32, 48),
            overlay_selected_fg: RC::Rgb(100, 180, 255),
            badge_bg: RC::Rgb(28, 32, 48),
            badge_fg: RC::Rgb(100, 180, 255),
            tool_badge_fg: RC::Rgb(38, 42, 60),
            assistant_accent: RC::Rgb(100, 180, 255),
            reasoning_bg: RC::Rgb(18, 22, 32),

            diff_added: RC::Rgb(80, 200, 120),
            diff_removed: RC::Rgb(220, 80, 80),
            diff_context: RC::Rgb(100, 108, 128),

            md_heading: RC::Rgb(240, 180, 60),
            md_link: RC::Rgb(100, 180, 255),
            md_link_url: RC::Rgb(130, 140, 160),
            md_code: RC::Rgb(120, 210, 210),
            md_code_block: RC::Reset,
            md_code_block_border: RC::Rgb(60, 70, 90),
            md_quote: RC::Rgb(130, 140, 160),
            md_quote_border: RC::Rgb(60, 70, 90),
            md_hr: RC::Rgb(60, 70, 90),
            md_list_bullet: RC::Rgb(120, 210, 210),

            syntax_comment: RC::Rgb(100, 108, 128),
            syntax_keyword: RC::Rgb(200, 120, 220),
            syntax_function: RC::Rgb(100, 180, 255),
            syntax_variable: RC::Rgb(240, 180, 60),
            syntax_string: RC::Rgb(120, 210, 160),
            syntax_number: RC::Rgb(220, 150, 100),
            syntax_type: RC::Rgb(120, 200, 240),
            syntax_operator: RC::Rgb(200, 120, 220),
            syntax_punctuation: RC::Rgb(130, 140, 160),

            thinking_off: RC::Rgb(45, 50, 65),
            thinking_minimal: RC::Rgb(100, 180, 255),
            thinking_low: RC::Rgb(80, 160, 200),
            thinking_medium: RC::Rgb(120, 200, 180),
            thinking_high: RC::Rgb(200, 160, 80),
            thinking_xhigh: RC::Rgb(220, 80, 80),
            bash_mode: RC::Rgb(240, 180, 60),
        }
    }

    /// Light theme.
    pub fn light() -> Self {
        Self {
            source_path: None,
            #[cfg(feature = "syntax-highlighting")]
            syntect_theme: None,
            accent: RC::Rgb(0, 100, 200),
            border: RC::Rgb(180, 190, 210),
            border_accent: RC::Rgb(0, 100, 200),
            border_muted: RC::Rgb(200, 208, 220),
            success: RC::Rgb(0, 140, 60),
            error: RC::Rgb(180, 30, 30),
            warning: RC::Rgb(160, 100, 0),
            muted: RC::Rgb(100, 110, 130),
            dim: RC::Rgb(150, 158, 175),
            text: RC::Reset,
            thinking_text: RC::Rgb(100, 110, 130),

            tool_pending_bg: RC::Rgb(240, 244, 255),
            tool_success_bg: RC::Rgb(230, 248, 236),
            tool_error_bg: RC::Rgb(255, 236, 236),
            tool_title: RC::Rgb(0, 100, 200),
            tool_output: RC::Reset,

            user_message_bg: RC::Rgb(240, 244, 255),
            user_message_text: RC::Reset,
            custom_message_bg: RC::Rgb(244, 248, 255),
            custom_message_text: RC::Reset,
            custom_message_label: RC::Rgb(0, 100, 200),
            selected_bg: RC::Rgb(220, 228, 248),

            overlay_bg: RC::Rgb(250, 252, 255),
            overlay_border: RC::Rgb(180, 190, 210),
            overlay_title: RC::Rgb(0, 100, 200),
            overlay_section: RC::Rgb(160, 100, 0),
            overlay_hint: RC::Rgb(100, 110, 130),
            overlay_selected_bg: RC::Rgb(220, 228, 248),
            overlay_selected_fg: RC::Rgb(0, 100, 200),
            badge_bg: RC::Rgb(230, 236, 250),
            badge_fg: RC::Rgb(0, 100, 200),
            tool_badge_fg: RC::Rgb(220, 228, 248),
            assistant_accent: RC::Rgb(0, 100, 200),
            reasoning_bg: RC::Rgb(238, 242, 250),

            diff_added: RC::Rgb(0, 140, 60),
            diff_removed: RC::Rgb(180, 30, 30),
            diff_context: RC::Rgb(100, 110, 130),

            md_heading: RC::Rgb(160, 100, 0),
            md_link: RC::Rgb(0, 100, 200),
            md_link_url: RC::Rgb(100, 110, 130),
            md_code: RC::Rgb(0, 120, 120),
            md_code_block: RC::Reset,
            md_code_block_border: RC::Rgb(180, 190, 210),
            md_quote: RC::Rgb(100, 110, 130),
            md_quote_border: RC::Rgb(180, 190, 210),
            md_hr: RC::Rgb(180, 190, 210),
            md_list_bullet: RC::Rgb(0, 120, 120),

            syntax_comment: RC::Rgb(100, 110, 130),
            syntax_keyword: RC::Rgb(140, 40, 160),
            syntax_function: RC::Rgb(0, 100, 200),
            syntax_variable: RC::Rgb(160, 100, 0),
            syntax_string: RC::Rgb(0, 120, 60),
            syntax_number: RC::Rgb(160, 80, 20),
            syntax_type: RC::Rgb(0, 100, 160),
            syntax_operator: RC::Rgb(140, 40, 160),
            syntax_punctuation: RC::Rgb(100, 110, 130),

            thinking_off: RC::Rgb(200, 208, 220),
            thinking_minimal: RC::Rgb(0, 100, 200),
            thinking_low: RC::Rgb(0, 120, 160),
            thinking_medium: RC::Rgb(0, 140, 100),
            thinking_high: RC::Rgb(160, 100, 0),
            thinking_xhigh: RC::Rgb(180, 30, 30),
            bash_mode: RC::Rgb(160, 100, 0),
        }
    }

    // -- Custom theme loading

    /// Build `ThemeColors` from a loaded `cade_core::resources::themes::Theme`.
    ///
    /// Returns the dark default for any token that is missing or unresolvable.
    pub fn from_theme(theme: &cade_core::resources::themes::Theme) -> Self {
        let mut base = Self::dark();
        base.source_path = Some(theme.source.clone());

        #[cfg(feature = "syntax-highlighting")]
        if theme.source.extension().and_then(|e| e.to_str()) == Some("tmTheme")
            && let Ok(syn_theme) = syntect::highlighting::ThemeSet::get_theme(&theme.source)
        {
            base.syntect_theme = Some(std::sync::Arc::new(syn_theme));
        }

        let resolve =
            |c: &cade_core::resources::themes::ThemeColor| -> RC { resolve_color(c, &theme.vars) };
        let t = &theme.colors;
        base.accent = resolve(&t.accent);
        base.border = resolve(&t.border);
        base.border_accent = resolve(&t.border_accent);
        base.border_muted = resolve(&t.border_muted);
        base.success = resolve(&t.success);
        base.error = resolve(&t.error);
        base.warning = resolve(&t.warning);
        base.muted = resolve(&t.muted);
        base.dim = resolve(&t.dim);
        base.thinking_text = resolve(&t.thinking_text);
        base.tool_pending_bg = resolve(&t.tool_pending_bg);
        base.tool_success_bg = resolve(&t.tool_success_bg);
        base.tool_error_bg = resolve(&t.tool_error_bg);
        base.tool_title = resolve(&t.tool_title);
        base.diff_added = resolve(&t.tool_diff_added);
        base.diff_removed = resolve(&t.tool_diff_removed);
        base.diff_context = resolve(&t.tool_diff_context);
        base.md_heading = resolve(&t.md_heading);
        base.md_link = resolve(&t.md_link);
        base.md_code = resolve(&t.md_code);
        base.thinking_off = resolve(&t.thinking_off);
        base.thinking_high = resolve(&t.thinking_high);
        base.thinking_xhigh = resolve(&t.thinking_xhigh);
        base.bash_mode = resolve(&t.bash_mode);

        // TUI-only derived surfaces not present in the external theme schema.
        base.overlay_bg = base.custom_message_bg;
        base.overlay_border = base.border;
        base.overlay_title = base.accent;
        base.overlay_section = base.md_heading;
        base.overlay_hint = base.muted;
        base.overlay_selected_bg = base.selected_bg;

        let dark_text = RC::Rgb(20, 20, 20); // Dark text for bright backgrounds

        if is_bright(&base.overlay_selected_bg) {
            base.overlay_selected_fg = dark_text;
        } else {
            base.overlay_selected_fg = base.accent;
        }

        base.badge_bg = base.selected_bg;
        if is_bright(&base.selected_bg) {
            base.badge_fg = dark_text;
        } else {
            base.badge_fg = base.accent;
        }

        if is_bright(&base.tool_pending_bg) {
            base.tool_badge_fg = dark_text;
        } else {
            base.tool_badge_fg = base.selected_bg;
        }

        base.assistant_accent = base.tool_title;
        base.reasoning_bg = base.tool_pending_bg;
        base
    }
}

// endregion: --- ThemeColors

// region:    --- Support

fn is_bright(color: &RC) -> bool {
    if let RC::Rgb(r, g, b) = color {
        // Standard relative luminance formula
        let luminance = 0.299 * (*r as f32) + 0.587 * (*g as f32) + 0.114 * (*b as f32);
        luminance > 128.0
    } else {
        false
    }
}

fn resolve_color(
    c: &cade_core::resources::themes::ThemeColor,
    vars: &std::collections::HashMap<String, cade_core::resources::themes::ThemeColor>,
) -> RC {
    use cade_core::resources::themes::ThemeColor;
    match c {
        ThemeColor::Index(i) => RC::Indexed(*i),
        ThemeColor::Hex(s) if s.is_empty() => RC::Reset,
        ThemeColor::Hex(s) => {
            // Might be a variable reference
            if let Some(var_val) = vars.get(s) {
                return resolve_color(var_val, vars);
            }
            // Try to parse as hex
            if let Some(rgb) = parse_hex(s) {
                return RC::Rgb(rgb.0, rgb.1, rgb.2);
            }
            RC::Reset
        }
    }
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

// endregion: --- Support

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dark_theme_smoke() {
        // -- Exec
        let colors = ThemeColors::dark();
        // -- Check — spot check a few fields are not Reset (accent should be a real color)
        assert_ne!(colors.accent, RC::Reset);
        assert_ne!(colors.success, RC::Reset);
    }

    #[test]
    fn test_light_theme_smoke() {
        // -- Exec
        let colors = ThemeColors::light();
        // -- Check
        assert_ne!(colors.accent, RC::Reset);
    }

    #[test]
    fn test_parse_hex_valid() {
        // -- Exec & Check
        assert_eq!(parse_hex("#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex("00ff00"), Some((0, 255, 0)));
    }

    #[test]
    fn test_parse_hex_invalid() {
        // -- Exec & Check
        assert!(parse_hex("xyz").is_none());
        assert!(parse_hex("#12345").is_none());
    }
}

// endregion: --- Tests
