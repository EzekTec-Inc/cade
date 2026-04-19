/// Theme color palette for the CADE TUI.
use ratatui::style::{Color as RC, Style, Modifier};
pub use cade_core::resources::themes::{ThemeColors, ColorDef, BorderStyle};

pub trait ColorDefExt {
    fn to_ratatui(self) -> RC;
}

impl ColorDefExt for ColorDef {
    fn to_ratatui(self) -> RC {
        match self {
            ColorDef::Rgb(r, g, b) => RC::Rgb(r, g, b),
            ColorDef::Reset => RC::Reset,
        }
    }
}

pub trait BorderStyleExt {
    fn to_ratatui(self) -> ratatui::widgets::BorderType;
}

impl BorderStyleExt for BorderStyle {
    fn to_ratatui(self) -> ratatui::widgets::BorderType {
        use ratatui::widgets::BorderType;
        match self {
            BorderStyle::Rounded => BorderType::Rounded,
            BorderStyle::Thick   => BorderType::Thick,
            BorderStyle::Plain   => BorderType::Plain,
            BorderStyle::Double  => BorderType::Double,
        }
    }
}

pub trait ThemeColorsExt {
    fn style_base(&self) -> Style;
    fn style_surface0(&self) -> Style;
    fn style_surface1(&self) -> Style;
    fn style_surface2(&self) -> Style;

    fn text_primary(&self) -> Style;
    fn text_muted(&self) -> Style;
    fn text_dim(&self) -> Style;
    
    fn text_primary_bold(&self) -> Style;
    fn text_muted_bold(&self) -> Style;

    fn border_base(&self) -> Style;
    fn border_focus(&self) -> Style;
    
    fn primary(&self) -> Style;
    fn primary_bold(&self) -> Style;
    fn success(&self) -> Style;
    fn error(&self) -> Style;
    fn warning(&self) -> Style;

    fn badge(&self) -> Style;

    fn diff_added(&self) -> Style;
    fn diff_removed(&self) -> Style;
    fn diff_context(&self) -> Style;

    fn md_heading(&self) -> Style;
    fn md_link(&self) -> Style;
    fn md_link_url(&self) -> Style;
    fn md_code(&self) -> Style;
    fn md_code_block(&self) -> Style;
    fn md_code_block_border(&self) -> Style;
    fn md_quote(&self) -> Style;
    fn md_quote_border(&self) -> Style;
    fn md_hr(&self) -> Style;
    fn md_list_bullet(&self) -> Style;

    fn syntax_comment(&self) -> Style;
    fn syntax_keyword(&self) -> Style;
    fn syntax_function(&self) -> Style;
    fn syntax_variable(&self) -> Style;
    fn syntax_string(&self) -> Style;
    fn syntax_number(&self) -> Style;
    fn syntax_type(&self) -> Style;
    fn syntax_operator(&self) -> Style;
    fn syntax_punctuation(&self) -> Style;

    fn thinking_off(&self) -> Style;
    fn thinking_minimal(&self) -> Style;
    fn thinking_low(&self) -> Style;
    fn thinking_medium(&self) -> Style;
    fn thinking_high(&self) -> Style;
    fn thinking_xhigh(&self) -> Style;

    fn bash_mode(&self) -> Style;

    fn from_theme(theme: &cade_core::resources::themes::Theme) -> ThemeColors;
}

impl ThemeColorsExt for ThemeColors {
    fn style_base(&self) -> Style { Style::default().bg(self.bg_base.to_ratatui()).fg(self.text_primary.to_ratatui()) }
    fn style_surface0(&self) -> Style { Style::default().bg(self.bg_surface0.to_ratatui()).fg(self.text_primary.to_ratatui()) }
    fn style_surface1(&self) -> Style { Style::default().bg(self.bg_surface1.to_ratatui()).fg(self.text_primary.to_ratatui()) }
    fn style_surface2(&self) -> Style { Style::default().bg(self.bg_surface2.to_ratatui()).fg(self.text_primary.to_ratatui()) }

    fn text_primary(&self) -> Style { Style::default().fg(self.text_primary.to_ratatui()) }
    fn text_muted(&self) -> Style { Style::default().fg(self.text_muted.to_ratatui()) }
    fn text_dim(&self) -> Style { Style::default().fg(self.text_dim.to_ratatui()) }
    
