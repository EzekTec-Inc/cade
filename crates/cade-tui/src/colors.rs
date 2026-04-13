/// Theme color palette for the CADE TUI.
use ratatui::style::{Color as RC, Style, Modifier};

// region:    --- ThemeColors

/// Resolved, Ratatui-ready color palette for the TUI.
///
/// Populated from a user-supplied JSON theme (via
/// `cade_core::resources::themes::Theme`) or from the built-in defaults.
#[derive(Clone, Debug)]
pub struct ThemeColors {
    // -- Core
    pub source_path: Option<std::path::PathBuf>,

    // -- Semantic Palette (Phase 1)
    pub bg_base: RC,
    pub bg_surface0: RC,
    pub bg_surface1: RC,
    pub bg_surface2: RC,

    pub primary: RC,
    pub success: RC,
    pub error: RC,
    pub warning: RC,

    pub text_primary: RC,
    pub text_muted: RC,
    pub text_dim: RC,

    pub border_base: RC,
    pub border_focus: RC,

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
    // -- Style Builders (Phase 2)
    pub fn style_base(&self) -> Style { Style::default().bg(self.bg_base).fg(self.text_primary) }
    pub fn style_surface0(&self) -> Style { Style::default().bg(self.bg_surface0).fg(self.text_primary) }
    pub fn style_surface1(&self) -> Style { Style::default().bg(self.bg_surface1).fg(self.text_primary) }
    pub fn style_surface2(&self) -> Style { Style::default().bg(self.bg_surface2).fg(self.text_primary) }

    pub fn text_primary(&self) -> Style { Style::default().fg(self.text_primary) }
    pub fn text_muted(&self) -> Style { Style::default().fg(self.text_muted) }
    pub fn text_dim(&self) -> Style { Style::default().fg(self.text_dim) }
    
    pub fn text_primary_bold(&self) -> Style { Style::default().fg(self.text_primary).add_modifier(Modifier::BOLD) }
    pub fn text_muted_bold(&self) -> Style { Style::default().fg(self.text_muted).add_modifier(Modifier::BOLD) }

    pub fn border_base(&self) -> Style { Style::default().fg(self.border_base) }
    pub fn border_focus(&self) -> Style { Style::default().fg(self.border_focus) }
    
    pub fn primary(&self) -> Style { Style::default().fg(self.primary) }
    pub fn primary_bold(&self) -> Style { Style::default().fg(self.primary).add_modifier(Modifier::BOLD) }
    pub fn success(&self) -> Style { Style::default().fg(self.success) }
    pub fn error(&self) -> Style { Style::default().fg(self.error) }
    pub fn warning(&self) -> Style { Style::default().fg(self.warning) }

    pub fn badge(&self) -> Style { Style::default().bg(self.bg_surface2).fg(self.primary) }

    // -- Built-in themes

    /// Dark theme (modern tonal scaling).
    pub fn dark() -> Self {
        Self {
            source_path: None,
            #[cfg(feature = "syntax-highlighting")]
            syntect_theme: None,

            // Semantic Elevation
            bg_base: RC::Rgb(10, 10, 18),
            bg_surface0: RC::Rgb(18, 22, 32),
            bg_surface1: RC::Rgb(22, 26, 40),
            bg_surface2: RC::Rgb(28, 32, 48),

            primary: RC::Rgb(100, 180, 255),
            success: RC::Rgb(80, 200, 120),
            error: RC::Rgb(220, 80, 80),
            warning: RC::Rgb(240, 180, 60),

            text_primary: RC::Reset,
            text_muted: RC::Rgb(130, 140, 160),
            text_dim: RC::Rgb(80, 88, 110),

            border_base: RC::Rgb(45, 50, 65),
            border_focus: RC::Rgb(100, 180, 255),

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

            // Semantic Elevation
            bg_base: RC::Rgb(250, 252, 255),
            bg_surface0: RC::Rgb(244, 248, 255),
            bg_surface1: RC::Rgb(240, 244, 255),
            bg_surface2: RC::Rgb(230, 236, 250),

            primary: RC::Rgb(0, 100, 200),
            success: RC::Rgb(0, 140, 60),
            error: RC::Rgb(180, 30, 30),
            warning: RC::Rgb(160, 100, 0),

            text_primary: RC::Reset,
            text_muted: RC::Rgb(100, 110, 130),
            text_dim: RC::Rgb(150, 158, 175),

            border_base: RC::Rgb(200, 208, 220),
            border_focus: RC::Rgb(0, 100, 200),

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
    /// Maps the legacy 50+ token schema into the new Semantic tokens.
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
        
        base.primary = resolve(&t.accent);
        
        // Map legacy UI backgrounds to semantic elevations
        // If a user had customized these, we pick approximate matches.
        base.bg_base = resolve(&t.custom_message_bg); 
        base.bg_surface0 = resolve(&t.user_message_bg);
        base.bg_surface1 = resolve(&t.tool_pending_bg);
        base.bg_surface2 = resolve(&t.selected_bg);
        
        base.border_base = resolve(&t.border);
        base.border_focus = resolve(&t.border_accent);
        
        base.text_primary = resolve(&t.text);
        base.text_muted = resolve(&t.muted);
        base.text_dim = resolve(&t.dim);

        base.success = resolve(&t.success);
        base.error = resolve(&t.error);
        base.warning = resolve(&t.warning);

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

        base
    }
}

// endregion: --- ThemeColors

// region:    --- Support

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
        let colors = ThemeColors::dark();
        assert_ne!(colors.primary, RC::Reset);
        assert_ne!(colors.success, RC::Reset);
    }

    #[test]
    fn test_light_theme_smoke() {
        let colors = ThemeColors::light();
        assert_ne!(colors.primary, RC::Reset);
    }

    #[test]
    fn test_parse_hex_valid() {
        assert_eq!(parse_hex("#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex("00ff00"), Some((0, 255, 0)));
    }

    #[test]
    fn test_parse_hex_invalid() {
        assert!(parse_hex("xyz").is_none());
        assert!(parse_hex("#12345").is_none());
    }
}
// endregion: --- Tests
