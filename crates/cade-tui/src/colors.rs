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
    fn bg_card_style(&self) -> Style;
    fn selected_bg_style(&self) -> Style;
    fn tool_success_bg_style(&self) -> Style;
    fn tool_error_bg_style(&self) -> Style;
    fn tool_pending_bg_style(&self) -> Style;
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
    fn bg_card_style(&self) -> Style { Style::default().bg(self.bg_card.to_ratatui()).fg(self.text_primary.to_ratatui()) }
    fn selected_bg_style(&self) -> Style { Style::default().bg(self.selected_bg.to_ratatui()).fg(self.text_primary.to_ratatui()) }
    fn tool_success_bg_style(&self) -> Style { Style::default().bg(self.tool_success_bg.to_ratatui()).fg(self.text_primary.to_ratatui()) }
    fn tool_error_bg_style(&self) -> Style { Style::default().bg(self.tool_error_bg.to_ratatui()).fg(self.text_primary.to_ratatui()) }
    fn tool_pending_bg_style(&self) -> Style { Style::default().bg(self.tool_pending_bg.to_ratatui()).fg(self.text_primary.to_ratatui()) }
}




#[cfg(feature = "syntax-highlighting")]
pub fn generate_syntect_theme(colors: &ThemeColors) -> syntect::highlighting::Theme {
    use syntect::highlighting::{Color, ThemeItem, ThemeSettings, ScopeSelectors};
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

    let mut add_scope = |scope: &str, fg: &ColorDef, font_style: Option<syntect::highlighting::FontStyle>| {
        if let Ok(selectors) = ScopeSelectors::from_str(scope) {
            theme.scopes.push(ThemeItem {
                scope: selectors,
                style: syntect::highlighting::StyleModifier {
                    foreground: Some(to_color(fg)),
                    background: None,
                    font_style,
                },
            });
        }
    };

    use syntect::highlighting::FontStyle;

    // Base syntax
    add_scope("comment", &colors.syntax_comment, Some(FontStyle::ITALIC));
    add_scope("keyword", &colors.syntax_keyword, Some(FontStyle::BOLD));
    add_scope("entity.name.function", &colors.syntax_entity_name_function, None);
    add_scope("variable", &colors.syntax_variable, None);
    add_scope("string", &colors.syntax_string, None);
    add_scope("constant.numeric", &colors.syntax_number, None);
    add_scope("entity.name.type", &colors.syntax_entity_name_type, None);
    add_scope("keyword.operator", &colors.syntax_keyword_operator, None);
    add_scope("punctuation", &colors.syntax_punctuation, None);
    
    // Extended syntax
    add_scope("constant", &colors.syntax_constant, None);
    add_scope("constant.character.escape", &colors.syntax_string_escape, None);
    add_scope("support.type", &colors.syntax_type_builtin, Some(FontStyle::ITALIC));
    add_scope("keyword.control", &colors.syntax_keyword_control, Some(FontStyle::BOLD));
    add_scope("variable.parameter", &colors.syntax_variable_parameter, Some(FontStyle::ITALIC));
    add_scope("variable.other.member", &colors.syntax_variable_other_member, None);
    add_scope("support.function", &colors.syntax_support_function, None);
    add_scope("support.macro", &colors.syntax_support_macro, None);

    // Additional premium scopes to map to existing colors
    add_scope("storage.type", &colors.syntax_type, Some(FontStyle::ITALIC));
    add_scope("storage.modifier", &colors.syntax_keyword, Some(FontStyle::BOLD));
    add_scope("entity.name.tag", &colors.syntax_keyword, None);
    add_scope("entity.other.attribute-name", &colors.syntax_variable_parameter, Some(FontStyle::ITALIC));
    add_scope("string.regexp", &colors.syntax_string_escape, None);
    add_scope("markup.heading", &colors.md_heading, Some(FontStyle::BOLD));
    add_scope("markup.bold", &colors.text_primary, Some(FontStyle::BOLD));
    add_scope("markup.italic", &colors.text_primary, Some(FontStyle::ITALIC));
    add_scope("markup.inline.raw", &colors.md_code, None);
    add_scope("markup.quote", &colors.md_quote, Some(FontStyle::ITALIC));

    theme
}