    fn text_primary_bold(&self) -> Style { Style::default().fg(self.text_primary.to_ratatui()).add_modifier(Modifier::BOLD) }
    fn text_muted_bold(&self) -> Style { Style::default().fg(self.text_muted.to_ratatui()).add_modifier(Modifier::BOLD) }

    fn border_base(&self) -> Style { Style::default().fg(self.border_base.to_ratatui()) }
    fn border_focus(&self) -> Style { Style::default().fg(self.border_focus.to_ratatui()) }
    
    fn primary(&self) -> Style { Style::default().fg(self.primary.to_ratatui()) }
    fn primary_bold(&self) -> Style { Style::default().fg(self.primary.to_ratatui()).add_modifier(Modifier::BOLD) }
    fn success(&self) -> Style { Style::default().fg(self.success.to_ratatui()) }
    fn error(&self) -> Style { Style::default().fg(self.error.to_ratatui()) }
    fn warning(&self) -> Style { Style::default().fg(self.warning.to_ratatui()) }

    fn badge(&self) -> Style { Style::default().bg(self.bg_surface2.to_ratatui()).fg(self.primary.to_ratatui()) }

    fn diff_added(&self) -> Style { Style::default().fg(self.diff_added.to_ratatui()) }
    fn diff_removed(&self) -> Style { Style::default().fg(self.diff_removed.to_ratatui()) }
    fn diff_context(&self) -> Style { Style::default().fg(self.diff_context.to_ratatui()) }

    fn md_heading(&self) -> Style { Style::default().fg(self.md_heading.to_ratatui()) }
    fn md_link(&self) -> Style { Style::default().fg(self.md_link.to_ratatui()) }
    fn md_link_url(&self) -> Style { Style::default().fg(self.md_link_url.to_ratatui()) }
    fn md_code(&self) -> Style { Style::default().fg(self.md_code.to_ratatui()) }
    fn md_code_block(&self) -> Style { Style::default().fg(self.md_code_block.to_ratatui()) }
    fn md_code_block_border(&self) -> Style { Style::default().fg(self.md_code_block_border.to_ratatui()) }
    fn md_quote(&self) -> Style { Style::default().fg(self.md_quote.to_ratatui()) }
    fn md_quote_border(&self) -> Style { Style::default().fg(self.md_quote_border.to_ratatui()) }
    fn md_hr(&self) -> Style { Style::default().fg(self.md_hr.to_ratatui()) }
    fn md_list_bullet(&self) -> Style { Style::default().fg(self.md_list_bullet.to_ratatui()) }

    fn syntax_comment(&self) -> Style { Style::default().fg(self.syntax_comment.to_ratatui()) }
    fn syntax_keyword(&self) -> Style { Style::default().fg(self.syntax_keyword.to_ratatui()) }
    fn syntax_function(&self) -> Style { Style::default().fg(self.syntax_function.to_ratatui()) }
    fn syntax_variable(&self) -> Style { Style::default().fg(self.syntax_variable.to_ratatui()) }
    fn syntax_string(&self) -> Style { Style::default().fg(self.syntax_string.to_ratatui()) }
    fn syntax_number(&self) -> Style { Style::default().fg(self.syntax_number.to_ratatui()) }
    fn syntax_type(&self) -> Style { Style::default().fg(self.syntax_type.to_ratatui()) }
    fn syntax_operator(&self) -> Style { Style::default().fg(self.syntax_operator.to_ratatui()) }
    fn syntax_punctuation(&self) -> Style { Style::default().fg(self.syntax_punctuation.to_ratatui()) }

    fn thinking_off(&self) -> Style { Style::default().fg(self.thinking_off.to_ratatui()) }
    fn thinking_minimal(&self) -> Style { Style::default().fg(self.thinking_minimal.to_ratatui()) }
    fn thinking_low(&self) -> Style { Style::default().fg(self.thinking_low.to_ratatui()) }
    fn thinking_medium(&self) -> Style { Style::default().fg(self.thinking_medium.to_ratatui()) }
    fn thinking_high(&self) -> Style { Style::default().fg(self.thinking_high.to_ratatui()) }
    fn thinking_xhigh(&self) -> Style { Style::default().fg(self.thinking_xhigh.to_ratatui()) }

    fn bash_mode(&self) -> Style { Style::default().fg(self.bash_mode.to_ratatui()) }

