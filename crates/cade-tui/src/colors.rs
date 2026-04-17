/// Theme color palette for the CADE TUI.
use ratatui::style::{Color as RC, Style, Modifier};

// region:    --- BorderStyle

/// Which ratatui `BorderType` the theme prefers for all overlay blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    Rounded,
    Thick,
    Plain,
    Double,
}

impl BorderStyle {
    /// Convert to the corresponding ratatui `BorderType`.
    pub fn to_ratatui(self) -> ratatui::widgets::BorderType {
        use ratatui::widgets::BorderType;
        match self {
            Self::Rounded => BorderType::Rounded,
            Self::Thick   => BorderType::Thick,
            Self::Plain   => BorderType::Plain,
            Self::Double  => BorderType::Double,
        }
    }
}

// endregion: --- BorderStyle

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

    // -- Extended surface tokens (Step 1)
    /// Preferred border character style for all overlay blocks.
    pub border_style: BorderStyle,
    /// Subtle card background for tool-result / message cards.
    pub bg_card: RC,
    /// Input area background (textarea row).
    pub bg_input: RC,
    /// Desaturated accent for secondary emphasis (badges, counts).
    pub accent_dim: RC,

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

    pub fn diff_added(&self) -> Style { Style::default().fg(self.diff_added) }
    pub fn diff_removed(&self) -> Style { Style::default().fg(self.diff_removed) }
    pub fn diff_context(&self) -> Style { Style::default().fg(self.diff_context) }

    pub fn md_heading(&self) -> Style { Style::default().fg(self.md_heading) }
    pub fn md_link(&self) -> Style { Style::default().fg(self.md_link) }
    pub fn md_link_url(&self) -> Style { Style::default().fg(self.md_link_url) }
    pub fn md_code(&self) -> Style { Style::default().fg(self.md_code) }
    pub fn md_code_block(&self) -> Style { Style::default().fg(self.md_code_block) }
    pub fn md_code_block_border(&self) -> Style { Style::default().fg(self.md_code_block_border) }
    pub fn md_quote(&self) -> Style { Style::default().fg(self.md_quote) }
    pub fn md_quote_border(&self) -> Style { Style::default().fg(self.md_quote_border) }
    pub fn md_hr(&self) -> Style { Style::default().fg(self.md_hr) }
    pub fn md_list_bullet(&self) -> Style { Style::default().fg(self.md_list_bullet) }

    pub fn syntax_comment(&self) -> Style { Style::default().fg(self.syntax_comment) }
    pub fn syntax_keyword(&self) -> Style { Style::default().fg(self.syntax_keyword) }
    pub fn syntax_function(&self) -> Style { Style::default().fg(self.syntax_function) }
    pub fn syntax_variable(&self) -> Style { Style::default().fg(self.syntax_variable) }
    pub fn syntax_string(&self) -> Style { Style::default().fg(self.syntax_string) }
    pub fn syntax_number(&self) -> Style { Style::default().fg(self.syntax_number) }
    pub fn syntax_type(&self) -> Style { Style::default().fg(self.syntax_type) }
    pub fn syntax_operator(&self) -> Style { Style::default().fg(self.syntax_operator) }
    pub fn syntax_punctuation(&self) -> Style { Style::default().fg(self.syntax_punctuation) }

    pub fn thinking_off(&self) -> Style { Style::default().fg(self.thinking_off) }
    pub fn thinking_minimal(&self) -> Style { Style::default().fg(self.thinking_minimal) }
    pub fn thinking_low(&self) -> Style { Style::default().fg(self.thinking_low) }
    pub fn thinking_medium(&self) -> Style { Style::default().fg(self.thinking_medium) }
    pub fn thinking_high(&self) -> Style { Style::default().fg(self.thinking_high) }
    pub fn thinking_xhigh(&self) -> Style { Style::default().fg(self.thinking_xhigh) }

    pub fn bash_mode(&self) -> Style { Style::default().fg(self.bash_mode) }

    // -- Built-in themes

    /// Dark theme (deep blue-black, high contrast, rich depth).
    pub fn dark() -> Self {
        Self {
            source_path: None,
            #[cfg(feature = "syntax-highlighting")]
            syntect_theme: None,

            // Semantic Elevation — noticeable depth between layers
            bg_base:     RC::Rgb(12,  13,  20),   // near-void blue-black
            bg_surface0: RC::Rgb(20,  22,  33),   // card base  (+8 step)
            bg_surface1: RC::Rgb(26,  28,  42),   // overlay base (+6 step)
            bg_surface2: RC::Rgb(34,  36,  54),   // selection highlight (+8 step)

            primary: RC::Rgb(122, 162, 247),   // vivid sky blue
            success: RC::Rgb( 73, 196, 127),   // fresh green
            error:   RC::Rgb(247,  93, 100),   // soft coral
            warning: RC::Rgb(224, 175, 104),   // warm amber

            text_primary: RC::Reset,
            text_muted:   RC::Rgb(122, 128, 153),  // mid-grey, blue-tinted
            text_dim:     RC::Rgb( 72,  78,  98),  // dark hint text

            border_base:  RC::Rgb( 41,  44,  64),  // barely-visible divider
            border_focus: RC::Rgb(122, 162, 247),  // matches primary

            diff_added:   RC::Rgb( 73, 196, 127),
            diff_removed: RC::Rgb(247,  93, 100),
            diff_context: RC::Rgb( 90,  98, 120),

            md_heading:          RC::Rgb(224, 175, 104),
            md_link:             RC::Rgb(122, 162, 247),
            md_link_url:         RC::Rgb(122, 128, 153),
            md_code:             RC::Rgb(115, 218, 202),
            md_code_block:       RC::Reset,
            md_code_block_border: RC::Rgb(48,  52,  72),
            md_quote:            RC::Rgb(122, 128, 153),
            md_quote_border:     RC::Rgb(48,  52,  72),
            md_hr:               RC::Rgb(48,  52,  72),
            md_list_bullet:      RC::Rgb(115, 218, 202),

            syntax_comment:     RC::Rgb( 90,  98, 120),
            syntax_keyword:     RC::Rgb(187, 154, 247),  // purple
            syntax_function:    RC::Rgb(122, 162, 247),  // blue
            syntax_variable:    RC::Rgb(224, 175, 104),  // amber
            syntax_string:      RC::Rgb(158, 206, 106),  // green
            syntax_number:      RC::Rgb(255, 158, 100),  // orange
            syntax_type:        RC::Rgb(115, 218, 202),  // teal
            syntax_operator:    RC::Rgb(187, 154, 247),
            syntax_punctuation: RC::Rgb(122, 128, 153),

            thinking_off:     RC::Rgb( 41,  44,  64),
            thinking_minimal: RC::Rgb(122, 162, 247),
            thinking_low:     RC::Rgb( 80, 160, 200),
            thinking_medium:  RC::Rgb(115, 218, 202),
            thinking_high:    RC::Rgb(224, 175, 104),
            thinking_xhigh:   RC::Rgb(247,  93, 100),
            bash_mode:        RC::Rgb(224, 175, 104),

            border_style: BorderStyle::Rounded,
            bg_card:      RC::Rgb(20,  22,  34),
            bg_input:     RC::Rgb(22,  24,  36),
            accent_dim:   RC::Rgb(64, 102, 168),
        }
    }

    /// Light theme (warm white base, high readability, clear depth).
    pub fn light() -> Self {
        Self {
            source_path: None,
            #[cfg(feature = "syntax-highlighting")]
            syntect_theme: None,

            // Semantic Elevation — clear layering on a white surface
            bg_base:     RC::Rgb(252, 252, 255),   // near-white, slight blue
            bg_surface0: RC::Rgb(244, 246, 255),   // card base
            bg_surface1: RC::Rgb(236, 240, 255),   // overlay / panel
            bg_surface2: RC::Rgb(220, 226, 248),   // selection highlight

            primary: RC::Rgb( 14,  98, 200),   // rich blue
            success: RC::Rgb(  0, 135,  75),   // forest green
            error:   RC::Rgb(185,  28,  28),   // deep red
            warning: RC::Rgb(146,  88,   0),   // dark amber

            text_primary: RC::Reset,
            text_muted:   RC::Rgb( 90, 100, 125),
            text_dim:     RC::Rgb(148, 158, 180),

            border_base:  RC::Rgb(198, 206, 226),
            border_focus: RC::Rgb( 14,  98, 200),

            diff_added:   RC::Rgb(  0, 135,  75),
            diff_removed: RC::Rgb(185,  28,  28),
            diff_context: RC::Rgb( 90, 100, 125),

            md_heading:          RC::Rgb(146,  88,   0),
            md_link:             RC::Rgb( 14,  98, 200),
            md_link_url:         RC::Rgb( 90, 100, 125),
            md_code:             RC::Rgb(  0, 118, 118),
            md_code_block:       RC::Reset,
            md_code_block_border: RC::Rgb(180, 190, 215),
            md_quote:            RC::Rgb( 90, 100, 125),
            md_quote_border:     RC::Rgb(180, 190, 215),
            md_hr:               RC::Rgb(180, 190, 215),
            md_list_bullet:      RC::Rgb(  0, 118, 118),

            syntax_comment:     RC::Rgb( 90, 100, 125),
            syntax_keyword:     RC::Rgb(128,  30, 155),
            syntax_function:    RC::Rgb( 14,  98, 200),
            syntax_variable:    RC::Rgb(146,  88,   0),
            syntax_string:      RC::Rgb(  0, 115,  55),
            syntax_number:      RC::Rgb(155,  70,  15),
            syntax_type:        RC::Rgb(  0,  95, 155),
            syntax_operator:    RC::Rgb(128,  30, 155),
            syntax_punctuation: RC::Rgb( 90, 100, 125),

            thinking_off:     RC::Rgb(198, 206, 226),
            thinking_minimal: RC::Rgb( 14,  98, 200),
            thinking_low:     RC::Rgb(  0, 118, 160),
            thinking_medium:  RC::Rgb(  0, 135,  75),
            thinking_high:    RC::Rgb(146,  88,   0),
            thinking_xhigh:   RC::Rgb(185,  28,  28),
            bash_mode:        RC::Rgb(146,  88,   0),

            border_style: BorderStyle::Rounded,
            bg_card:      RC::Rgb(242, 245, 255),
            bg_input:     RC::Rgb(236, 240, 255),
            accent_dim:   RC::Rgb( 76, 136, 210),
        }
    }

    // -- Additional built-in themes

    /// Catppuccin Mocha — warm purple-tinted dark.
    /// Palette source: <https://github.com/catppuccin/catppuccin>
    pub fn catppuccin_mocha() -> Self {
        let mut c = Self::dark();
        // Base surfaces
        c.bg_base     = RC::Rgb( 30,  30,  46);  // Crust
        c.bg_surface0 = RC::Rgb( 36,  36,  54);  // Mantle
        c.bg_surface1 = RC::Rgb( 49,  50,  68);  // Base
        c.bg_surface2 = RC::Rgb( 69,  71,  90);  // Surface0
        c.bg_card     = RC::Rgb( 36,  36,  54);
        c.bg_input    = RC::Rgb( 49,  50,  68);
        // Accents
        c.primary      = RC::Rgb(137, 180, 250);  // Blue
        c.success      = RC::Rgb(166, 227, 161);  // Green
        c.error        = RC::Rgb(243, 139, 168);  // Red
        c.warning      = RC::Rgb(249, 226, 175);  // Yellow
        c.accent_dim   = RC::Rgb( 88, 128, 200);
        // Text
        c.text_muted   = RC::Rgb(166, 173, 200);  // Overlay2
        c.text_dim     = RC::Rgb(108, 112, 134);  // Surface2
        // Borders
        c.border_base  = RC::Rgb( 69,  71,  90);
        c.border_focus = RC::Rgb(137, 180, 250);
        // Diff
        c.diff_added   = RC::Rgb(166, 227, 161);
        c.diff_removed = RC::Rgb(243, 139, 168);
        // Markdown
        c.md_heading    = RC::Rgb(249, 226, 175);
        c.md_link       = RC::Rgb(137, 180, 250);
        c.md_code       = RC::Rgb(148, 226, 213);  // Teal
        c.md_list_bullet = RC::Rgb(148, 226, 213);
        // Syntax
        c.syntax_keyword    = RC::Rgb(203, 166, 247);  // Mauve
        c.syntax_function   = RC::Rgb(137, 180, 250);  // Blue
        c.syntax_string     = RC::Rgb(166, 227, 161);  // Green
        c.syntax_number     = RC::Rgb(250, 179, 135);  // Peach
        c.syntax_type       = RC::Rgb(148, 226, 213);  // Teal
        c.syntax_variable   = RC::Rgb(249, 226, 175);  // Yellow
        c.syntax_operator   = RC::Rgb(203, 166, 247);
        // Thinking
        c.thinking_minimal  = RC::Rgb(137, 180, 250);
        c.thinking_medium   = RC::Rgb(148, 226, 213);
        c.thinking_high     = RC::Rgb(249, 226, 175);
        c.thinking_xhigh    = RC::Rgb(243, 139, 168);
        c.bash_mode         = RC::Rgb(249, 226, 175);
        c
    }

    /// Catppuccin Latte — warm beige light.
    /// Palette source: <https://github.com/catppuccin/catppuccin>
    pub fn catppuccin_latte() -> Self {
        let mut c = Self::light();
        // Base surfaces
        c.bg_base     = RC::Rgb(239, 241, 245);  // Base
        c.bg_surface0 = RC::Rgb(230, 233, 239);  // Mantle
        c.bg_surface1 = RC::Rgb(220, 224, 232);  // Crust
        c.bg_surface2 = RC::Rgb(204, 208, 218);  // Surface0
        c.bg_card     = RC::Rgb(230, 233, 239);
        c.bg_input    = RC::Rgb(220, 224, 232);
        // Accents
        c.primary      = RC::Rgb( 30, 102, 245);  // Blue
        c.success      = RC::Rgb( 64, 160,  43);  // Green
        c.error        = RC::Rgb(210,  15,  57);  // Red
        c.warning      = RC::Rgb(223, 142,  29);  // Yellow
        c.accent_dim   = RC::Rgb( 80, 140, 210);
        // Text
        c.text_muted   = RC::Rgb( 92, 106, 134);  // Overlay2
        c.text_dim     = RC::Rgb(156, 160, 176);  // Surface2
        // Borders
        c.border_base  = RC::Rgb(188, 192, 204);
        c.border_focus = RC::Rgb( 30, 102, 245);
        // Markdown
        c.md_heading    = RC::Rgb(223, 142,  29);
        c.md_link       = RC::Rgb( 30, 102, 245);
        c.md_code       = RC::Rgb( 23, 146, 153);  // Teal
        c.md_list_bullet = RC::Rgb( 23, 146, 153);
        // Syntax
        c.syntax_keyword    = RC::Rgb(136,  57, 239);  // Mauve
        c.syntax_function   = RC::Rgb( 30, 102, 245);
        c.syntax_string     = RC::Rgb( 64, 160,  43);
        c.syntax_number     = RC::Rgb(254, 100,  11);  // Peach
        c.syntax_type       = RC::Rgb( 23, 146, 153);
        c.syntax_variable   = RC::Rgb(223, 142,  29);
        c.syntax_operator   = RC::Rgb(136,  57, 239);
        c.bash_mode         = RC::Rgb(223, 142,  29);
        c
    }

    /// Tokyo Night — deep indigo dark with neon cyan + rose accents.
    /// Palette source: <https://github.com/enkia/tokyo-night-vscode-theme>
    pub fn tokyo_night() -> Self {
        let mut c = Self::dark();
        // Base surfaces
        c.bg_base     = RC::Rgb( 26,  27,  38);  // bg
        c.bg_surface0 = RC::Rgb( 28,  29,  44);  // bg_dark
        c.bg_surface1 = RC::Rgb( 32,  34,  51);  // bg_highlight
        c.bg_surface2 = RC::Rgb( 41,  44,  66);  // terminal_black
        c.bg_card     = RC::Rgb( 28,  29,  44);
        c.bg_input    = RC::Rgb( 32,  34,  51);
        // Accents
        c.primary      = RC::Rgb(122, 162, 247);  // blue
        c.success      = RC::Rgb(158, 206, 106);  // green
        c.error        = RC::Rgb(247,  93, 100);  // red
        c.warning      = RC::Rgb(224, 175, 104);  // yellow
        c.accent_dim   = RC::Rgb( 65, 105, 190);
        // Text
        c.text_muted   = RC::Rgb(169, 177, 214);  // fg_dark
        c.text_dim     = RC::Rgb( 86,  95, 137);  // comment
        // Borders
        c.border_base  = RC::Rgb( 41,  44,  66);
        c.border_focus = RC::Rgb(122, 162, 247);
        // Diff
        c.diff_added   = RC::Rgb(158, 206, 106);
        c.diff_removed = RC::Rgb(247,  93, 100);
        // Markdown
        c.md_heading    = RC::Rgb(224, 175, 104);
        c.md_link       = RC::Rgb(122, 162, 247);
        c.md_code       = RC::Rgb(  42, 195, 222);  // cyan
        c.md_list_bullet = RC::Rgb( 42, 195, 222);
        // Syntax
        c.syntax_keyword    = RC::Rgb(187, 154, 247);  // purple
        c.syntax_function   = RC::Rgb(122, 162, 247);  // blue
        c.syntax_string     = RC::Rgb(158, 206, 106);  // green
        c.syntax_number     = RC::Rgb(255, 158, 100);  // orange
        c.syntax_type       = RC::Rgb( 42, 195, 222);  // cyan
        c.syntax_variable   = RC::Rgb(224, 175, 104);  // yellow
        c.syntax_operator   = RC::Rgb(187, 154, 247);
        // Thinking
        c.thinking_minimal  = RC::Rgb(122, 162, 247);
        c.thinking_medium   = RC::Rgb( 42, 195, 222);
        c.thinking_high     = RC::Rgb(224, 175, 104);
        c.thinking_xhigh    = RC::Rgb(247,  93, 100);
        c.bash_mode         = RC::Rgb(224, 175, 104);
        c
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

    // -- Step 1: new token fields
    #[test]
    fn test_dark_has_border_style_rounded() {
        let c = ThemeColors::dark();
        assert_eq!(c.border_style, BorderStyle::Rounded);
    }

    #[test]
    fn test_dark_has_bg_card() {
        let c = ThemeColors::dark();
        assert_ne!(c.bg_card, RC::Reset);
    }

    #[test]
    fn test_dark_has_bg_input() {
        let c = ThemeColors::dark();
        assert_ne!(c.bg_input, RC::Reset);
    }

    #[test]
    fn test_dark_has_accent_dim() {
        let c = ThemeColors::dark();
        assert_ne!(c.accent_dim, RC::Reset);
    }

    #[test]
    fn test_border_style_to_ratatui_rounded() {
        use ratatui::widgets::BorderType;
        assert_eq!(BorderStyle::Rounded.to_ratatui(), BorderType::Rounded);
    }

    #[test]
    fn test_border_style_to_ratatui_thick() {
        use ratatui::widgets::BorderType;
        assert_eq!(BorderStyle::Thick.to_ratatui(), BorderType::Thick);
    }

    #[test]
    fn test_light_has_new_fields() {
        let c = ThemeColors::light();
        assert_eq!(c.border_style, BorderStyle::Rounded);
        assert_ne!(c.bg_card, RC::Reset);
    }

    // -- Step 4: new built-in themes
    #[test]
    fn test_catppuccin_mocha_smoke() {
        let c = ThemeColors::catppuccin_mocha();
        assert_ne!(c.primary, RC::Reset);
        assert_ne!(c.bg_base, RC::Reset);
        assert_eq!(c.border_style, BorderStyle::Rounded);
    }

    #[test]
    fn test_catppuccin_latte_smoke() {
        let c = ThemeColors::catppuccin_latte();
        assert_ne!(c.primary, RC::Reset);
        assert_ne!(c.bg_base, RC::Reset);
    }

    #[test]
    fn test_tokyo_night_smoke() {
        let c = ThemeColors::tokyo_night();
        assert_ne!(c.primary, RC::Reset);
        assert_ne!(c.bg_base, RC::Reset);
    }
}
// endregion: --- Tests