    fn from_theme(theme: &cade_core::resources::themes::Theme) -> ThemeColors {
        let mut base = Self::dark();
        base.source_path = Some(theme.source.clone());

        let resolve =
            |c: &cade_core::resources::themes::ThemeColor| -> ColorDef { resolve_color(c, &theme.vars) };
        let t = &theme.colors;
        
        base.primary = resolve(&t.accent);
        
        // Map legacy UI backgrounds to semantic elevations
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

// region:    --- Support

fn resolve_color(
    c: &cade_core::resources::themes::ThemeColor,
    vars: &std::collections::HashMap<String, cade_core::resources::themes::ThemeColor>,
) -> ColorDef {
    use cade_core::resources::themes::ThemeColor;
    match c {
        ThemeColor::Index(_i) => ColorDef::Reset, // ColorDef doesn't support index natively
        ThemeColor::Hex(s) if s.is_empty() => ColorDef::Reset,
        ThemeColor::Hex(s) => {
            // Might be a variable reference
            if let Some(var_val) = vars.get(s) {
                return resolve_color(var_val, vars);
            }
            // Try to parse as hex
            if let Some(rgb) = parse_hex(s) {
                return ColorDef::Rgb(rgb.0, rgb.1, rgb.2);
            }
            ColorDef::Reset
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


#[cfg(feature = "syntax-highlighting")]
pub fn generate_syntect_theme(colors: &ThemeColors) -> syntect::highlighting::Theme {
    use syntect::highlighting::{Color, ThemeItem, ThemeSettings, FontStyle, ScopeSelectors};
    use std::str::FromStr;

    let to_color = |c: &ColorDef| -> Color {
        match c {
            ColorDef::Rgb(r, g, b) => Color { r: *r, g: *g, b: *b, a: 255 },
            ColorDef::Reset => Color { r: 205, g: 214, b: 244, a: 255 }, // Default fg
        }
    };

    let mut theme = syntect::highlighting::Theme {
        name: Some("CadeDynamic".to_string()),
        author: Some("CADE".to_string()),
        settings: ThemeSettings {
            foreground: Some(to_color(&colors.text_primary)),
            background: Some(to_color(&colors.bg_surface1)),
            caret: Some(to_color(&colors.primary)),
            line_highlight: Some(to_color(&colors.bg_surface2)),
            misspelling: Some(to_color(&colors.error)),
            minimap_border: None,
            accent: Some(to_color(&colors.accent_dim)),
            popup_css: None,
            phantom_css: None,
            brackets_background: None,
            brackets_options: None,
            brackets_foreground: None,
            bracket_contents_options: None,
            bracket_contents_foreground: None,
            tags_options: None,
            tags_foreground: None,
            find_highlight: None,
            find_highlight_foreground: None,
            highlight: None,
            gutter: None,
            gutter_foreground: None,
            selection: Some(to_color(&colors.bg_surface2)),
            selection_foreground: None,
            selection_border: None,
            inactive_selection: None,
            inactive_selection_foreground: None,
            guide: None,
            active_guide: None,
            stack_guide: None,
            shadow: None,
        },
        scopes: Vec::new(),
    };

    let mut add_scope = |scope: &str, fg: &ColorDef| {
        if let Ok(selectors) = ScopeSelectors::from_str(scope) {
            theme.scopes.push(ThemeItem {
                scope: selectors,
                style: syntect::highlighting::StyleModifier {
                    foreground: Some(to_color(fg)),
                    background: None,
                    font_style: None,
                },
            });
        }
    };

    add_scope("comment", &colors.syntax_comment);
    add_scope("keyword", &colors.syntax_keyword);
    add_scope("entity.name.function", &colors.syntax_entity_name_function);
    add_scope("variable", &colors.syntax_variable);
    add_scope("string", &colors.syntax_string);
    add_scope("constant.numeric", &colors.syntax_number);
    add_scope("entity.name.type", &colors.syntax_entity_name_type);
    add_scope("keyword.operator", &colors.syntax_keyword_operator);
    add_scope("punctuation", &colors.syntax_punctuation);
    
    add_scope("constant", &colors.syntax_constant);
    add_scope("constant.character.escape", &colors.syntax_string_escape);
    add_scope("support.type", &colors.syntax_type_builtin);
    add_scope("keyword.control", &colors.syntax_keyword_control);
    add_scope("variable.parameter", &colors.syntax_variable_parameter);
    add_scope("variable.other.member", &colors.syntax_variable_other_member);
    add_scope("support.function", &colors.syntax_support_function);
    add_scope("support.macro", &colors.syntax_support_macro);

    theme
}
